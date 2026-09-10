use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use toml::Value;
use workengine_adapters_worker::{Sandbox, SecretFile};

/// Worker profile loaded from configuration. Secrets are transient file references.
#[derive(Debug)]
pub struct ResolvedProfile {
    pub argv: Vec<String>,
    pub secret_files: Vec<SecretFile>,
    pub retry_limit: u32,
    pub checkout: Option<PathBuf>,
    pub sandbox: Sandbox,
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
    let secret_files = match prof.get("secret_file") {
        None => Vec::new(),
        Some(Value::Table(map)) => resolve_secret_files(name, map, getenv)?,
        Some(_) => bail!("profile {name} secret_file must be a table of fromEnv references"),
    };
    let sandbox = resolve_sandbox(name, prof.get("sandbox"))?;
    Ok(ResolvedProfile {
        argv,
        secret_files,
        retry_limit,
        checkout,
        sandbox,
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
            if table.keys().any(|key| key != "type" && key != "rootfs") {
                bail!("profile {profile} bubblewrap sandbox has unknown fields");
            }
            Ok(Sandbox::Bubblewrap {
                rootfs: PathBuf::from(rootfs),
            })
        }
        "oci" => {
            let Some(engine) = table.get("engine").and_then(Value::as_str) else {
                bail!("profile {profile} OCI sandbox.engine is required");
            };
            let Some(image) = table.get("image").and_then(Value::as_str) else {
                bail!("profile {profile} OCI sandbox.image is required");
            };
            if table
                .keys()
                .any(|key| key != "type" && key != "engine" && key != "image")
            {
                bail!("profile {profile} OCI sandbox has unknown fields");
            }
            Ok(Sandbox::Oci {
                engine: engine.to_owned(),
                image: image.to_owned(),
            })
        }
        _ => bail!("profile {profile} sandbox.type must be bubblewrap or oci"),
    }
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
) -> anyhow::Result<Vec<SecretFile>> {
    let mut out = Vec::new();
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
        out.push(SecretFile::new(key.clone(), val).map_err(anyhow::Error::from)?);
    }
    Ok(out)
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
sandbox = { type = "oci", engine = "docker", image = "example@sha256:123" }

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
sandbox = { type = "oci", engine = "docker", image = "example@sha256:123" }

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
sandbox = { type = "oci", engine = "docker", image = "example@sha256:123" }

[profile.broken.secret_file]
TOKEN = "inline-secret"

[profile.ok]
argv = ["/bin/true"]
sandbox = { type = "oci", engine = "docker", image = "example@sha256:123" }
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
sandbox = { type = "oci", engine = "docker", image = "example@sha256:123" }
"#,
        )
        .unwrap();
        let table = load_profile_from_dir(dir.path(), "ok").unwrap();
        let got = resolve_profile(&table, "ok", |_| None).unwrap();
        assert_eq!(got.argv, vec!["/bin/true"]);
    }
}
