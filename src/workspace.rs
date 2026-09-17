//! Private Git objects and three-way file operations. The real index is never written.
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct File {
    pub bytes: Vec<u8>,
    pub executable: bool,
}
pub type Snapshot = BTreeMap<String, File>;

pub fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let out = Command::new("git")
        .arg("-c")
        .arg("core.hooksPath=/dev/null")
        .arg("-c")
        .arg("core.fsmonitor=false")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()?;
    ensure!(
        out.status.success(),
        "git {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    Ok(out.stdout)
}
fn output(mut cmd: Command, input: &[u8]) -> Result<Vec<u8>> {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child.stdin.take().unwrap().write_all(input)?;
    let out = child.wait_with_output()?;
    ensure!(
        out.status.success(),
        "git: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    Ok(out.stdout)
}
pub fn valid_path(path: &str) -> Result<()> {
    ensure!(
        !path.is_empty()
            && Path::new(path)
                .components()
                .all(|p| matches!(p, Component::Normal(s) if s != ".git")),
        "unsafe repository path: {path}"
    );
    Ok(())
}
fn safe_path(root: &Path, path: &str) -> Result<PathBuf> {
    valid_path(path)?;
    let mut target = root.to_path_buf();
    for part in Path::new(path).components() {
        target.push(part);
        if let Ok(m) = fs::symlink_metadata(&target) {
            ensure!(
                !m.file_type().is_symlink(),
                "symlink needs explicit handling: {path}"
            );
        }
    }
    Ok(target)
}
pub fn read_file(root: &Path, path: &str) -> Result<Option<File>> {
    let p = safe_path(root, path)?;
    match fs::metadata(&p) {
        Ok(m) => {
            ensure!(m.is_file(), "unsupported directory/submodule at {path}");
            Ok(Some(File {
                bytes: fs::read(p)?,
                executable: m.permissions().mode() & 0o111 != 0,
            }))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
pub fn capture(root: &Path) -> Result<Snapshot> {
    let paths = git(
        root,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ],
    )?;
    let mut result = Snapshot::new();
    for path in paths.split(|b| *b == 0).filter(|s| !s.is_empty()) {
        let path =
            std::str::from_utf8(path).context("non-UTF-8 filenames are not supported yet")?;
        if let Some(file) = read_file(root, path)? {
            result.insert(path.into(), file);
        }
    }
    Ok(result)
}

/// Paths already owned by a baseline stay in scope even after an ignore-rule change.
pub fn capture_known(root: &Path, known: &Snapshot) -> Result<Snapshot> {
    let mut files = capture(root)?;
    for path in known.keys() {
        if let Some(file) = read_file(root, path)? {
            files.insert(path.clone(), file);
        }
    }
    Ok(files)
}

#[derive(Debug)]
pub struct Workspace {
    pub real: PathBuf,
    pub shadow: PathBuf,
    pub storage: PathBuf,
}
impl Workspace {
    pub fn create(project: &Path, session: &Path) -> Result<Self> {
        let real = PathBuf::from(
            String::from_utf8(git(project, &["rev-parse", "--show-toplevel"])?)?.trim(),
        )
        .canonicalize()?;
        git(&real, &["rev-parse", "--verify", "HEAD"]).context(
            "Tandem needs an existing HEAD; create the repository's first commit yourself",
        )?;
        ensure!(
            !session.starts_with(&real),
            "session storage must be outside the project"
        );
        fs::create_dir_all(session)?;
        fs::set_permissions(session, fs::Permissions::from_mode(0o700))?;
        let storage = session.join("objects");
        let shadow = session.join("worktree");
        git(
            session,
            &[
                "clone",
                "--shared",
                "--no-checkout",
                "--",
                real.to_str().unwrap(),
                storage.to_str().unwrap(),
            ],
        )?;
        git(
            &storage,
            &[
                "worktree",
                "add",
                "--detach",
                shadow.to_str().unwrap(),
                "HEAD",
            ],
        )?;
        let ws = Self {
            real,
            shadow,
            storage,
        };
        let baseline = capture(&ws.real)?;
        ws.sync(&baseline)?;
        Ok(ws)
    }
    /// Git trees are content-addressed snapshots, not commits or refs in the user's repository.
    pub fn store(&self, files: &Snapshot) -> Result<String> {
        let index = self.storage.join("tandem-index");
        let mut cmd = Command::new("git");
        cmd.arg("-C")
            .arg(&self.storage)
            .args(["read-tree", "--empty"])
            .env("GIT_INDEX_FILE", &index);
        output(cmd, &[])?;
        let mut entries = Vec::new();
        for (path, file) in files {
            valid_path(path)?;
            let mut cmd = Command::new("git");
            cmd.arg("-C")
                .arg(&self.storage)
                .args(["hash-object", "-w", "--stdin"]);
            let hash = String::from_utf8(output(cmd, &file.bytes)?)?;
            write!(
                entries,
                "{} {}\t{}\0",
                if file.executable { "100755" } else { "100644" },
                hash.trim(),
                path
            )?;
        }
        let mut cmd = Command::new("git");
        cmd.arg("-C")
            .arg(&self.storage)
            .args(["update-index", "-z", "--index-info"])
            .env("GIT_INDEX_FILE", &index);
        output(cmd, &entries)?;
        let mut cmd = Command::new("git");
        cmd.arg("-C")
            .arg(&self.storage)
            .arg("write-tree")
            .env("GIT_INDEX_FILE", &index);
        Ok(String::from_utf8(output(cmd, &[])?)?.trim().into())
    }
    pub fn load(&self, tree: &str) -> Result<Snapshot> {
        ensure!(
            tree.len() >= 40 && tree.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid tree id"
        );
        let mut result = Snapshot::new();
        for entry in git(&self.storage, &["ls-tree", "-rz", tree])?
            .split(|b| *b == 0)
            .filter(|s| !s.is_empty())
        {
            let (meta, path) = std::str::from_utf8(entry)?
                .split_once('\t')
                .context("invalid tree")?;
            let fields: Vec<_> = meta.split(' ').collect();
            ensure!(fields[1] == "blob", "unsupported tree entry");
            valid_path(path)?;
            result.insert(
                path.into(),
                File {
                    bytes: git(&self.storage, &["cat-file", "blob", fields[2]])?,
                    executable: fields[0] == "100755",
                },
            );
        }
        Ok(result)
    }
    pub fn diff(&self, base: &str, result: &str, context: usize) -> Result<String> {
        Ok(String::from_utf8_lossy(&git(
            &self.storage,
            &[
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--no-renames",
                &format!("--unified={context}"),
                base,
                result,
                "--",
            ],
        )?)
        .into())
    }
    pub fn sync(&self, files: &Snapshot) -> Result<()> {
        replace(&self.shadow, &capture(&self.shadow)?, files)?;
        // Only the private shadow index is updated. This also keeps baseline untracked
        // files visible to capture if the proposal introduces new ignore rules.
        let tree = self.store(files)?;
        git(&self.shadow, &["read-tree", &tree])?;
        Ok(())
    }
}

pub fn merge(base: &Snapshot, current: &Snapshot, desired: &Snapshot) -> Result<Snapshot> {
    let paths: BTreeSet<_> = base.keys().chain(desired.keys()).collect();
    let mut result = current.clone();
    let mut conflicts = Vec::new();
    for path in paths {
        let b = base.get(path);
        let c = current.get(path);
        let d = desired.get(path);
        if b == d || c == d {
            continue;
        }
        if c == b {
            if let Some(d) = d {
                result.insert(path.clone(), d.clone());
            } else {
                result.remove(path);
            }
            continue;
        }
        let merged = (|| -> Result<File> {
            let (b, c, d) = match (b, c, d) {
                (Some(b), Some(c), Some(d)) => (b, c, d),
                _ => bail!("creation/deletion overlap"),
            };
            ensure!(
                ![b, c, d].iter().any(|f| f.bytes.contains(&0)),
                "binary overlap"
            );
            let executable = if c.executable == b.executable {
                d.executable
            } else if d.executable == b.executable || c.executable == d.executable {
                c.executable
            } else {
                bail!("mode conflict")
            };
            let dir = tempfile::tempdir()?;
            for (name, f) in [("base", b), ("current", c), ("desired", d)] {
                fs::write(dir.path().join(name), &f.bytes)?;
            }
            let out = Command::new("git")
                .args([
                    "merge-file",
                    "-p",
                    "--diff-algorithm=histogram",
                    "current",
                    "base",
                    "desired",
                ])
                .current_dir(dir.path())
                .output()?;
            ensure!(out.status.success(), "overlapping edits");
            Ok(File {
                bytes: out.stdout,
                executable,
            })
        })();
        match merged {
            Ok(f) => {
                result.insert(path.clone(), f);
            }
            Err(_) => conflicts.push(path.clone()),
        }
    }
    ensure!(
        conflicts.is_empty(),
        "conflict: {} (no files changed)",
        conflicts.join(", ")
    );
    Ok(result)
}

/// Preflight every path before any writes. Replace files atomically; rollback ordinary IO failures.
/// Concurrent writers must save before applying. Recheck each original immediately before writing.
pub fn replace(root: &Path, current: &Snapshot, desired: &Snapshot) -> Result<()> {
    let paths: BTreeSet<_> = current
        .keys()
        .chain(desired.keys())
        .filter(|p| current.get(*p) != desired.get(*p))
        .collect();
    for path in &paths {
        ensure!(
            read_file(root, path)?.as_ref() == current.get(*path),
            "file changed during operation: {path}"
        );
    }
    let mut written = Vec::new();
    for path in paths {
        let result = (|| -> Result<()> {
            ensure!(
                read_file(root, path)?.as_ref() == current.get(path),
                "file changed during operation: {path}"
            );
            write_file(root, path, desired.get(path))
        })();
        if let Err(e) = result {
            for p in written.into_iter().rev() {
                // Never roll back over a concurrent human edit.
                if read_file(root, p)?.as_ref() == desired.get(p) {
                    write_file(root, p, current.get(p))?;
                }
            }
            return Err(e);
        }
        written.push(path);
    }
    Ok(())
}
fn write_file(root: &Path, path: &str, file: Option<&File>) -> Result<()> {
    let target = safe_path(root, path)?;
    if let Some(file) = file {
        let parent = target.parent().unwrap();
        fs::create_dir_all(parent)?;
        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        temp.write_all(&file.bytes)?;
        // Preserve the user's read/write permissions on an existing file; Git only
        // models the executable bit. In particular, do not widen a private 0600 file.
        let existing = fs::metadata(&target)
            .map(|m| m.permissions().mode())
            .unwrap_or(0o600);
        let execute = if file.executable {
            if existing & 0o111 != 0 {
                existing & 0o111
            } else {
                0o100
            }
        } else {
            0
        };
        let mode = (existing & 0o666) | execute;
        temp.as_file()
            .set_permissions(fs::Permissions::from_mode(mode))?;
        temp.as_file().sync_all()?;
        temp.persist(&target)?;
    } else if target.exists() {
        fs::remove_file(target)?;
    }
    Ok(())
}
