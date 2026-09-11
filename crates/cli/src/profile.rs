use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use toml::Value;
use workengine_adapters_worker::{EgressPolicy, ResourceLimits, Sandbox, SecretFile, WorkerPolicy};
use workengine_domain::{SecretRef, SecretSource};

/// Worker profile loaded from configuration. Secrets are transient file references.
#[derive(Debug)]
pub struct ResolvedProfile {
    pub argv: Vec<String>,
    pub secret_files: Vec<SecretFile>,
    pub secret_refs: Vec<SecretRef>,
    pub retry_limit: u32,
    pub checkout: Option<PathBuf>,
    pub sandbox: Sandbox,
    pub policy: WorkerPolicy,
}

pub fn load_table(path: &Path) -> anyhow::Result<toml::Table> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("read config {}", path.display()))?;
    text.parse()
        .with_context(|| format!("parse config {}", path.display()))
}

/// Load exactly one profile from `<config-dir>/<profile>.toml`. Parsing another
/// profile is deliberately impossible here, so a broken unrelated file cannot
/// block a Work.
pub fn load_profile_from_dir(config_dir: &Path, name: &str) -> anyhow::Result<toml::Table> {
    let path = config_dir.join(format!("{name}.toml"));
    let body = load_table(&path)?;
    let mut profiles = toml::Table::new();
    profiles.insert(name.to_owned(), Value::Table(body));
    let mut table = toml::Table::new();
    table.insert("profile".to_owned(), Value::Table(profiles));
    Ok(table)
}

pub fn resolve_profile(
    table: &toml::Table,
    name: &str,
    getenv: impl Fn(&str) -> Option<String>,
) -> anyhow::Result<ResolvedProfile> {
    let Some(profiles) = table.get("profile").and_then(Value::as_table) else {
        bail!("config has no [profile.*] tables");
    };
    let Some(prof) = profiles.get(name).and_then(Value::as_table) else {
        bail!("unknown worker profile {name}");
    };
    let argv = string_array(prof.get("argv"), "argv")?;
    if argv.is_empty() || argv[0].is_empty() {
        bail!("profile {name} argv is empty");
    }
    let retry_limit = match prof.get("retry_limit") {
        None => 0,
        Some(Value::Integer(n)) if *n >= 0 => u32::try_from(*n).unwrap_or(u32::MAX),
        Some(_) => bail!("profile {name} retry_limit must be a non-negative integer"),
    };
    let checkout = match prof.get("checkout") {
        None => None,
        Some(Value::String(s)) => Some(PathBuf::from(s)),
        Some(_) => bail!("profile {name} checkout must be a string path"),
    };
    if prof.contains_key("env") {
        bail!("profile {name} env is unsupported in schema v2; use [profile.{name}.secret_file]");
    }
    let (secret_files, secret_refs) = match prof.get("secret_file") {
        None => (Vec::new(), Vec::new()),
        Some(Value::Table(map)) => resolve_secret_files(name, map, getenv)?,
        Some(_) => bail!("profile {name} secret_file must be a table of fromEnv references"),
    };
    let sandbox = resolve_sandbox(name, prof.get("sandbox"))?;
    let policy = resolve_policy(name, prof.get("policy"))?;
    if prof.keys().any(|key| {
        !matches!(
            key.as_str(),
            "argv" | "retry_limit" | "checkout" | "secret_file" | "sandbox" | "policy"
        )
    }) {
        bail!("profile {name} has unknown fields");
    }
    Ok(ResolvedProfile {
        argv,
        secret_files,
        secret_refs,
        retry_limit,
        checkout,
        sandbox,
        policy,
    })
}

