//! ecl/image.rs —— 只读脚本镜像（`EclImage`）：字节码 + 入口表 + 内容哈希（占位）。
//!
//! 静态数据，**不进 `World`**（I7：World 内无引用/指针）；`&EclImage` 参数穿线与
//! `&WorldTables` 同款——`step`/`step_with_director` 按帧传参消费，两机同镜像由二进制
//! 同一性/哈希保证，不随快照回滚。无脚本场景传 [`EclImage::empty`]（零任务即零成本，
//! T2 金向量一号逐位不变门）。

/// 脚本镜像：`code` 是全部子程序共享的扁平字流（`Task.pc` 是其**绝对**字索引，不是相对
/// 某个 sub 入口的偏移——`JMP`/`CALL`/`RET` 的目标字面量与 `pc` 同一坐标系）；
/// `subs[i]` = 脚本 i 的入口字索引；`content_hash` 占位（同 `WorldTables` 惯例，编译器
/// 落地时填真哈希，供回放头/联机握手核对两机镜像一致，本刀恒 0）。
pub struct EclImage {
    pub code: Vec<u32>,
    pub subs: Vec<u32>,
    pub content_hash: u64,
}

impl EclImage {
    /// 零脚本镜像。`Vec::new()` 是 const fn 且无堆分配（长度/容量 0，指针悬空不解引用）——
    /// 空镜像构造与消费都是零成本，金向量一号场景用它穿线即可验证"空镜像零行为"。
    pub const fn empty() -> EclImage {
        EclImage {
            code: Vec::new(),
            subs: Vec::new(),
            content_hash: 0,
        }
    }

    /// 脚本 id → 入口字索引；越界（未知脚本号）→ `None`（`OP_SPAWN`/`World::spawn_task`
    /// 用此判定坏号，确定性报错/计数，不 panic）。
    pub fn entry(&self, script: u16) -> Option<u32> {
        self.subs.get(script as usize).copied()
    }
}

impl Default for EclImage {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_has_no_entries() {
        let img = EclImage::empty();
        assert!(img.code.is_empty());
        assert!(img.subs.is_empty());
        assert_eq!(img.entry(0), None);
    }

    #[test]
    fn entry_resolves_in_range_and_none_out_of_range() {
        let img = EclImage {
            code: vec![0, 1, 2, 3],
            subs: vec![0, 2],
            content_hash: 0,
        };
        assert_eq!(img.entry(0), Some(0));
        assert_eq!(img.entry(1), Some(2));
        assert_eq!(img.entry(2), None, "越界脚本号 → None");
    }
}
