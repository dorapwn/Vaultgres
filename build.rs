use std::process::Command;

fn main() {
    // Resolution order for the GIT_HASH rustc-env:
    //   1. GIT_HASH env var (set by Dockerfile `ARG GIT_HASH` so the
    //      hash is deterministic even when .git/ is stripped from the
    //      build context — see https://github.com/neoalienson/Vaultgres/issues/39).
    //   2. `git rev-parse --short HEAD` from the local checkout (works for
    //      developers and any build that has .git/ present).
    //   3. "unknown" — only if both above fail.
    //
    // Emitting `cargo:rerun-if-env-changed=GIT_HASH` makes cargo rebuild
    // when the build-arg changes (Docker), so a new `--build-arg GIT_HASH=...`
    // produces a binary that reflects it without a source change.
    let from_env =
        std::env::var("GIT_HASH").ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());

    let from_git = || -> String {
        Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| {
                let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if s.is_empty() { None } else { Some(s) }
            })
            .unwrap_or_else(|| "unknown".to_string())
    };

    let git_hash = from_env.unwrap_or_else(from_git);

    println!("cargo:rustc-env=GIT_HASH={}", git_hash);
    println!("cargo:rerun-if-env-changed=GIT_HASH");
    println!("cargo:rerun-if-changed=.git/HEAD");
}
