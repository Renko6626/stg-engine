//! 构建期注入 git 短 SHA（`STG_GIT_SHA`），供 `build_info()` 报告 wheel 出处。
//! 无 git / 非仓库时退化为 `"unknown"`（CI 从 tarball 构建等场景）。
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    let sha = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=STG_GIT_SHA={sha}");
}
