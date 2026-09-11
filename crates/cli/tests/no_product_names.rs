//! F4: domain source contains no product or tracker names.
//! Lives here so the scan can read files without violating domain clippy (F3).

use std::fs;
use std::path::Path;

const FORBIDDEN: &[&str] = &[
    "jira",
    "yougile",
    "linear",
    "asana",
    "trello",
    "youtrack",
    "notion",
    "monday.com",
    "github",
    "gitlab",
    "bitbucket",
    "cursor",
    "claude",
    "codex",
    "openai",
    "anthropic",
    "copilot",
];

#[test]
fn domain_src_has_no_product_or_tracker_names() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../domain/src");
    let mut hits = Vec::new();
    visit(&root, &mut hits);
    assert!(
        hits.is_empty(),
        "workengine-domain must not name products or trackers:\n{}",
        hits.join("\n")
    );
}

#[test]
fn async_runtime_is_not_in_domain_or_application() {
    for manifest in [
        include_str!("../../domain/Cargo.toml"),
        include_str!("../../application/Cargo.toml"),
    ] {
        assert!(
            !manifest.contains("tokio"),
            "Tokio belongs only to the HTTP adapter, never domain or application"
        );
    }
}

fn visit(dir: &Path, hits: &mut Vec<String>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            visit(&path, hits);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap().to_ascii_lowercase();
        for name in FORBIDDEN {
            if contains_token(&text, name) {
                hits.push(format!("{}: {name}", path.display()));
            }
        }
    }
}

fn contains_token(haystack: &str, needle: &str) -> bool {
    let bytes = haystack.as_bytes();
    let mut from = 0;
    while let Some(rel) = haystack[from..].find(needle) {
        let start = from + rel;
        let end = start + needle.len();
        let before_ok = start == 0 || !is_token_char(bytes[start - 1]);
        let after_ok = end == bytes.len() || !is_token_char(bytes[end]);
        if before_ok && after_ok {
            return true;
        }
        from = start + 1;
    }
    false
}

fn is_token_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}
