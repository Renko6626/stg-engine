//! # stg-derive —— 编译期过程宏
//!
//! Rust 强制 proc-macro 单独成 crate（`[lib] proc-macro = true`），故它虽是 stg-core 的
//! 编译期工具、却物理上独立于内核三 crate。
//!
//! ## M0 将落地：`#[derive(Checksum)]`
//!
//! stg-world-design.md **D11** 的"防漏单一真相源"：从结构体字段自动生成
//!
//! - `checksum() -> u64`：按字段声明序合并（联机随包 / CI 断言用）；
//! - `checksum_report()`（debug）：逐字段哈希清单，desync 时直接定位分歧字段；
//! - `#[checksum(skip = "理由")]`：跳过某字段必须给出理由字符串（宏强制）。
//!
//! 目的：新增字段自动纳入校验和，杜绝"加了字段忘了哈希 → 静默漏检 desync"。
//!
//! 当前为脚手架占位，尚未导出任何宏。
