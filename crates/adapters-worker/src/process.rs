//! Generic process Worker: argv + env from a profile. Not a vendor adapter.

use std::fs::{self, File};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use workengine_application::{AppError, ChannelReaction, RunRequest, WorkerExit, WorkerRunner};
use workengine_domain::{OUTCOME_SCHEMA_VERSION, Outcome, OutcomeKind};

use crate::decode_outcome;
use crate::encode_outcome;
use crate::supervise::{ChildWait, spawn_supervised};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Sandbox {
    Bubblewrap {
        rootfs: PathBuf,
        rootfs_digest: String,
        seccomp_profile: PathBuf,
        seccomp_digest: String,
    },
    Oci {
        engine: String,
        image: String,
        seccomp_profile: PathBuf,
        seccomp_digest: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceLimits {
    pub memory_bytes: u64,
    pub max_processes: u32,
    pub max_open_files: u32,
    pub max_file_bytes: u64,
    pub cpu_seconds: u64,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            memory_bytes: 1_073_741_824,
            max_processes: 64,
            max_open_files: 256,
            max_file_bytes: 1_073_741_824,
            cpu_seconds: 900,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EgressPolicy {
    Deny,
    BrokerSocket(PathBuf),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkerPolicy {
    pub resources: ResourceLimits,
    pub egress: EgressPolicy,
}

impl Default for WorkerPolicy {
    fn default() -> Self {
        Self {
            resources: ResourceLimits::default(),
            egress: EgressPolicy::Deny,
        }
    }
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
    policy: WorkerPolicy,
}

impl ProcessWorkerRunner {
    pub fn new(
        argv: Vec<String>,
        secret_files: Vec<SecretFile>,
        sandbox: Sandbox,
    ) -> Result<Self, AppError> {
        Self::new_with_policy(argv, secret_files, sandbox, WorkerPolicy::default())
    }

    pub fn new_with_policy(
        argv: Vec<String>,
        secret_files: Vec<SecretFile>,
        sandbox: Sandbox,
        policy: WorkerPolicy,
    ) -> Result<Self, AppError> {
        if argv.is_empty() || argv[0].is_empty() {
            return Err(AppError::worker("profile argv is empty"));
        }
        match &sandbox {
            Sandbox::Bubblewrap { rootfs, .. } if !rootfs.is_dir() || rootfs == Path::new("/") => {
                return Err(AppError::worker(
                    "bubblewrap rootfs must be a dedicated directory",
                ));
            }
            Sandbox::Bubblewrap { rootfs, .. }
                if !secret_files.is_empty() && !rootfs.join("run/secrets").is_dir() =>
            {
                return Err(AppError::worker(
                    "bubblewrap rootfs must provide /run/secrets for declared secret files",
                ));
            }
            Sandbox::Bubblewrap {
                rootfs,
                rootfs_digest: expected,
                seccomp_profile,
                seccomp_digest,
            } => {
                validate_digest(expected)?;
                let actual = digest_rootfs(rootfs)?;
                if actual != *expected {
                    return Err(AppError::worker(format!(
                        "bubblewrap rootfs digest mismatch: expected {expected}, got {actual}"
                    )));
                }
                validate_policy_file(seccomp_profile, seccomp_digest)?;
            }
            Sandbox::Oci { engine, .. } if engine != "docker" && engine != "podman" => {
                return Err(AppError::worker("OCI engine must be docker or podman"));
            }
            Sandbox::Oci { image, .. } if image_digest(image).is_none() => {
                return Err(AppError::worker("OCI image must be digest pinned"));
            }
            Sandbox::Oci {
                engine,
                image,
                seccomp_profile,
                seccomp_digest,
            } => {
                validate_policy_file(seccomp_profile, seccomp_digest)?;
                verify_oci_image(engine, image)?;
            }
        }
        validate_resource_limits(&policy.resources)?;
        validate_egress(&policy.egress)?;
        Ok(Self {
            argv,
            secret_files,
            sandbox,
            policy,
        })
    }
}

impl WorkerRunner for ProcessWorkerRunner {
    fn run(&self, request: &mut RunRequest<'_>) -> Result<WorkerExit, AppError> {
        let profile = request.work.attributes().worker_profile();
        fs::create_dir_all(request.workspace_root).map_err(AppError::worker)?;
        self.verify_runtime()?;
        let secrets = SecretDirectory::create(&self.secret_files)?;
        let (cmd, _seccomp_file) =
            self.command(request, secrets.as_ref().map(SecretDirectory::path))?;
        let waited = spawn_supervised(cmd, request);
        if !matches!(&waited, Ok(ChildWait::Exited { .. })) {
            reclaim_owned_container(
                request.control_root,
                request.work.id().as_str(),
                request.execution_id.as_str(),
                request.attempt_id.as_str(),
            )?;
        }
        match waited {
            Ok(ChildWait::Exited { .. }) => {
                read_or_fail(request.workspace_root, profile).map(WorkerExit::Completed)
            }
            Ok(ChildWait::BudgetExceeded) => {
                persist_timed_out(request.workspace_root, profile).map(WorkerExit::Completed)
            }
            Ok(ChildWait::Parked { control_request_id }) => {
                Ok(WorkerExit::Parked { control_request_id })
            }
            Ok(ChildWait::Aborted { control_request_id }) => {
                Ok(WorkerExit::Aborted { control_request_id })
            }
            Err(_) => Err(AppError::Channel(ChannelReaction::Fail)),
        }
    }

    fn decode(&self, bytes: &[u8]) -> Result<Outcome, AppError> {
        decode_outcome(bytes)
    }
}

impl ProcessWorkerRunner {
    fn command(
        &self,
        request: &RunRequest<'_>,
        secrets: Option<&Path>,
    ) -> Result<(Command, Option<File>), AppError> {
        match &self.sandbox {
            Sandbox::Bubblewrap {
                rootfs,
                seccomp_profile,
                ..
            } => {
                let seccomp_file = inherited_policy_file(seccomp_profile)?;
                let seccomp_fd = seccomp_file.as_raw_fd().to_string();
                let limits = &self.policy.resources;
                let cpu_seconds = limits.cpu_seconds.min(request.budget.as_secs().max(1));
                let process_ceiling =
                    current_uid_process_count().saturating_add(u64::from(limits.max_processes));
                let mut cmd = Command::new("prlimit");
                controlled_environment(&mut cmd);
                cmd.args([
                    &format!("--as={}", limits.memory_bytes),
                    &format!("--nproc={process_ceiling}"),
                    &format!("--nofile={}", limits.max_open_files),
                    &format!("--fsize={}", limits.max_file_bytes),
                    &format!("--cpu={cpu_seconds}"),
                    "--",
                    "bwrap",
                ]);
                cmd.args([
                    "--unshare-all",
                    "--unshare-user",
                    "--new-session",
                    "--die-with-parent",
                    "--disable-userns",
                    "--cap-drop",
                    "ALL",
                    "--seccomp",
                    &seccomp_fd,
                    "--clearenv",
                    "--ro-bind",
                ]);
                cmd.arg(rootfs).arg("/");
                cmd.args(["--bind"])
                    .arg(request.workspace_root)
                    .arg("/workspace");
                cmd.args([
                    "--chdir",
                    "/workspace",
                    "--tmpfs",
                    "/tmp",
                    "--proc",
                    "/proc",
                    "--dev",
                    "/dev",
                    "--tmpfs",
                    "/run",
                    "--dir",
                    "/run/workengine",
                ]);
                cmd.args(["--bind"])
                    .arg(request.control_root)
                    .arg("/run/workengine");
                if let Some(secrets) = secrets {
                    cmd.args(["--dir", "/run/secrets", "--ro-bind"])
                        .arg(secrets)
                        .arg("/run/secrets");
                    for secret in &self.secret_files {
                        cmd.args([
                            "--setenv",
                            &format!("{}_FILE", secret.name),
                            &format!("/run/secrets/{}", secret.name),
                        ]);
                    }
                }
                attach_egress(&mut cmd, &self.policy.egress, true);
                cmd.arg("--").arg(&self.argv[0]).args(&self.argv[1..]);
                Ok((cmd, Some(seccomp_file)))
            }
            Sandbox::Oci {
                engine,
                image,
                seccomp_profile,
                ..
            } => {
                let mut cmd = Command::new(engine);
                controlled_environment(&mut cmd);
                let container = format!("workengine-{}", request.attempt_id);
                record_oci_owner(
                    request.control_root,
                    engine,
                    &container,
                    request.work.id().as_str(),
                    request.execution_id.as_str(),
                    request.attempt_id.as_str(),
                )?;
                let uid = nix::unistd::Uid::current().as_raw().to_string();
                let gid = nix::unistd::Gid::current().as_raw().to_string();
                let limits = &self.policy.resources;
                let cpu_seconds = limits.cpu_seconds.min(request.budget.as_secs().max(1));
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
                    "--security-opt",
                    &format!("seccomp={}", seccomp_profile.display()),
                    "--memory",
                    &limits.memory_bytes.to_string(),
                    "--pids-limit",
                    &limits.max_processes.to_string(),
                    "--ulimit",
                    &format!("nofile={}:{}", limits.max_open_files, limits.max_open_files),
                    "--ulimit",
                    &format!("fsize={}:{}", limits.max_file_bytes, limits.max_file_bytes),
                    "--ulimit",
                    &format!("cpu={cpu_seconds}:{cpu_seconds}"),
                    "--name",
                    &container,
                    "--label",
                    &format!("workengine.work_id={}", request.work.id()),
                    "--label",
                    &format!("workengine.execution_id={}", request.execution_id),
                    "--label",
                    &format!("workengine.attempt_id={}", request.attempt_id),
                    "--user",
                ]);
                cmd.arg(format!("{uid}:{gid}"));
                cmd.arg("--cidfile")
                    .arg(request.control_root.join("container.cid"));
                cmd.arg("--mount").arg(format!(
                    "type=bind,src={},dst=/workspace,rw",
                    request.workspace_root.display()
                ));
                cmd.arg("--mount").arg(format!(
                    "type=bind,src={},dst=/run/workengine,rw",
                    request.control_root.display()
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
                attach_egress(&mut cmd, &self.policy.egress, false);
                cmd.arg(image).arg(&self.argv[0]).args(&self.argv[1..]);
                Ok((cmd, None))
            }
        }
    }

    fn verify_runtime(&self) -> Result<(), AppError> {
        match &self.sandbox {
            Sandbox::Bubblewrap {
                rootfs,
                rootfs_digest: expected,
                seccomp_profile,
                seccomp_digest,
            } => {
                let actual = digest_rootfs(rootfs)?;
                if actual != *expected {
                    return Err(AppError::worker(format!(
                        "bubblewrap rootfs digest changed: expected {expected}, got {actual}"
                    )));
                }
                validate_policy_file(seccomp_profile, seccomp_digest)
            }
            Sandbox::Oci {
                engine,
                image,
                seccomp_profile,
                seccomp_digest,
            } => {
                validate_policy_file(seccomp_profile, seccomp_digest)?;
                verify_oci_image(engine, image)
            }
        }
    }
}

fn validate_digest(value: &str) -> Result<(), AppError> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(AppError::worker(
            "digest must use sha256:<64 lowercase hex>",
        ));
    };
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(AppError::worker(
            "digest must use sha256:<64 lowercase hex>",
        ));
    }
    Ok(())
}

fn image_digest(image: &str) -> Option<&str> {
    let (name, digest) = image.rsplit_once('@')?;
    if name.is_empty() || name.contains('@') {
        return None;
    }
    validate_digest(digest).is_ok().then_some(digest)
}

fn validate_resource_limits(limits: &ResourceLimits) -> Result<(), AppError> {
    if limits.memory_bytes < 16 * 1024 * 1024
        || limits.max_processes == 0
        || limits.max_open_files < 16
        || limits.max_file_bytes == 0
        || limits.cpu_seconds == 0
    {
        return Err(AppError::worker(
            "resource limits must be finite and large enough to launch a Worker",
        ));
    }
    Ok(())
}

fn validate_policy_file(path: &Path, expected: &str) -> Result<(), AppError> {
    validate_digest(expected)?;
    let metadata = path.symlink_metadata().map_err(AppError::worker)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() == 0 {
        return Err(AppError::worker(
            "seccomp profile must be a non-empty regular file and not a symlink",
        ));
    }
    let actual = digest_file(path)?;
    if actual != expected {
        return Err(AppError::worker(format!(
            "seccomp profile digest mismatch: expected {expected}, got {actual}"
        )));
    }
    Ok(())
}

pub fn digest_file(path: &Path) -> Result<String, AppError> {
    let bytes = crate::fs_safe::read(path).map_err(AppError::worker)?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

fn validate_egress(egress: &EgressPolicy) -> Result<(), AppError> {
    let EgressPolicy::BrokerSocket(path) = egress else {
        return Ok(());
    };
    let metadata = path.symlink_metadata().map_err(AppError::worker)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        if metadata.file_type().is_symlink() || !metadata.file_type().is_socket() {
            return Err(AppError::worker(
                "egress broker must be a Unix socket and not a symlink",
            ));
        }
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        return Err(AppError::worker(
            "egress broker sockets are supported only on Unix",
        ));
    }
    Ok(())
}

pub fn digest_rootfs(root: &Path) -> Result<String, AppError> {
    let metadata = root.symlink_metadata().map_err(AppError::worker)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() || root == Path::new("/") {
        return Err(AppError::worker(
            "bubblewrap rootfs must be a dedicated directory",
        ));
    }
    let mut hasher = Sha256::new();
    hash_record(&mut hasher, b'd', &[], metadata_mode(&metadata), &[]);
    hash_tree(root, root, &mut hasher)?;
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

fn hash_tree(root: &Path, directory: &Path, hasher: &mut Sha256) -> Result<(), AppError> {
    let mut entries = fs::read_dir(directory)
        .map_err(AppError::worker)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(AppError::worker)?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let relative = path.strip_prefix(root).map_err(AppError::worker)?;
        let relative = relative
            .to_str()
            .ok_or_else(|| AppError::worker("rootfs path is not valid UTF-8"))?;
        let metadata = path.symlink_metadata().map_err(AppError::worker)?;
        if metadata.file_type().is_symlink() {
            let target = fs::read_link(&path).map_err(AppError::worker)?;
            let target = target
                .to_str()
                .ok_or_else(|| AppError::worker("rootfs symlink is not valid UTF-8"))?;
            hash_record(
                hasher,
                b'l',
                relative.as_bytes(),
                metadata_mode(&metadata),
                target.as_bytes(),
            );
        } else if metadata.is_dir() {
            hash_record(
                hasher,
                b'd',
                relative.as_bytes(),
                metadata_mode(&metadata),
                &[],
            );
            hash_tree(root, &path, hasher)?;
        } else if metadata.is_file() {
            let bytes = crate::fs_safe::read(&path).map_err(AppError::worker)?;
            hash_record(
                hasher,
                b'f',
                relative.as_bytes(),
                metadata_mode(&metadata),
                &bytes,
            );
        } else {
            return Err(AppError::worker(format!(
                "bubblewrap rootfs contains a special file: {relative}"
            )));
        }
    }
    Ok(())
}

fn hash_record(hasher: &mut Sha256, kind: u8, path: &[u8], mode: u32, payload: &[u8]) {
    hasher.update([kind]);
    hasher.update((path.len() as u64).to_be_bytes());
    hasher.update(path);
    hasher.update(mode.to_be_bytes());
    hasher.update((payload.len() as u64).to_be_bytes());
    hasher.update(payload);
}

fn metadata_mode(metadata: &fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode()
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        0
    }
}

