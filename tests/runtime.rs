//! Explicit runtime checks, excluded from Nix builders without namespaces/local sockets.
use std::{fs, path::Path};
use tandem::{
    agent::{MockBackend, WorkspaceMode, codex::sandbox_command},
    controller::Controller,
    protocol::{Action, Stage, TourSource},
    server,
    workspace::git,
};

fn fixture() -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    let r = t.path().join("repo");
    fs::create_dir(&r).unwrap();
    git(&r, &["init", "-q"]).unwrap();
    fs::write(r.join("app.txt"), "entry\nhandler\n").unwrap();
    git(&r, &["add", "."]).unwrap();
    git(
        &r,
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
    t
}
#[tokio::test]
#[ignore = "requires Linux mount/PID namespaces and Bubblewrap; run explicitly outside restricted builders"]
async fn runtime_enforces_source_capabilities() {
    let t = fixture();
    let c = Controller::create(&t.path().join("repo"), &t.path().join("session")).unwrap();
    let h = t.path().join("home");
    fs::create_dir(&h).unwrap();
    fs::write(h.join("config.toml"), "").unwrap();
    for mode in [
        WorkspaceMode::ReadOnly,
        WorkspaceMode::Build,
        WorkspaceMode::Refine,
    ] {
        let result = sandbox_command(&h, &c.workspace.shadow, mode, Path::new("sh"))
            .args(["-c", "printf 'proposal\\n' > app.txt"])
            .output()
            .await
            .unwrap();
        if mode == WorkspaceMode::ReadOnly {
            assert!(!result.status.success());
            assert_eq!(
                fs::read_to_string(c.workspace.shadow.join("app.txt")).unwrap(),
                "entry\nhandler\n"
            );
        } else {
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            assert_eq!(
                fs::read_to_string(c.workspace.shadow.join("app.txt")).unwrap(),
                "proposal\n"
            );
        }
    }
    assert_eq!(
        fs::read_to_string(c.workspace.real.join("app.txt")).unwrap(),
        "entry\nhandler\n"
    );
    let result = sandbox_command(
        &h,
        &c.workspace.shadow,
        WorkspaceMode::Build,
        Path::new("sh"),
    )
    .args(["-c", "printf broken > .git"])
    .output()
    .await
    .unwrap();
    assert!(!result.status.success());
}
#[tokio::test]
#[ignore = "requires local Unix socket binding; run explicitly outside restricted builders"]
async fn socket_workflow_with_backend_and_repository_tour() {
    let t = fixture();
    let c = Controller::create(&t.path().join("repo"), &t.path().join("session")).unwrap();
    let socket = t.path().join("controller.sock");
    let path = socket.clone();
    let handle =
        tokio::spawn(
            async move { server::serve(c, Box::new(MockBackend::default()), &path).await },
        );
    for _ in 0..100 {
        if socket.exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    async fn send(socket: &Path, action: Action) -> tandem::protocol::View {
        let response = server::request(socket, action).await.unwrap();
        assert!(response.error.is_none(), "{:?}", response.error);
        response.view.unwrap()
    }
    async fn idle(socket: &Path) -> tandem::protocol::View {
        for _ in 0..100 {
            let v = send(socket, Action::Status).await;
            if !v.busy {
                return v;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("backend did not finish")
    }
    send(
        &socket,
        Action::Message {
            text: "Give me a tour".into(),
        },
    )
    .await;
    let view = idle(&socket).await;
    assert_eq!(view.stage, Stage::Discuss);
    assert_eq!(view.tour.unwrap().source, TourSource::Repository);
    assert!(view.proposal.is_none());
    send(&socket, Action::TourNext).await;
    send(&socket, Action::TourClose).await;
    let removed = server::request(
        &socket,
        Action::Input {
            text: "/plan example".into(),
        },
    )
    .await
    .unwrap();
    assert!(removed.error.is_some());
    assert_eq!(removed.view.unwrap().stage, Stage::Discuss);
    send(
        &socket,
        Action::Message {
            text: "sounds good".into(),
        },
    )
    .await;
    assert_eq!(idle(&socket).await.stage, Stage::Discuss);
    send(
        &socket,
        Action::Message {
            text: "add demo file".into(),
        },
    )
    .await;
    idle(&socket).await;
    send(
        &socket,
        Action::Input {
            text: "/begin".into(),
        },
    )
    .await;
    assert_eq!(idle(&socket).await.stage, Stage::Tour);
    let removed = server::request(
        &socket,
        Action::Input {
            text: "/review".into(),
        },
    )
    .await
    .unwrap();
    assert!(removed.error.is_some());
    assert_eq!(removed.view.unwrap().stage, Stage::Tour);
    assert!(!t.path().join("repo/tandem-example.txt").exists());
    send(
        &socket,
        Action::Input {
            text: "change before application".into(),
        },
    )
    .await;
    let revised = idle(&socket).await;
    assert_eq!(revised.proposal, Some(2));
    assert!(
        fs::read_to_string(t.path().join("session/worktree/tandem-example.txt"))
            .unwrap()
            .contains("Offline revision 2")
    );
    assert!(!t.path().join("repo/tandem-example.txt").exists());
    send(&socket, Action::Switch { proposal: 1 }).await;
    send(
        &socket,
        Action::Input {
            text: "/apply".into(),
        },
    )
    .await;
    assert!(t.path().join("repo/tandem-example.txt").exists());
    send(&socket, Action::Revert { change: 0 }).await;
    assert!(!t.path().join("repo/tandem-example.txt").exists());
    send(&socket, Action::Shutdown).await;
    handle.await.unwrap().unwrap();
}

#[tokio::test]
#[ignore = "requires Bubblewrap; cancels a local fixture process, no model calls"]
async fn cancellation_reaps_runtime_before_returning() {
    use std::os::unix::fs::PermissionsExt;
    use tandem::agent::{AgentBackend, Turn, codex::CodexBackend};
    let t = fixture();
    let c = Controller::create(&t.path().join("repo"), &t.path().join("session")).unwrap();
    let program = c.workspace.shadow.join("fake-codex");
    fs::write(
        &program,
        "#!/bin/sh\nsleep 2\nprintf late > late.txt\nsleep 60\n",
    )
    .unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
    let mut backend = CodexBackend::new(&t.path().join("session"), program).unwrap();
    let (events, _rx) = tokio::sync::mpsc::unbounded_channel();
    backend
        .start(
            Turn {
                prompt: "fixture".into(),
                context: "{}".into(),
                mode: WorkspaceMode::Build,
                workspace: c.workspace.shadow.clone(),
            },
            events,
        )
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    tokio::time::timeout(std::time::Duration::from_secs(3), backend.cancel())
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(2200)).await;
    assert!(!c.workspace.shadow.join("late.txt").exists());
}
