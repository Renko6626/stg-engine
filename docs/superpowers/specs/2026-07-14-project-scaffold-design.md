# 项目脚手架与全流程工作流 —— 设计 spec

**日期**：2026-07-14
**范围**：为 stg_engine 建立 Cargo workspace 骨架、git/GitHub 结构、跨平台确定性 CI，
并铺好后续全流程开发的工作流。**不含**任何引擎逻辑实现（数学核/池/World 归 M0，走独立
spec→plan→TDD 循环）。
**上游**：`design_doc.md` v0.3、`stg-world-design.md` v1.0（本 spec 只做落地，不改设计决策）。

## 已定决策（本次 brainstorm 拍板）

| # | 决策 | 选择 | 理由 |
|---|---|---|---|
| 1 | 脚手架范围 | 仅 Phase 1 三 crate + `stg-derive` | YAGNI；godot/py/net 到各自 milestone 再加（§1.3） |
| 2 | 跨平台 aarch64 验证 | GitHub Actions 双架构 CI | DoD 要求 x86_64/aarch64 逐帧一致；开发机是 Windows x86_64，aarch64 靠 CI |
| 3 | git 托管 | git init + GitHub 私有仓（Renko6626/stg-engine） | 承载 Actions CI |
| 4 | proc-macro 独立 crate | 保留 `stg-derive` | Rust 强制 proc-macro 独立成 crate，承载 `#[derive(Checksum)]`（D11） |
| 5 | 烘焙表生成器归属 | `stg-harness`（断层线以上） | 生成用 f64，f64 禁于 core；core 只 `include_bytes!` commit 字节 |
| 6 | 校验和算法 | vendored FNV-1a 64（`stg_core::checksum`） | D11：算法字节冻结，绝不走外部依赖 |
| 7 | 工具链 | 钉死 `1.92.0`，edition 2024，resolver 3 | 可复现；排除工具链漂移变量 |
| 8 | Cargo.lock | 提交 | 确定性须锁依赖版本 |

## Workspace 结构

```
Cargo.toml  rust-toolchain.toml  rustfmt.toml  .gitignore  Cargo.lock
CLAUDE.md  README.md  design_doc.md  stg-world-design.md
docs/superpowers/{specs,plans}/
.github/workflows/ci.yml
crates/{stg-core, stg-derive, stg-ecl-compiler, stg-harness}/
```

三个结构判断：`stg-derive` proc-macro 独立成 crate（决策 4）；烘焙表生成器住 harness
（决策 5）；两份设计文档留根目录由 CLAUDE.md 引用。

## 确定性纪律焊进脚手架（从第一天）

- `stg-core/Cargo.toml` 零依赖 + 注释确定性防火墙；CI `cargo tree -p stg-core` 断言无
  `rand/getrandom/chrono/time/libm/godot/gdext`。
- `[profile.dev] overflow-checks = true`（P4-c 溢出即 panic）；release wrapping。
- CI 用 **debug** 跑金向量：既覆盖帧内断言/PhaseGuard，又做跨平台校验和对拍。

## CI 矩阵（`.github/workflows/ci.yml`）

- `lint`：fmt --check + clippy -D warnings + 依赖防火墙。
- `vector`（矩阵）：`windows-latest` / `ubuntu-24.04` / `ubuntu-24.04-arm` → build(debug) →
  test → `verify-tables` → `golden --out checksums.txt` → 上传工件。
- `determinism-gate`：下载三份 checksum 断言逐字节全等，desync 当场红。
- **私有仓风险**：arm64 托管 runner 可能计费/需付费计划；yml 文末附 QEMU 回退（在 x86_64
  runner 上 `docker run --platform linux/arm64`）。

## 端到端先行

`stg-harness golden` 即使世界为空也先算确定性 checksum，让整条 DoD 流水线（三平台构建→产出
工件→对拍）在有复杂逻辑之前就被证明可用。M0 起把占位演化替换为真实整数世界 `step()`。

## 全流程工作流

每个 milestone 走 superpowers 环（brainstorming→writing-plans→executing-plans→code-review→
finishing-branch），TDD 强制，金向量即回归向量，trunk-based 分支。`CLAUDE.md` 是每 session 第一读物。

## 明确的非目标（本 spec）

- 不实现任何数学核/池/World/step/VM 逻辑（M0/M1）。
- 不建 stg-godot / stg-py / stg-net（M2/M4/M5）。
- 不烘焙真实 sin/cos/easing 表（M0）。

## 验收

- `cargo build/test --workspace`、`fmt --check`、`clippy -D warnings` 本地全绿。
- `stg-harness golden` / `verify-tables` 可跑。
- git init + 首次提交（含 Cargo.lock）+ 推送 GitHub 私有仓，CI 首跑触发。
- 下一步：writing-plans 产出 M0 实施计划。
