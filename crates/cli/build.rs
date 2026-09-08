fn main() {
    let sha = git_sha();
    println!("cargo:rustc-env=WORKENGINE_GIT_SHA={sha}");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    if let Ok(head) = std::fs::read_to_string("../../.git/HEAD")
        && let Some(rel) = head.strip_prefix("ref: ")
    {
        println!("cargo:rerun-if-changed=../../.git/{}", rel.trim());
    }
}

fn git_sha() -> String {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output();
    match output {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim().to_owned(),
        _ => String::new(),
    }
}