fn current_uid_process_count() -> u64 {
    #[cfg(target_os = "linux")]
    {
        let uid = nix::unistd::Uid::current().as_raw().to_string();
        fs::read_dir("/proc")
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .bytes()
                    .all(|byte| byte.is_ascii_digit())
            })
            .filter_map(|entry| {
                let owned = fs::read_to_string(entry.path().join("status"))
                    .ok()
                    .and_then(|status| {
                        status
                            .lines()
                            .find_map(|line| line.strip_prefix("Uid:\t"))
                            .and_then(|ids| ids.split_whitespace().next().map(str::to_owned))
                    })
                    .as_deref()
                    == Some(uid.as_str());
                owned.then(|| {
                    fs::read_dir(entry.path().join("task"))
                        .map(|tasks| tasks.count() as u64)
                        .unwrap_or(1)
                })
            })
            .sum()
    }
    #[cfg(not(target_os = "linux"))]
    {
        0
    }
}

fn inherited_policy_file(path: &Path) -> Result<File, AppError> {
    use std::os::unix::fs::OpenOptionsExt;

    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
        .open(path)
        .map_err(AppError::worker)?;
    if !file.metadata().map_err(AppError::worker)?.is_file() {
        return Err(AppError::worker(
            "seccomp profile must remain a regular file",
        ));
    }
    nix::fcntl::fcntl(
        &file,
        nix::fcntl::FcntlArg::F_SETFD(nix::fcntl::FdFlag::empty()),
    )
    .map_err(AppError::worker)?;
    Ok(file)
}