fn resolve_sandbox(profile: &str, value: Option<&Value>) -> anyhow::Result<Sandbox> {
    let Some(table) = value.and_then(Value::as_table) else {
        bail!("profile {profile} must declare a sandbox");
    };
    let Some(kind) = table.get("type").and_then(Value::as_str) else {
        bail!("profile {profile} sandbox.type is required");
    };
    match kind {
        "bubblewrap" => {
            let Some(rootfs) = table.get("rootfs").and_then(Value::as_str) else {
                bail!("profile {profile} bubblewrap sandbox.rootfs is required");
            };
            let Some(rootfs_digest) = table.get("digest").and_then(Value::as_str) else {
                bail!("profile {profile} bubblewrap sandbox.digest is required");
            };
            let Some(seccomp) = table.get("seccomp").and_then(Value::as_str) else {
                bail!("profile {profile} bubblewrap sandbox.seccomp is required");
            };
            let Some(seccomp_digest) = table.get("seccomp_digest").and_then(Value::as_str) else {
                bail!("profile {profile} bubblewrap sandbox.seccomp_digest is required");
            };
            if table.keys().any(|key| {
                !matches!(
                    key.as_str(),
                    "type" | "rootfs" | "digest" | "seccomp" | "seccomp_digest"
                )
            }) {
                bail!("profile {profile} bubblewrap sandbox has unknown fields");
            }
            Ok(Sandbox::Bubblewrap {
                rootfs: PathBuf::from(rootfs),
                rootfs_digest: rootfs_digest.to_owned(),
                seccomp_profile: PathBuf::from(seccomp),
                seccomp_digest: seccomp_digest.to_owned(),
            })
        }
        "oci" => {
            let Some(engine) = table.get("engine").and_then(Value::as_str) else {
                bail!("profile {profile} OCI sandbox.engine is required");
            };
            let Some(image) = table.get("image").and_then(Value::as_str) else {
                bail!("profile {profile} OCI sandbox.image is required");
            };
            let Some(seccomp) = table.get("seccomp").and_then(Value::as_str) else {
                bail!("profile {profile} OCI sandbox.seccomp is required");
            };
            let Some(seccomp_digest) = table.get("seccomp_digest").and_then(Value::as_str) else {
                bail!("profile {profile} OCI sandbox.seccomp_digest is required");
            };
            if table.keys().any(|key| {
                !matches!(
                    key.as_str(),
                    "type" | "engine" | "image" | "seccomp" | "seccomp_digest"
                )
            }) {
                bail!("profile {profile} OCI sandbox has unknown fields");
            }
            Ok(Sandbox::Oci {
                engine: engine.to_owned(),
                image: image.to_owned(),
                seccomp_profile: PathBuf::from(seccomp),
                seccomp_digest: seccomp_digest.to_owned(),
            })
        }
        _ => bail!("profile {profile} sandbox.type must be bubblewrap or oci"),
    }
}

fn resolve_policy(profile: &str, value: Option<&Value>) -> anyhow::Result<WorkerPolicy> {
    let Some(value) = value else {
        return Ok(WorkerPolicy::default());
    };
    let Some(table) = value.as_table() else {
        bail!("profile {profile} policy must be a table");
    };
    if table.keys().any(|key| {
        !matches!(
            key.as_str(),
            "memory_bytes"
                | "max_processes"
                | "max_open_files"
                | "max_file_bytes"
                | "cpu_seconds"
                | "egress_broker_socket"
        )
    }) {
        bail!("profile {profile} policy has unknown fields");
    }
    let defaults = ResourceLimits::default();
    let resources = ResourceLimits {
        memory_bytes: policy_u64(table, "memory_bytes", defaults.memory_bytes, profile)?,
        max_processes: policy_u32(table, "max_processes", defaults.max_processes, profile)?,
        max_open_files: policy_u32(table, "max_open_files", defaults.max_open_files, profile)?,
        max_file_bytes: policy_u64(table, "max_file_bytes", defaults.max_file_bytes, profile)?,
        cpu_seconds: policy_u64(table, "cpu_seconds", defaults.cpu_seconds, profile)?,
    };
    let egress = match table.get("egress_broker_socket") {
        None => EgressPolicy::Deny,
        Some(Value::String(path)) => EgressPolicy::BrokerSocket(PathBuf::from(path)),
        Some(_) => bail!("profile {profile} policy.egress_broker_socket must be a path string"),
    };
    Ok(WorkerPolicy { resources, egress })
}

