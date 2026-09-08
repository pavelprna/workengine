use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use toml::Value;

/// Worker profile loaded from configuration. Secrets are env references only.
#[derive(Debug)]
pub struct ResolvedProfile {
    pub argv: Vec<String>,
    pub env: Vec<(String, String)>,
    pub retry_limit: u32,
    pub checkout: Option<PathBuf>,
}

pub fn load_table(path: &Path) -> anyhow::Result<toml::Table> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("read config {}", path.display()))?;
    text.parse()
        .with_context(|| format!("parse config {}", path.display()))
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
    let env = match prof.get("env") {
        None => Vec::new(),
        Some(Value::Table(map)) => resolve_env(name, map, getenv)?,
        Some(_) => bail!("profile {name} env must be a table of fromEnv references"),
    };
    Ok(ResolvedProfile {
        argv,
        env,
        retry_limit,
        checkout,
    })
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

fn resolve_env(
    profile: &str,
    map: &toml::Table,
    getenv: impl Fn(&str) -> Option<String>,
) -> anyhow::Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for (key, value) in map {
        let Some(table) = value.as_table() else {
            bail!(
                "profile {profile} env.{key} must be {{ fromEnv = \"NAME\" }}, not an inline value"
            );
        };
        let Some(from) = table.get("fromEnv").and_then(Value::as_str) else {
            bail!("profile {profile} env.{key} must set fromEnv");
        };
        if table.keys().any(|k| k != "fromEnv") {
            bail!("profile {profile} env.{key} only fromEnv is allowed");
        }
        let Some(val) = getenv(from) else {
            bail!("environment variable {from} is not set for profile {profile} env.{key}");
        };
        out.push((key.clone(), val));
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
    fn loads_argv_and_env_refs() {
        let t = table(
            r#"
[profile.echo]
argv = ["/bin/true"]
retry_limit = 2
checkout = "/src"

[profile.echo.env]
TOKEN = { fromEnv = "TOKEN" }
"#,
        );
        let got =
            resolve_profile(&t, "echo", |k| (k == "TOKEN").then(|| "secret".to_owned())).unwrap();
        assert_eq!(got.argv, vec!["/bin/true"]);
        assert_eq!(got.retry_limit, 2);
        assert_eq!(got.checkout.as_deref(), Some(Path::new("/src")));
        assert_eq!(got.env, vec![("TOKEN".to_owned(), "secret".to_owned())]);
    }

    #[test]
    fn rejects_inline_env_values() {
        let t = table(
            r#"
[profile.echo]
argv = ["/bin/true"]

[profile.echo.env]
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

[profile.broken.env]
TOKEN = "inline-secret"

[profile.ok]
argv = ["/bin/true"]
"#,
        );
        let got = resolve_profile(&t, "ok", |_| None).unwrap();
        assert_eq!(got.argv, vec!["/bin/true"]);
    }
}
