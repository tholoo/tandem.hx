//! Exercise the same process/IPC interface used by the Steel plugin.
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tandem::{
    protocol::{Action, EditorContext},
    server,
    workspace::git,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStdout, Command},
};

const BINARY: &str = env!("CARGO_BIN_EXE_tandem");

#[tokio::test]
#[ignore = "requires local Unix sockets and child processes; no model calls"]
async fn unreadable_live_controller_does_not_start_a_second_session() {
    let f = Fixture::new();
    let mut initializer = f.editor(&f.repo);
    initializer.ready().await;
    let socket = f.socket(&f.repo);
    initializer.stop().await;

    // Keep the address and lock of a live controller, but send an unreadable reply.
    let directory = socket.parent().unwrap().parent().unwrap();
    let lock = fs::OpenOptions::new()
        .write(true)
        .open(directory.join("controller.lock"))
        .unwrap();
    lock.try_lock().unwrap();
    let before = fs::read_dir(directory).unwrap().count();
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let responder = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let (read, mut write) = stream.split();
        let mut reader = BufReader::new(read);
        let request = server::read_frame(&mut reader).await.unwrap().unwrap();
        let mut reply = serde_json::to_vec(&json!({
            "version": 1, "id": request["id"], "view": false,
            "text": null, "error": null
        }))
        .unwrap();
        reply.push(b'\n');
        write.write_all(&reply).await.unwrap();
    });
    let mut editor = f.editor(&f.repo);
    let error = editor.next().await;
    editor.child.wait().await.unwrap();
    responder.await.unwrap();
    fs::remove_file(&socket).unwrap();
    assert_eq!(error["event"], "error");
    let text = error["text"].as_str().unwrap();
    assert!(
        text.contains("could not read the existing Tandem controller"),
        "{text}"
    );
    assert!(text.contains("invalid type"), "{text}");
    assert!(!text.contains("lock acquisition"), "{text}");
    assert_eq!(fs::read_dir(directory).unwrap().count(), before);
}

struct Fixture {
    root: tempfile::TempDir,
    repo: PathBuf,
    runtime: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("repo");
        let runtime = root.path().join("run");
        fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q"]).unwrap();
        fs::write(repo.join("app.txt"), "entry\nhandler\n").unwrap();
        git(&repo, &["add", "."]).unwrap();
        git(
            &repo,
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-qm",
                "fixture",
            ],
        )
        .unwrap();
        Self {
            root,
            repo,
            runtime,
        }
    }
    fn command(&self) -> Command {
        let mut command = Command::new(BINARY);
        command
            .env("TANDEM_RUNTIME_DIR", &self.runtime)
            .env_remove("ZELLIJ_SESSION_NAME")
            .env_remove("ZELLIJ_PANE_ID");
        command
    }
    fn editor(&self, project: &Path) -> Editor {
        let mut child = self
            .command()
            .arg("editor")
            .arg(project)
            .arg("--mock")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let lines = BufReader::new(child.stdout.take().unwrap()).lines();
        Editor { child, lines }
    }
    fn socket(&self, project: &Path) -> PathBuf {
        for entry in fs::read_dir(&self.runtime).unwrap() {
            let path = entry.unwrap().path().join("active.json");
            if let Ok(bytes) = fs::read(path) {
                let address: Value = serde_json::from_slice(&bytes).unwrap();
                if address["project"] == project.to_str().unwrap() {
                    return PathBuf::from(address["socket"].as_str().unwrap());
                }
            }
        }
        panic!("missing session address")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // Keep failures from leaking our own fixture controllers.
        if let Ok(entries) = fs::read_dir(&self.runtime) {
            for entry in entries.flatten() {
                if let Ok(bytes) = fs::read(entry.path().join("active.json"))
                    && let Ok(address) = serde_json::from_slice::<Value>(&bytes)
                    && let Some(socket) =
                        address["socket"].as_str().filter(|s| Path::new(s).exists())
                {
                    for action in ["cancel", "shutdown"] {
                        let _ = std::process::Command::new(BINARY)
                            .args(["send", socket, &json!({"action":action}).to_string()])
                            .output();
                    }
                }
            }
        }
    }
}
struct Editor {
    child: Child,
    lines: Lines<BufReader<ChildStdout>>,
}
impl Editor {
    async fn next(&mut self) -> Value {
        let line = tokio::time::timeout(Duration::from_secs(15), self.lines.next_line())
            .await
            .expect("editor timed out")
            .unwrap()
            .expect("editor unexpectedly exited");
        serde_json::from_str(&line).unwrap()
    }
    async fn ready(&mut self) {
        let event = self.next().await;
        assert_eq!(event["event"], "connected", "{event}");
    }
    async fn send(&mut self, action: Value) {
        let mut request = action;
        request["version"] = json!(1);
        request["id"] = json!(90);
        let mut bytes = serde_json::to_vec(&request).unwrap();
        bytes.push(b'\n');
        self.child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(&bytes)
            .await
            .unwrap();
    }
    async fn context(&mut self, project: &Path) {
        self.send(json!({"action":"context", "context": EditorContext {
            file: Some(project.join("app.txt").display().to_string()), line: 1, column: 1,
            ..Default::default()
        }}))
        .await;
        loop {
            if self.next().await["id"] == 90 {
                break;
            }
        }
    }
    async fn stop(&mut self) {
        self.send(json!({"action":"stop"})).await;
        assert!(
            tokio::time::timeout(Duration::from_secs(5), self.child.wait())
                .await
                .unwrap()
                .unwrap()
                .success()
        );
    }
}