fn policy_u64(
    table: &toml::Table,
    field: &str,
    default: u64,
    profile: &str,
) -> anyhow::Result<u64> {
    match table.get(field) {
        None => Ok(default),
        Some(Value::Integer(value)) if *value > 0 => u64::try_from(*value)
            .map_err(|_| anyhow::anyhow!("profile {profile} policy.{field} is too large")),
        Some(_) => bail!("profile {profile} policy.{field} must be a positive integer"),
    }
}

fn policy_u32(
    table: &toml::Table,
    field: &str,
    default: u32,
    profile: &str,
) -> anyhow::Result<u32> {
    let value = policy_u64(table, field, u64::from(default), profile)?;
    u32::try_from(value)
        .map_err(|_| anyhow::anyhow!("profile {profile} policy.{field} is too large"))
}

fn string_array(value: Option<&Value>, field: &str) -> anyhow::Result<Vec<String>> {
    let Some(Value::Array(items)) = value else {
        bail!("{field} must be an array of strings");
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let Some(s) = item.as_str() else {
            bail!("{field} must be an array of strings");
        };
        out.push(s.to_owned());
    }
    Ok(out)
}

fn resolve_secret_files(
    profile: &str,
    map: &toml::Table,
    getenv: impl Fn(&str) -> Option<String>,
) -> anyhow::Result<(Vec<SecretFile>, Vec<SecretRef>)> {
    let mut files = Vec::new();
    let mut refs = Vec::new();
    for (key, value) in map {
        let Some(table) = value.as_table() else {
            bail!(
                "profile {profile} secret_file.{key} must be {{ fromEnv = \"NAME\" }}, not an inline value"
            );
        };
        let Some(from) = table.get("fromEnv").and_then(Value::as_str) else {
            bail!("profile {profile} secret_file.{key} must set fromEnv");
        };
        if table.keys().any(|k| k != "fromEnv") {
            bail!("profile {profile} secret_file.{key} only fromEnv is allowed");
        }
        let Some(val) = getenv(from) else {
            bail!("environment variable {from} is not set for profile {profile} secret_file.{key}");
        };
        files.push(SecretFile::new(key.clone(), val).map_err(anyhow::Error::from)?);
        refs.push(SecretRef::new(
            key.clone(),
            SecretSource::environment_variable(from)?,
        )?);
    }
    Ok((files, refs))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(text: &str) -> toml::Table {
        text.parse().unwrap()
    }

    #[test]
    fn loads_argv_and_secret_file_refs() {
        let t = table(
            r#"
[profile.echo]
argv = ["/bin/true"]
retry_limit = 2
checkout = "/src"
sandbox = { type = "oci", engine = "docker", image = "example@sha256:123", seccomp = "/etc/workengine/seccomp.json", seccomp_digest = "sha256:1111111111111111111111111111111111111111111111111111111111111111" }

[profile.echo.secret_file]
TOKEN = { fromEnv = "TOKEN" }
"#,
        );
        let got =
            resolve_profile(&t, "echo", |k| (k == "TOKEN").then(|| "secret".to_owned())).unwrap();
        assert_eq!(got.argv, vec!["/bin/true"]);
        assert_eq!(got.retry_limit, 2);
        assert_eq!(got.checkout.as_deref(), Some(Path::new("/src")));
        assert_eq!(got.secret_files.len(), 1);
        assert!(matches!(got.sandbox, Sandbox::Oci { .. }));
    }

    #[test]
    fn rejects_inline_env_values() {
        let t = table(
            r#"
[profile.echo]
argv = ["/bin/true"]
sandbox = { type = "oci", engine = "docker", image = "example@sha256:123", seccomp = "/etc/workengine/seccomp.json", seccomp_digest = "sha256:1111111111111111111111111111111111111111111111111111111111111111" }

[profile.echo.secret_file]
TOKEN = "inline-secret"
"#,
        );
        let err = resolve_profile(&t, "echo", |_| None).unwrap_err();
        assert!(err.to_string().contains("fromEnv"), "{err}");
    }

    #[test]
    fn broken_other_profile_does_not_block() {
        let t = table(
            r#"
[profile.broken]
argv = ["/bin/false"]
sandbox = { type = "oci", engine = "docker", image = "example@sha256:123", seccomp = "/etc/workengine/seccomp.json", seccomp_digest = "sha256:1111111111111111111111111111111111111111111111111111111111111111" }

[profile.broken.secret_file]
TOKEN = "inline-secret"

[profile.ok]
argv = ["/bin/true"]
sandbox = { type = "oci", engine = "docker", image = "example@sha256:123", seccomp = "/etc/workengine/seccomp.json", seccomp_digest = "sha256:1111111111111111111111111111111111111111111111111111111111111111" }
"#,
        );
        let got = resolve_profile(&t, "ok", |_| None).unwrap();
        assert_eq!(got.argv, vec!["/bin/true"]);
    }

    #[test]
    fn profile_dir_loads_only_the_requested_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("broken.toml"), "this is = invalid").unwrap();
        std::fs::write(
            dir.path().join("ok.toml"),
            r#"
argv = ["/bin/true"]
sandbox = { type = "oci", engine = "docker", image = "example@sha256:123", seccomp = "/etc/workengine/seccomp.json", seccomp_digest = "sha256:1111111111111111111111111111111111111111111111111111111111111111" }
"#,
        )
        .unwrap();
        let table = load_profile_from_dir(dir.path(), "ok").unwrap();
        let got = resolve_profile(&table, "ok", |_| None).unwrap();
        assert_eq!(got.argv, vec!["/bin/true"]);
    }

    #[test]
    fn policy_is_deny_by_default_and_extra_egress_is_explicit() {
        let defaults = table(
            r#"
[profile.echo]
argv = ["/bin/true"]
sandbox = { type = "oci", engine = "docker", image = "example@sha256:123", seccomp = "/etc/workengine/seccomp.json", seccomp_digest = "sha256:1111111111111111111111111111111111111111111111111111111111111111" }
"#,
        );
        let resolved = resolve_profile(&defaults, "echo", |_| None).unwrap();
        assert_eq!(resolved.policy.egress, EgressPolicy::Deny);

        let brokered = table(
            r#"
[profile.echo]
argv = ["/bin/true"]
sandbox = { type = "oci", engine = "docker", image = "example@sha256:123", seccomp = "/etc/workengine/seccomp.json", seccomp_digest = "sha256:1111111111111111111111111111111111111111111111111111111111111111" }
policy = { memory_bytes = 536870912, max_processes = 32, egress_broker_socket = "/run/workengine/broker.sock" }
"#,
        );
        let resolved = resolve_profile(&brokered, "echo", |_| None).unwrap();
        assert_eq!(resolved.policy.resources.memory_bytes, 536_870_912);
        assert_eq!(resolved.policy.resources.max_processes, 32);
        assert_eq!(
            resolved.policy.egress,
            EgressPolicy::BrokerSocket(PathBuf::from("/run/workengine/broker.sock"))
        );
    }

    #[test]
    fn bubblewrap_requires_rootfs_digest_and_seccomp_policy() {
        let missing = table(
            r#"
[profile.echo]
argv = ["/bin/true"]
sandbox = { type = "bubblewrap", rootfs = "/runtime" }
"#,
        );
        let error = resolve_profile(&missing, "echo", |_| None).unwrap_err();
        assert!(error.to_string().contains("sandbox.digest"), "{error}");
    }
}
