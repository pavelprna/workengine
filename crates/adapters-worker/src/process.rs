//! Generic process Worker: argv + env from a profile. Not a vendor adapter.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use workengine_application::{AppError, ChannelReaction, RunRequest, WorkerRunner};
use workengine_domain::{OUTCOME_SCHEMA_VERSION, Outcome, OutcomeKind};

use crate::decode_outcome;
use crate::encode_outcome;
use crate::supervise::{ChildWait, spawn_supervised};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Sandbox {
    Bubblewrap { rootfs: PathBuf },
    Oci { engine: String, image: String },
}

/// A credential delivered to a Worker as a read-only file, never as an
/// environment value or a command-line argument.
#[derive(Clone, Eq, PartialEq)]
pub struct SecretFile {
    name: String,
    value: String,
}

impl std::fmt::Debug for SecretFile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SecretFile")
            .field("name", &self.name)
            .field("value", &"[redacted]")
            .finish()
    }
}

impl SecretFile {
    pub fn new(name: String, value: String) -> Result<Self, AppError> {
        if !valid_secret_name(&name) {
            return Err(AppError::worker(
                "secret-file name must be an ASCII environment-style identifier",
            ));
        }
        Ok(Self { name, value })
    }
}

pub struct ProcessWorkerRunner {
    argv: Vec<String>,
    secret_files: Vec<SecretFile>,
    sandbox: Sandbox,
}

impl ProcessWorkerRunner {
    pub fn new(
        argv: Vec<String>,
        secret_files: Vec<SecretFile>,
        sandbox: Sandbox,
    ) -> Result<Self, AppError> {
        if argv.is_empty() || argv[0].is_empty() {
            return Err(AppError::worker("profile argv is empty"));
        }
        match &sandbox {
            Sandbox::Bubblewrap { rootfs } if !rootfs.is_dir() || rootfs == Path::new("/") => {
                return Err(AppError::worker(
                    "bubblewrap rootfs must be a dedicated directory",
                ));
            }
            Sandbox::Bubblewrap { rootfs }
                if !secret_files.is_empty() && !rootfs.join("run/secrets").is_dir() =>
            {
                return Err(AppError::worker(
                    "bubblewrap rootfs must provide /run/secrets for declared secret files",
                ));
            }
            Sandbox::Oci { image, .. } if !image.contains("@sha256:") => {
                return Err(AppError::worker("OCI image must be digest pinned"));
            }
            _ => {}
        }
        Ok(Self {
            argv,
            secret_files,
            sandbox,
        })
    }
}

impl WorkerRunner for ProcessWorkerRunner {
    fn run(&self, request: &RunRequest<'_>) -> Result<Outcome, AppError> {
        let profile = request.work.attributes().worker_profile();
        fs::create_dir_all(request.workspace_root).map_err(AppError::worker)?;
        let secrets = SecretDirectory::create(&self.secret_files)?;
        let cmd = self.command(
            request.workspace_root,
            secrets.as_ref().map(SecretDirectory::path),
        )?;
        match spawn_supervised(cmd, request.work.id().as_str(), request.budget) {
            Ok(ChildWait::Exited { .. }) => read_or_fail(request.workspace_root, profile),
            Ok(ChildWait::BudgetExceeded) => persist_timed_out(request.workspace_root, profile),
            Err(_) => Err(AppError::Channel(ChannelReaction::Fail)),
        }
    }

    fn decode(&self, bytes: &[u8]) -> Result<Outcome, AppError> {
        decode_outcome(bytes)
    }
}

