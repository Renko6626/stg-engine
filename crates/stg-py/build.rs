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
    // 工作树有未提交改动（含未跟踪文件以外的修改）时标 `-dirty`：本地在提交前构建的 wheel
    // 不会冒充干净提交的出处。CI 从 tag 检出构建时工作树干净，不带后缀。
    let dirty = sha != "unknown"
        && Command::new("git")
            .args(["status", "--porcelain", "--untracked-files=no"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .is_some_and(|o| !o.stdout.is_empty());
    let sha = if dirty { format!("{sha}-dirty") } else { sha };
    println!("cargo:rerun-if-changed=../../.git/index");
    println!("cargo:rustc-env=STG_GIT_SHA={sha}");
}