fn controlled_environment(command: &mut Command) {
    command
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin");
}

fn attach_egress(command: &mut Command, policy: &EgressPolicy, bubblewrap: bool) {
    let EgressPolicy::BrokerSocket(path) = policy else {
        return;
    };
    if bubblewrap {
        command
            .args(["--ro-bind"])
            .arg(path)
            .arg("/run/workengine-egress.sock")
            .args([
                "--setenv",
                "WORKENGINE_EGRESS_SOCKET",
                "/run/workengine-egress.sock",
            ]);
    } else {
        command
            .arg("--mount")
            .arg(format!(
                "type=bind,src={},dst=/run/workengine-egress.sock,readonly",
                path.display()
            ))
            .args([
                "--env",
                "WORKENGINE_EGRESS_SOCKET=/run/workengine-egress.sock",
            ]);
    }
}

fn verify_oci_image(engine: &str, image: &str) -> Result<(), AppError> {
    let expected =
        image_digest(image).ok_or_else(|| AppError::worker("OCI image must be digest pinned"))?;
    let mut command = Command::new(engine);
    controlled_environment(&mut command);
    let inspected = command
        .args([
            "image",
            "inspect",
            "--format",
            "{{json .RepoDigests}}",
            image,
        ])
        .output()
        .map_err(AppError::worker)?;
    if !inspected.status.success() {
        return Err(AppError::worker("failed to inspect pinned OCI image"));
    }
    if !oci_inspection_matches(&inspected.stdout, expected)? {
        return Err(AppError::worker(
            "OCI engine did not resolve the configured image to its pinned digest",
        ));
    }
    Ok(())
}