impl ProcessWorkerRunner {
    fn command(&self, workspace: &Path, secrets: Option<&Path>) -> Result<Command, AppError> {
        match &self.sandbox {
            Sandbox::Bubblewrap { rootfs } => {
                let mut cmd = Command::new("bwrap");
                cmd.args([
                    "--unshare-all",
                    "--new-session",
                    "--die-with-parent",
                    "--clearenv",
                    "--ro-bind",
                ]);
                cmd.arg(rootfs).arg("/");
                cmd.args(["--bind"]).arg(workspace).arg("/workspace");
                cmd.args([
                    "--chdir",
                    "/workspace",
                    "--tmpfs",
                    "/tmp",
                    "--proc",
                    "/proc",
                    "--dev",
                    "/dev",
                    "--dir",
                    "/run",
                ]);
                if let Some(secrets) = secrets {
                    cmd.args(["--ro-bind"]).arg(secrets).arg("/run/secrets");
                    for secret in &self.secret_files {
                        cmd.args([
                            "--setenv",
                            &format!("{}_FILE", secret.name),
                            &format!("/run/secrets/{}", secret.name),
                        ]);
                    }
                }
                cmd.arg("--").arg(&self.argv[0]).args(&self.argv[1..]);
                Ok(cmd)
            }
            Sandbox::Oci { engine, image } => {
                let mut cmd = Command::new(engine);
                let uid = nix::unistd::Uid::current().as_raw().to_string();
                let gid = nix::unistd::Gid::current().as_raw().to_string();
                cmd.args([
                    "run",
                    "--rm",
                    "--read-only",
                    "--network",
                    "none",
                    "--cap-drop",
                    "ALL",
                    "--security-opt",
                    "no-new-privileges",
                    "--pids-limit",
                    "64",
                    "--user",
                ]);
                cmd.arg(format!("{uid}:{gid}"));
                cmd.arg("--mount").arg(format!(
                    "type=bind,src={},dst=/workspace,rw",
                    workspace.display()
                ));
                cmd.args(["--workdir", "/workspace"]);
                if let Some(secrets) = secrets {
                    cmd.arg("--mount").arg(format!(
                        "type=bind,src={},dst=/run/secrets,readonly",
                        secrets.display()
                    ));
                    for secret in &self.secret_files {
                        cmd.args([
                            "--env",
                            &format!("{}_FILE=/run/secrets/{}", secret.name, secret.name),
                        ]);
                    }
                }
                cmd.arg(image).arg(&self.argv[0]).args(&self.argv[1..]);
                Ok(cmd)
            }
        }
    }
}

struct SecretDirectory(tempfile::TempDir);

impl SecretDirectory {
    fn create(secrets: &[SecretFile]) -> Result<Option<Self>, AppError> {
        if secrets.is_empty() {
            return Ok(None);
        }
        let dir = tempfile::Builder::new()
            .prefix("workengine-secrets-")
            .tempdir()
            .map_err(AppError::worker)?;
        for secret in secrets {
            let path = dir.path().join(&secret.name);
            fs::write(&path, &secret.value).map_err(AppError::worker)?;
            restrict_file(&path)?;
        }
        Ok(Some(Self(dir)))
    }

    fn path(&self) -> &Path {
        self.0.path()
    }
}

fn valid_secret_name(name: &str) -> bool {
    let mut chars = name.bytes();
    matches!(chars.next(), Some(b'A'..=b'Z' | b'a'..=b'z' | b'_'))
        && chars.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn restrict_file(path: &Path) -> Result<(), AppError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(AppError::worker)?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

fn read_or_fail(root: &Path, profile: &str) -> Result<Outcome, AppError> {
    let dest = root.join("outcome.json");
    match fs::read(&dest) {
        Ok(bytes) => decode_outcome(&bytes),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            Outcome::new(OUTCOME_SCHEMA_VERSION, OutcomeKind::Failed, profile)
                .map_err(AppError::from)
        }
        Err(err) => Err(AppError::worker(err)),
    }
}

