//! Discover one controller and one editor per checkout for the native Helix UI.
//! Nothing here injects input into the editor or mutates the real repository.
use crate::{protocol::*, server, workspace::git};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};
use tokio::io::{AsyncWriteExt, BufReader};

pub struct Registry {
    pub project: PathBuf,
    directory: PathBuf,
}
#[derive(Serialize, Deserialize)]
struct Address {
    project: PathBuf,
    socket: PathBuf,
}
fn private_directory(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_dir() && metadata.uid() == fs::metadata("/proc/self")?.uid(),
        "session directory must belong to the current user"
    );
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}
impl Registry {
    pub fn discover(project: &Path) -> Result<Self> {
        let mut directory = project.to_path_buf();
        while !directory.is_dir() {
            directory = directory
                .parent()
                .context("no project directory")?
                .to_path_buf();
        }
        let project = PathBuf::from(
            String::from_utf8(git(&directory, &["rev-parse", "--show-toplevel"])?)?.trim(),
        )
        .canonicalize()?;
        let root = if let Some(path) = std::env::var_os("TANDEM_RUNTIME_DIR") {
            PathBuf::from(path)
        } else if let Some(path) = std::env::var_os("XDG_RUNTIME_DIR") {
            PathBuf::from(path).join("tandem")
        } else {
            std::env::temp_dir().join(format!("tandem-{}", fs::metadata("/proc/self")?.uid()))
        };
        private_directory(&root)?;
        // Stable FNV-1a key, checked against the full canonical path below. This is an
        // address, not an authentication token. Separate Git worktrees get separate keys.
        let key = project
            .as_os_str()
            .as_encoded_bytes()
            .iter()
            .fold(0xcbf29ce484222325u64, |hash, byte| {
                (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
            });
        let directory = root.join(format!("checkout-{key:016x}"));
        private_directory(&directory)?;
        let registry = Self { project, directory };
        let identity = registry.directory.join("project");
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&identity)
        {
            Ok(mut file) => {
                use std::io::Write;
                file.write_all(registry.project.as_os_str().as_encoded_bytes())?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                ensure!(
                    fs::read(&identity)? == registry.project.as_os_str().as_encoded_bytes(),
                    "checkout registry collision; use a different TANDEM_RUNTIME_DIR"
                );
            }
            Err(e) => return Err(e.into()),
        }
        Ok(registry)
    }
    fn lock(&self, name: &str) -> Result<File> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(self.directory.join(name))?;
        file.try_lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(file)
    }
    /// Hold this guard for the controller's entire lifetime, including initial capture.
    pub fn controller_lock(&self) -> Result<File> {
        self.lock("controller.lock").context(
            "Tandem is already active for this working tree; use :tandem to reconnect, or a separate Git worktree")
    }
    pub fn new_session(&self) -> Result<PathBuf> {
        Ok(tempfile::Builder::new()
            .prefix("session-")
            .tempdir_in(&self.directory)?
            .keep())
    }
    pub fn publish(&self, socket: &Path) -> Result<()> {
        self.write(
            "active.json",
            &Address {
                project: self.project.clone(),
                socket: socket.into(),
            },
        )
    }
    fn write(&self, name: &str, value: &impl Serialize) -> Result<()> {
        let path = self.directory.join(name);
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, serde_json::to_vec(value)?)?;
        fs::rename(tmp, path)?;
        Ok(())
    }
    async fn running(&self) -> Result<Option<PathBuf>> {
        let bytes = match fs::read(self.directory.join("active.json")) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("read Tandem controller address"),
        };
        let address: Address =
            serde_json::from_slice(&bytes).context("invalid Tandem controller address")?;
        ensure!(
            address.project == self.project,
            "Tandem controller address belongs to a different project"
        );
        let reply = tokio::time::timeout(
            Duration::from_secs(2),
            server::request(&address.socket, Action::Status),
        )
        .await
        .context(
            "existing Tandem controller did not respond; stop that session before reconnecting",
        )?;
        let reply = match reply {
            Ok(reply) => reply,
            Err(error) if error.downcast_ref::<std::io::Error>().is_some_and(|error| {
                matches!(error.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused)
            }) => return Ok(None),
            Err(error) => return Err(error).with_context(|| format!(
                "could not read the existing Tandem controller at {}; stop that session before reconnecting with this build",
                address.socket.display()
            )),
        };
        ensure!(
            reply.error.is_none(),
            "existing Tandem controller rejected status: {:?}",
            reply.error
        );
        let view = reply
            .view
            .context("existing Tandem controller returned no state")?;
        ensure!(
            Some(view.real.as_str()) == self.project.to_str(),
            "existing Tandem controller belongs to a different project"
        );
        Ok(Some(address.socket))
    }
    async fn connect(&self, mock: bool) -> Result<PathBuf> {
        if let Some(socket) = self.running().await? {
            return Ok(socket);
        }
        let session = self.new_session()?;
        let socket = session.join("controller.sock");
        let log_path = session.with_extension("log");
        let log = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&log_path)?;
        let mut command = Command::new(std::env::current_exe()?);
        command
            .arg("start")
            .arg(&self.project)
            .arg("--session")
            .arg(&session)
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log);
        if mock {
            command.arg("--mock");
        }
        let mut child = command.spawn().context("start Tandem controller")?;
        for _ in 0..200 {
            if socket.exists() && self.running().await?.as_ref() == Some(&socket) {
                // Reap the child if it exits while the editor remains open. Detaching the
                // editor deliberately leaves this controller and its proposals alive.
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                return Ok(socket);
            }
            if child.try_wait()?.is_some() {
                bail!(
                    "Tandem could not start: {}",
                    fs::read_to_string(&log_path).unwrap_or_default()
                );
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let _ = child.kill();
        let _ = child.wait();
        bail!("Tandem startup timed out; see {}", log_path.display())
    }
}

