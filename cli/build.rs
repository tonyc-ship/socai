use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

// Tracked paths whose contents are compiled into the `socai` binary (and the
// daemon it spawns). Uncommitted changes here must invalidate the build
// identity so a rebuilt CLI never treats a stale daemon at the same HEAD as
// compatible.
const BUILD_IDENTITY_PATHS: &[&str] = &["cli", "core", "Cargo.toml", "Cargo.lock"];

fn main() {
    println!("cargo:rerun-if-env-changed=SOCAI_BUILD_SHA");
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
    register_git_rerun_paths();
    register_source_rerun_paths();

    if let Some(sha) = env_sha("SOCAI_BUILD_SHA")
        .or_else(|| env_sha("GITHUB_SHA"))
        .or_else(git_build_identity)
    {
        println!("cargo:rustc-env=SOCAI_BUILD_SHA={sha}");
    }
}

fn env_sha(name: &str) -> Option<String> {
    env::var(name).ok().and_then(normalize_sha)
}

// HEAD SHA, plus a deterministic content marker when the relevant worktree is
// dirty so two different uncommitted states at the same HEAD never collide.
// Explicit `SOCAI_BUILD_SHA`/`GITHUB_SHA` (clean release builds) bypass this and
// stay stable.
fn git_build_identity() -> Option<String> {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").ok()?;
    let head = git_head_sha(&manifest_dir)?;
    match git_dirty_marker(&manifest_dir) {
        Some(marker) => Some(format!("{head}-dirty.{marker}")),
        None => Some(head),
    }
}

fn git_head_sha(manifest_dir: &str) -> Option<String> {
    let output = Command::new("git")
        .args(["-C", manifest_dir, "rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .and_then(normalize_sha)
}

// Hash of the tracked diff against HEAD for the paths compiled into the binary.
// Returns `None` when the relevant worktree is clean (or git is unavailable),
// which keeps clean builds reporting the bare HEAD SHA.
fn git_dirty_marker(manifest_dir: &str) -> Option<String> {
    let mut args = vec![
        "-C".to_string(),
        manifest_dir.to_string(),
        "diff".to_string(),
        "HEAD".to_string(),
        "--".to_string(),
    ];
    args.extend(BUILD_IDENTITY_PATHS.iter().map(|spec| repo_pathspec(spec)));
    let output = Command::new("git").args(&args).output().ok()?;
    if !output.status.success() || output.stdout.is_empty() {
        return None;
    }
    Some(fnv1a_hex(&output.stdout))
}

// `:/` anchors the pathspec at the worktree root regardless of `-C` cwd.
fn repo_pathspec(relative: &str) -> String {
    format!(":/{relative}")
}

fn normalize_sha(raw: String) -> Option<String> {
    let value = raw.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("unknown") {
        None
    } else {
        Some(value.to_string())
    }
}

// Deterministic 64-bit FNV-1a digest as zero-padded hex. Avoids a
// build-dependency on a hashing crate while staying stable across runs/hosts.
fn fnv1a_hex(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
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

// Re-run the build script whenever the sources baked into the build identity
// change, so the dirty marker tracks normal `cargo run`/`cargo build` edits
// instead of going stale until the next commit/checkout.
fn register_source_rerun_paths() {
    let Some(manifest_dir) = env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from) else {
        return;
    };
    // `cli` is the manifest dir; the rest hang off the workspace root.
    emit_rerun_if_changed(&manifest_dir);
    let Some(repo_root) = manifest_dir.parent() else {
        return;
    };
    for relative in ["core", "Cargo.toml", "Cargo.lock"] {
        emit_rerun_if_changed(&repo_root.join(relative));
    }
}

fn emit_rerun_if_changed(path: &Path) {
    // Skip missing paths: an absent `rerun-if-changed` target forces the build
    // script to re-run on every invocation.
    if path.exists() {
        println!("cargo:rerun-if-changed={}", path.display());
    }
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