fn persist_timed_out(root: &Path, profile: &str) -> Result<Outcome, AppError> {
    let outcome = Outcome::new(OUTCOME_SCHEMA_VERSION, OutcomeKind::TimedOut, profile)
        .map_err(AppError::from)?;
    let json = encode_outcome(&outcome)?;
    fs::write(root.join("outcome.json"), json).map_err(AppError::worker)?;
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use workengine_domain::{Work, WorkAttributes, WorkId};

    fn work() -> Work {
        Work::new(
            WorkId::parse("work-1").unwrap(),
            WorkAttributes::new("goal", "echo").unwrap(),
            1,
        )
        .unwrap()
    }

    fn sandbox(root: &tempfile::TempDir) -> Sandbox {
        let rootfs = root.path().join("rootfs");
        for directory in ["dev", "proc", "tmp", "run", "run/secrets", "workspace"] {
            fs::create_dir_all(rootfs.join(directory)).unwrap();
        }
        fs::copy("/usr/bin/busybox", rootfs.join("busybox")).unwrap();
        Sandbox::Bubblewrap { rootfs }
    }

    fn unavailable_sandbox() -> bool {
        std::process::Command::new("bwrap")
            .args([
                "--unshare-user",
                "--ro-bind",
                "/",
                "/",
                "--",
                "/usr/bin/true",
            ])
            .status()
            .map(|status| !status.success())
            .unwrap_or(true)
    }

    #[cfg(unix)]
    #[test]
    fn missing_outcome_is_failed_not_succeeded() {
        if unavailable_sandbox() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let runner = ProcessWorkerRunner::new(
            vec!["/busybox".to_owned(), "true".to_owned()],
            vec![],
            sandbox(&root),
        )
        .unwrap();
        let work = work();
        let outcome = runner
            .run(&RunRequest {
                work: &work,
                workspace_root: dir.path(),
                budget: Duration::from_secs(2),
            })
            .unwrap();
        assert_eq!(outcome.kind(), OutcomeKind::Failed);
        assert!(!dir.path().join("outcome.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn schema_outcome_file_is_decoded() {
        if unavailable_sandbox() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let runner = ProcessWorkerRunner::new(
            vec![
                "/busybox".to_owned(),
                "sh".to_owned(),
                "-c".to_owned(),
                "printf '%s\\n' '{\"schemaVersion\":1,\"kind\":\"succeeded\",\"workerProfile\":\"echo\"}' > outcome.json"
                    .to_owned(),
            ],
            vec![],
            sandbox(&root),
        )
        .unwrap();
        let work = work();
        let outcome = runner
            .run(&RunRequest {
                work: &work,
                workspace_root: dir.path(),
                budget: Duration::from_secs(2),
            })
            .unwrap();
        assert_eq!(outcome.kind(), OutcomeKind::Succeeded);
    }

    #[cfg(unix)]
    #[test]
    fn undeclared_env_is_not_inherited() {
        if unavailable_sandbox() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        assert!(
            std::env::var("HOME").is_ok(),
            "parent HOME must exist so the test can prove it is not inherited"
        );
        let runner = ProcessWorkerRunner::new(
            vec![
                "/busybox".to_owned(),
                "sh".to_owned(),
                "-c".to_owned(),
                "printenv HOME > leaked.txt || true".to_owned(),
            ],
            vec![],
            sandbox(&root),
        )
        .unwrap();
        let work = work();
        let _ = runner
            .run(&RunRequest {
                work: &work,
                workspace_root: dir.path(),
                budget: Duration::from_secs(2),
            })
            .unwrap();
        let leaked = fs::read_to_string(dir.path().join("leaked.txt")).unwrap();
        assert!(
            leaked.trim().is_empty(),
            "child inherited undeclared HOME: {leaked}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn declared_secret_is_exposed_only_as_a_file_path() {
        if unavailable_sandbox() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let runner = ProcessWorkerRunner::new(
            vec![
                "/busybox".to_owned(),
                "sh".to_owned(),
                "-c".to_owned(),
                "test -z \"${WORKENGINE_TEST_OK-}\" && /busybox cat \"$WORKENGINE_TEST_OK_FILE\" > ok.txt"
                    .to_owned(),
            ],
            vec![SecretFile::new("WORKENGINE_TEST_OK".to_owned(), "visible".to_owned())
                .unwrap()],
            sandbox(&root),
        )
        .unwrap();
        let work = work();
        let _ = runner
            .run(&RunRequest {
                work: &work,
                workspace_root: dir.path(),
                budget: Duration::from_secs(2),
            })
            .unwrap();
        let got = fs::read_to_string(dir.path().join("ok.txt")).unwrap();
        assert_eq!(got.trim(), "visible");
    }

    #[test]
    fn secret_file_rejects_path_escape() {
        assert!(SecretFile::new("../TOKEN".to_owned(), "value".to_owned()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn secret_files_are_private_on_the_host() {
        use std::os::unix::fs::PermissionsExt;

        let secret = SecretFile::new("TOKEN".to_owned(), "value".to_owned()).unwrap();
        let dir = SecretDirectory::create(&[secret]).unwrap().unwrap();
        let mode = fs::metadata(dir.path().join("TOKEN"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}