fn oci_inspection_matches(bytes: &[u8], expected: &str) -> Result<bool, AppError> {
    let repo_digests: Vec<String> = serde_json::from_slice(bytes).map_err(AppError::worker)?;
    Ok(repo_digests.iter().any(|value| {
        value
            .rsplit_once('@')
            .is_some_and(|(_, digest)| digest == expected)
    }))
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OciOwner {
    schema_version: u32,
    engine: String,
    container: String,
    work_id: String,
    execution_id: String,
    attempt_id: String,
}

fn record_oci_owner(
    control_root: &Path,
    engine: &str,
    container: &str,
    work_id: &str,
    execution_id: &str,
    attempt_id: &str,
) -> Result<(), AppError> {
    let owner = OciOwner {
        schema_version: 1,
        engine: engine.to_owned(),
        container: container.to_owned(),
        work_id: work_id.to_owned(),
        execution_id: execution_id.to_owned(),
        attempt_id: attempt_id.to_owned(),
    };
    crate::fs_safe::write(
        &control_root.join("oci-owner.json"),
        &serde_json::to_vec(&owner).map_err(AppError::worker)?,
    )
}

pub(crate) fn reclaim_owned_container(
    control_root: &Path,
    work_id: &str,
    execution_id: &str,
    attempt_id: &str,
) -> Result<bool, AppError> {
    let bytes = match crate::fs_safe::read(&control_root.join("oci-owner.json")) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(AppError::worker(error)),
    };
    let owner: OciOwner = serde_json::from_slice(&bytes).map_err(AppError::outcome_schema)?;
    if owner.schema_version != 1
        || owner.work_id != work_id
        || owner.execution_id != execution_id
        || owner.attempt_id != attempt_id
    {
        return Err(AppError::worker(
            "OCI ownership proof does not match the active attempt",
        ));
    }
    if owner.engine != "docker" && owner.engine != "podman" {
        return Err(AppError::worker(
            "OCI ownership proof names an invalid engine",
        ));
    }
    let mut inspect = Command::new(&owner.engine);
    controlled_environment(&mut inspect);
    let inspected = inspect
        .args([
            "inspect",
            "--format",
            "{{json .Config.Labels}}",
            &owner.container,
        ])
        .output()
        .map_err(AppError::worker)?;
    if !inspected.status.success() {
        return Ok(false);
    }
    let labels: std::collections::HashMap<String, String> =
        serde_json::from_slice(&inspected.stdout).map_err(AppError::worker)?;
    if labels.get("workengine.work_id").map(String::as_str) != Some(work_id)
        || labels.get("workengine.execution_id").map(String::as_str) != Some(execution_id)
        || labels.get("workengine.attempt_id").map(String::as_str) != Some(attempt_id)
    {
        return Err(AppError::worker(
            "OCI labels do not prove attempt ownership",
        ));
    }
    let mut remove = Command::new(&owner.engine);
    controlled_environment(&mut remove);
    let removed = remove
        .args(["rm", "--force", &owner.container])
        .status()
        .map_err(AppError::worker)?;
    if !removed.success() {
        return Err(AppError::worker("failed to tear down owned OCI container"));
    }
    Ok(true)
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
            crate::fs_safe::create_private(&path, secret.value.as_bytes())?;
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

fn read_or_fail(root: &Path, profile: &str) -> Result<Outcome, AppError> {
    let dest = root.join("outcome.json");
    match crate::fs_safe::read(&dest) {
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
    crate::fs_safe::write(&root.join("outcome.json"), json.as_bytes())?;
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard, OnceLock};
    use std::time::Duration;
    use workengine_application::DiscardAttemptRecorder;
    use workengine_domain::{AttemptId, ExecutionId, Work, WorkAttributes, WorkId};

    fn work() -> Work {
        Work::new(
            WorkId::parse("work-1").unwrap(),
            WorkAttributes::new("goal", "echo").unwrap(),
            1,
        )
        .unwrap()
    }

    fn run(
        runner: &ProcessWorkerRunner,
        work: &Work,
        workspace_root: &Path,
        budget: Duration,
    ) -> Result<Outcome, AppError> {
        let execution_id = ExecutionId::parse("execution-test").unwrap();
        let attempt_id = AttemptId::parse("attempt-test").unwrap();
        let mut recorder = DiscardAttemptRecorder;
        match runner.run(&mut RunRequest {
            work,
            execution_id: &execution_id,
            attempt_id: &attempt_id,
            workspace_root,
            control_root: workspace_root,
            budget,
            recorder: &mut recorder,
        })? {
            WorkerExit::Completed(outcome) => Ok(outcome),
            WorkerExit::Parked { .. } | WorkerExit::Aborted { .. } => {
                Err(AppError::worker("unexpected test control exit"))
            }
        }
    }

    fn sandbox(root: &tempfile::TempDir) -> Sandbox {
        let rootfs = root.path().join("rootfs");
        for directory in ["dev", "proc", "tmp", "run", "run/secrets", "workspace"] {
            fs::create_dir_all(rootfs.join(directory)).unwrap();
        }
        fs::copy("/usr/bin/busybox", rootfs.join("busybox")).unwrap();
        let seccomp_profile = root.path().join("seccomp.bpf");
        fs::write(&seccomp_profile, [0x06, 0, 0, 0, 0, 0, 0xff, 0x7f]).unwrap();
        let rootfs_digest = digest_rootfs(&rootfs).unwrap();
        let seccomp_digest = digest_file(&seccomp_profile).unwrap();
        Sandbox::Bubblewrap {
            rootfs,
            rootfs_digest,
            seccomp_profile,
            seccomp_digest,
        }
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

    fn sandbox_test_lock() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[test]
    fn oci_command_carries_attempt_ownership_labels() {
        let dir = tempfile::tempdir().unwrap();
        let work = work();
        let execution_id = ExecutionId::parse("execution-owner").unwrap();
        let attempt_id = AttemptId::parse("attempt-owner").unwrap();
        let mut recorder = DiscardAttemptRecorder;
        let request = RunRequest {
            work: &work,
            execution_id: &execution_id,
            attempt_id: &attempt_id,
            workspace_root: dir.path(),
            control_root: dir.path(),
            budget: Duration::from_secs(1),
            recorder: &mut recorder,
        };
        let runner = ProcessWorkerRunner {
            argv: vec!["worker".to_owned()],
            secret_files: Vec::new(),
            sandbox: Sandbox::Oci {
                engine: "docker".to_owned(),
                image: format!("example@sha256:{}", "1".repeat(64)),
                seccomp_profile: {
                    let path = dir.path().join("seccomp.json");
                    fs::write(&path, "{}").unwrap();
                    path
                },
                seccomp_digest: digest_file(&dir.path().join("seccomp.json")).unwrap(),
            },
            policy: WorkerPolicy::default(),
        };
        let (command, _) = runner.command(&request, None).unwrap();
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(args.contains(&"workengine.work_id=work-1".to_owned()));
        assert!(args.contains(&"workengine.execution_id=execution-owner".to_owned()));
        assert!(args.contains(&"workengine.attempt_id=attempt-owner".to_owned()));
        assert!(args.contains(&"--network".to_owned()));
        assert!(args.contains(&"none".to_owned()));
        assert!(args.contains(&"--cap-drop".to_owned()));
        assert!(args.contains(&"ALL".to_owned()));
        assert!(args.contains(&"--memory".to_owned()));
        assert!(args.iter().any(|arg| arg.starts_with("seccomp=")));
        assert!(!args.iter().any(|arg| arg.contains("EGRESS")));
        assert!(dir.path().join("oci-owner.json").exists());
        let environment = command
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.map(|value| value.to_string_lossy().into_owned()),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            environment,
            vec![(
                "PATH".to_owned(),
                Some("/usr/sbin:/usr/bin:/sbin:/bin".to_owned())
            )]
        );
    }

    #[test]
    fn bubblewrap_command_applies_policy_without_a_network_capability() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let runner =
            ProcessWorkerRunner::new(vec!["/busybox".to_owned()], vec![], sandbox(&runtime))
                .unwrap();
        let work = work();
        let execution_id = ExecutionId::parse("execution-policy").unwrap();
        let attempt_id = AttemptId::parse("attempt-policy").unwrap();
        let mut recorder = DiscardAttemptRecorder;
        let request = RunRequest {
            work: &work,
            execution_id: &execution_id,
            attempt_id: &attempt_id,
            workspace_root: dir.path(),
            control_root: dir.path(),
            budget: Duration::from_secs(10),
            recorder: &mut recorder,
        };
        let (command, seccomp) = runner.command(&request, None).unwrap();
        assert!(seccomp.is_some());
        assert_eq!(command.get_program(), "prlimit");
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(args.iter().any(|arg| arg.starts_with("--as=")));
        assert!(args.iter().any(|arg| arg.starts_with("--nproc=")));
        assert!(args.windows(2).any(|args| args == ["--cap-drop", "ALL"]));
        assert!(args.iter().any(|arg| arg == "--seccomp"));
        assert!(args.iter().any(|arg| arg == "--unshare-all"));
        assert!(!args.iter().any(|arg| arg == "--share-net"));
    }

    #[cfg(unix)]
    #[test]
    fn bubblewrap_policy_command_can_launch() {
        let _guard = sandbox_test_lock();
        if unavailable_sandbox() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let runner = ProcessWorkerRunner::new(
            vec!["/busybox".to_owned(), "true".to_owned()],
            vec![],
            sandbox(&runtime),
        )
        .unwrap();
        let work = work();
        let execution_id = ExecutionId::parse("execution-launch").unwrap();
        let attempt_id = AttemptId::parse("attempt-launch").unwrap();
        let mut recorder = DiscardAttemptRecorder;
        let request = RunRequest {
            work: &work,
            execution_id: &execution_id,
            attempt_id: &attempt_id,
            workspace_root: dir.path(),
            control_root: dir.path(),
            budget: Duration::from_secs(10),
            recorder: &mut recorder,
        };
        let (mut command, _seccomp) = runner.command(&request, None).unwrap();
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "bubblewrap policy failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn oci_inspection_requires_the_exact_configured_digest() {
        let expected = format!("sha256:{}", "1".repeat(64));
        let matching = format!(r#"["registry.example/agent@{expected}"]"#);
        assert!(oci_inspection_matches(matching.as_bytes(), &expected).unwrap());
        let different = format!(r#"["registry.example/agent@sha256:{}"]"#, "2".repeat(64));
        assert!(!oci_inspection_matches(different.as_bytes(), &expected).unwrap());
        assert!(oci_inspection_matches(b"not-json", &expected).is_err());
    }

    #[test]
    fn rootfs_digest_detects_file_and_symlink_mutation_without_following() {
        let root = tempfile::tempdir().unwrap();
        let sandbox = sandbox(&root);
        let runner =
            ProcessWorkerRunner::new(vec!["/busybox".to_owned()], vec![], sandbox).unwrap();
        fs::write(root.path().join("rootfs/busybox"), "changed").unwrap();
        let error = runner.verify_runtime().unwrap_err();
        assert!(error.to_string().contains("digest changed"), "{error}");

        let other = tempfile::tempdir().unwrap();
        fs::create_dir(other.path().join("rootfs")).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/etc/passwd", other.path().join("rootfs/link")).unwrap();
            let first = digest_rootfs(&other.path().join("rootfs")).unwrap();
            fs::remove_file(other.path().join("rootfs/link")).unwrap();
            std::os::unix::fs::symlink("/etc/shadow", other.path().join("rootfs/link")).unwrap();
            let second = digest_rootfs(&other.path().join("rootfs")).unwrap();
            assert_ne!(first, second);
        }
    }

    #[test]
    fn seccomp_digest_detects_policy_mutation() {
        let root = tempfile::tempdir().unwrap();
        let sandbox = sandbox(&root);
        let runner =
            ProcessWorkerRunner::new(vec!["/busybox".to_owned()], vec![], sandbox).unwrap();
        fs::write(root.path().join("seccomp.bpf"), "changed").unwrap();
        let error = runner.verify_runtime().unwrap_err();
        assert!(
            error
                .to_string()
                .contains("seccomp profile digest mismatch"),
            "{error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn egress_requires_an_explicit_unix_broker_socket() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("broker.sock");
        let seccomp = dir.path().join("seccomp.json");
        fs::write(&seccomp, "{}").unwrap();
        let runner = ProcessWorkerRunner {
            argv: vec!["worker".to_owned()],
            secret_files: vec![],
            sandbox: Sandbox::Oci {
                engine: "docker".to_owned(),
                image: format!("example@sha256:{}", "1".repeat(64)),
                seccomp_profile: seccomp,
                seccomp_digest: digest_file(&dir.path().join("seccomp.json")).unwrap(),
            },
            policy: WorkerPolicy {
                resources: ResourceLimits::default(),
                egress: EgressPolicy::BrokerSocket(socket),
            },
        };
        let work = work();
        let execution_id = ExecutionId::parse("execution-1").unwrap();
        let attempt_id = AttemptId::parse("attempt-1").unwrap();
        let mut recorder = DiscardAttemptRecorder;
        let request = RunRequest {
            work: &work,
            execution_id: &execution_id,
            attempt_id: &attempt_id,
            workspace_root: dir.path(),
            control_root: dir.path(),
            budget: Duration::from_secs(1),
            recorder: &mut recorder,
        };
        let (command, _) = runner.command(&request, None).unwrap();
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(
            args.iter()
                .any(|arg| arg == "WORKENGINE_EGRESS_SOCKET=/run/workengine-egress.sock")
        );
        assert!(args.iter().any(|arg| arg.contains("broker.sock")));
        assert!(args.windows(2).any(|args| args == ["--network", "none"]));
    }

    #[cfg(unix)]
    #[test]
    fn outcome_reader_does_not_follow_worker_symlink() {
        let dir = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink("/etc/passwd", dir.path().join("outcome.json")).unwrap();
        assert!(read_or_fail(dir.path(), "echo").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn missing_outcome_is_failed_not_succeeded() {
        let _guard = sandbox_test_lock();
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
        let outcome = run(&runner, &work, dir.path(), Duration::from_secs(2)).unwrap();
        assert_eq!(outcome.kind(), OutcomeKind::Failed);
        assert!(!dir.path().join("outcome.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn schema_outcome_file_is_decoded() {
        let _guard = sandbox_test_lock();
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
        let outcome = run(&runner, &work, dir.path(), Duration::from_secs(2)).unwrap();
        assert_eq!(outcome.kind(), OutcomeKind::Succeeded);
    }

    #[cfg(unix)]
    #[test]
    fn undeclared_env_is_not_inherited() {
        let _guard = sandbox_test_lock();
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
        let _ = run(&runner, &work, dir.path(), Duration::from_secs(2)).unwrap();
        let leaked = fs::read_to_string(dir.path().join("leaked.txt")).unwrap();
        assert!(
            leaked.trim().is_empty(),
            "child inherited undeclared HOME: {leaked}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn declared_secret_is_exposed_only_as_a_file_path() {
        let _guard = sandbox_test_lock();
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
        let _ = run(&runner, &work, dir.path(), Duration::from_secs(2)).unwrap();
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
