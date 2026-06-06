use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=SOCAI_BUILD_SHA");
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
    register_git_rerun_paths();

    if let Some(sha) = env_sha("SOCAI_BUILD_SHA")
        .or_else(|| env_sha("GITHUB_SHA"))
        .or_else(git_sha)
    {
        println!("cargo:rustc-env=SOCAI_BUILD_SHA={sha}");
    }
}

fn env_sha(name: &str) -> Option<String> {
    env::var(name).ok().and_then(normalize_sha)
}

fn git_sha() -> Option<String> {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").ok()?;
    let output = Command::new("git")
        .args(["-C", &manifest_dir, "rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .and_then(normalize_sha)
}

fn normalize_sha(raw: String) -> Option<String> {
    let value = raw.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("unknown") {
        None
    } else {
        Some(value.to_string())
    }
}

fn register_git_rerun_paths() {
    let Some(manifest_dir) = env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from) else {
        return;
    };
    let Some(git_dir) = git_path(&manifest_dir, "--git-dir") else {
        return;
    };
    let common_dir = git_path(&manifest_dir, "--git-common-dir").unwrap_or_else(|| git_dir.clone());

    let git_dir = absolute_path(&manifest_dir, &git_dir);
    let common_dir = absolute_path(&manifest_dir, &common_dir);
    let head = git_dir.join("HEAD");
    println!("cargo:rerun-if-changed={}", head.display());
    println!(
        "cargo:rerun-if-changed={}",
        common_dir.join("packed-refs").display()
    );

    let Ok(head_contents) = std::fs::read_to_string(&head) else {
        return;
    };
    let Some(reference) = head_contents.strip_prefix("ref:").map(str::trim) else {
        return;
    };
    println!(
        "cargo:rerun-if-changed={}",
        common_dir.join(reference).display()
    );
}

fn git_path(manifest_dir: &Path, flag: &str) -> Option<PathBuf> {
    let manifest_dir = manifest_dir.to_str()?;
    let output = Command::new("git")
        .args(["-C", manifest_dir, "rev-parse", flag])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let raw = String::from_utf8(output.stdout).ok()?;
    Some(PathBuf::from(raw.trim()))
}

fn absolute_path(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}