async fn emit(value: &Value) -> Result<()> {
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    let mut stdout = tokio::io::stdout();
    stdout.write_all(&bytes).await?;
    stdout.flush().await?;
    Ok(())
}
pub async fn editor(project: &Path, mock: bool) -> Result<()> {
    let registry = Registry::discover(project)?;
    let _editor = registry.lock("editor.lock").context(
        "Tandem already has an editor for this working tree. Close its Tandem session or editor before reconnecting here; use another Git worktree for an independent session.")?;
    let socket = registry.connect(mock).await?;
    let attached = server::request(&socket, Action::EditorConnection { connected: true }).await?;
    ensure!(
        attached.error.is_none(),
        "could not attach editor: {:?}",
        attached.error
    );
    let result = async {
        emit(&json!({"event":"connected", "project":registry.project})).await?;
        let stream = tokio::net::UnixStream::connect(&socket).await?;
        let (read, mut write) = stream.into_split();
        let mut reader = BufReader::new(read);
        let mut stdin = BufReader::new(tokio::io::stdin());
        loop {
            tokio::select! {
                frame = server::read_frame(&mut reader) => {
                    let Some(frame) = frame? else { break; };
                    emit(&frame).await?;
                }
                frame = server::read_frame(&mut stdin) => {
                    let Some(frame) = frame? else { break; };
                    match frame["action"].as_str() {
                        Some("stop") => {
                            let reply = server::request(&socket, Action::Cancel).await?;
                            ensure!(reply.error.is_none(), "could not cancel: {:?}", reply.error);
                            let reply = server::request(&socket, Action::Shutdown).await?;
                            ensure!(reply.error.is_none(), "could not stop: {:?}", reply.error);
                            break;
                        }
                        _ => {
                            let mut bytes = serde_json::to_vec(&frame)?;
                            bytes.push(b'\n');
                            write.write_all(&bytes).await?;
                        }
                    }
                }
            }
        }
        Ok(())
    }
    .await;
    let _ = tokio::time::timeout(
        Duration::from_secs(2),
        server::request(&socket, Action::EditorConnection { connected: false }),
    )
    .await;
    result
}