#[tokio::test]
#[ignore = "requires local Unix sockets and child processes; no model calls"]
async fn native_sessions_are_exclusive_reconnectable_and_separate_per_worktree() {
    let f = Fixture::new();
    let mut editor = f.editor(&f.repo.join("app.txt"));
    editor.ready().await;
    editor.context(&f.repo).await;
    let socket = f.socket(&f.repo);
    let original = server::request(&socket, Action::Status)
        .await
        .unwrap()
        .view
        .unwrap();
    let reply = server::request(
        &socket,
        Action::Input {
            text: "sounds good".into(),
        },
    )
    .await
    .unwrap();
    assert!(reply.error.is_none());
    assert_eq!(reply.view.unwrap().stage, tandem::protocol::Stage::Discuss);

    let alias = f.root.path().join("alias");
    std::os::unix::fs::symlink(&f.repo, &alias).unwrap();
    let mut duplicate = f.editor(&alias);
    let error = duplicate.next().await;
    assert_eq!(error["event"], "error");
    assert!(
        error["text"]
            .as_str()
            .unwrap()
            .contains("already has an editor")
    );
    duplicate.child.wait().await.unwrap();
    let duplicate_session = f.root.path().join("duplicate-session");
    let result = f
        .command()
        .arg("start")
        .arg(&f.repo)
        .arg("--session")
        .arg(&duplicate_session)
        .arg("--mock")
        .output()
        .await
        .unwrap();
    assert!(!result.status.success());
    assert!(!duplicate_session.exists());

    editor.child.stdin.take();
    editor.child.wait().await.unwrap();
    server::request(
        &socket,
        Action::Input {
            text: "Discuss an example change".into(),
        },
    )
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let rejected = server::request(
        &socket,
        Action::Begin {
            text: String::new(),
        },
    )
    .await
    .unwrap();
    assert!(rejected.error.unwrap().contains("editor disconnected"));

    let mut reconnected = f.editor(&f.repo);
    reconnected.ready().await;
    let history = reconnected.next().await;
    assert_eq!(history["event"], "history");
    assert!(
        history["lines"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s.as_str().unwrap().contains("sounds good"))
    );
    reconnected.context(&f.repo).await;
    assert_eq!(f.socket(&f.repo), socket);
    assert_eq!(
        server::request(&socket, Action::Status)
            .await
            .unwrap()
            .view
            .unwrap()
            .shadow,
        original.shadow
    );

    let other = f.root.path().join("other");
    git(
        &f.repo,
        &[
            "worktree",
            "add",
            "--detach",
            other.to_str().unwrap(),
            "HEAD",
        ],
    )
    .unwrap();
    let mut independent = f.editor(&other);
    independent.ready().await;
    let other_socket = f.socket(&other);
    assert_ne!(other_socket, socket);
    independent.stop().await;
    assert!(!other_socket.exists());
    assert!(socket.exists());
    reconnected.stop().await;
    assert!(!socket.exists());

    // A dead socket address may be replaced; an unreadable live controller may not.
    let stale = tokio::net::UnixListener::bind(&socket).unwrap();
    drop(stale);
    let mut restarted = f.editor(&f.repo);
    restarted.ready().await;
    assert_ne!(f.socket(&f.repo), socket);
    restarted.stop().await;
}
