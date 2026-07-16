//! 变换段池（D4）——手写特例：段（16 槽）整借整还、无逐槽 generation（段的生命被弹句柄
//! 的 generation 罩住）。分配 = 空闲位图最低空位（I4）。alloc **不清零**：写满 16 槽
//! （拷贝 + 尾零）是 `create_bullet_with_xform` 的义务（复用槽写满哲学）。
//! op 编号已冻结（spec 2026-07-16；改动 = 过评审 + bump engine_ver）。

use crate::checksum::{Checksum, Fnv1a64};

pub(crate) const SEG_CAP: usize = 2048;
pub(crate) const SLOTS_PER_SEG: usize = 16;
/// 哑弹哨兵（与 `bullets.transform_head` 既用值一致）。
pub(crate) const XFORM_NONE: u16 = 0xFFFF;

// ── op 编号（冻结）─────────────────────────────────────────────
pub const OP_END: u8 = 0;
pub const OP_SET_SPEED: u8 = 1;
pub const OP_ADD_SPEED: u8 = 2;
pub const OP_SET_ANGLE: u8 = 3;
pub const OP_TURN: u8 = 4;
pub const OP_AIM_PLAYER: u8 = 5;
pub const OP_SET_SPRITE: u8 = 6;
pub const OP_SET_LIFE: u8 = 7;
pub const OP_SET_ANG_VEL: u8 = 8;
pub const OP_SET_ACCEL: u8 = 9;
pub const OP_SET_GRAVITY: u8 = 10;
pub const OP_STOP_FX: u8 = 11;
pub const OP_LOOP: u8 = 12;
/// 本刀已实现的最大 op 号；> 此值（或 13..=17 预留区）= 未知 op → P4-b 终止序列。
pub(crate) const OP_MAX_IMPLEMENTED: u8 = OP_LOOP;
/// 扩展槽数（游标步进 = 1 + ARITY[op]）。本刀全 0；11b 的 STEP_* 为 1。
pub(crate) const ARITY: [u8; 13] = [0; 13];

/// 一个变换槽（12 B，相对 wait 制：发射本 op 后等 wait 帧再执行下一槽）。
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct XformSlot {
    pub wait: u16,
    pub op: u8,
    pub _pad: u8,
    pub args: [i32; 2],
}

/// 段池本体。inline 数组住 WorldBody（I7 无堆容器；World 整体 alloc_zeroed）。全零 = 合法空池。
#[repr(C)]
pub struct XformSegPool {
    pub(crate) occupied: [u64; SEG_CAP / 64],
    pub(crate) slots: [XformSlot; SEG_CAP * SLOTS_PER_SEG],
}

impl XformSegPool {
    /// 最低空段（I4 确定性分配）。**不清零槽内容**——写满是调用方义务。
    ///
    /// 生产消费方：`create_bullet_with_xform`（先段后弹的"先段"半边）。
    pub(crate) fn alloc(&mut self) -> Option<u16> {
        for (w, word) in self.occupied.iter_mut().enumerate() {
            if *word != u64::MAX {
                let bit = (!*word).trailing_zeros() as usize;
                let seg = w * 64 + bit;
                if seg >= SEG_CAP {
                    return None; // 末字幽灵位（SEG_CAP 恰为 64 倍数时不可达，防御）
                }
                *word |= 1 << bit;
                return Some(seg as u16);
            }
        }
        None
    }

    /// 还段。双 free / 越界属引擎 bug（P4-c debug 断言）；release 幂等清位。
    ///
    /// 生产消费方：`create_bullet_with_xform` 的弹池满回滚半边 + `cleanup` 的弹回收还段。
    pub(crate) fn free(&mut self, seg: u16) {
        let s = seg as usize;
        debug_assert!(s < SEG_CAP, "还段越界（引擎 bug）");
        if s >= SEG_CAP {
            return;
        }
        debug_assert!(
            self.occupied[s / 64] & (1 << (s % 64)) != 0,
            "双重还段（引擎 bug）"
        );
        self.occupied[s / 64] &= !(1 << (s % 64));
    }

    /// 生产消费方：`run_transforms` 的游标执行器只读遍历。
    pub(crate) fn seg_slots(&self, seg: u16) -> &[XformSlot] {
        let base = seg as usize * SLOTS_PER_SEG;
        &self.slots[base..base + SLOTS_PER_SEG]
    }

    /// 生产消费方：`create_bullet_with_xform` 写满 16 槽（拷贝 + 尾零）。
    pub(crate) fn seg_slots_mut(&mut self, seg: u16) -> &mut [XformSlot] {
        let base = seg as usize * SLOTS_PER_SEG;
        &mut self.slots[base..base + SLOTS_PER_SEG]
    }

    /// 快照拷贝（copy_into 家族）。
    pub(crate) fn copy_into(&self, dst: &mut XformSegPool) {
        dst.occupied.copy_from_slice(&self.occupied);
        dst.slots.copy_from_slice(&self.slots);
    }
}

/// P6：全量入校验和——占用位图 + **全部**槽（哈希全槽不看占用），小端、声明序。
impl Checksum for XformSegPool {
    fn hash_into(&self, h: &mut Fnv1a64) {
        for w in &self.occupied {
            w.hash_into(h);
        }
        for s in &self.slots {
            s.wait.hash_into(h);
            s.op.hash_into(h);
            s._pad.hash_into(h);
            s.args[0].hash_into(h);
            s.args[1].hash_into(h);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checksum::Checksum;

    #[test]
    fn alloc_is_lowest_free_and_deterministic() {
        let mut w = crate::step::World::new(1);
        let a = w.body.xforms.alloc().unwrap();
        let b = w.body.xforms.alloc().unwrap();
        assert_eq!((a, b), (0, 1)); // 最低空位升序
        w.body.xforms.free(a);
        assert_eq!(w.body.xforms.alloc().unwrap(), 0); // 还段后复用最低位
    }

    #[test]
    fn pool_full_returns_none() {
        let mut w = crate::step::World::new(1);
        for k in 0..SEG_CAP {
            assert!(w.body.xforms.alloc().is_some(), "第 {k} 段应成功");
        }
        assert!(w.body.xforms.alloc().is_none()); // 2048 段耗尽
    }

    #[test]
    fn checksum_sensitive_to_any_slot_byte_and_occupancy() {
        let mut w = crate::step::World::new(1);
        let base = w.body.xforms.checksum();
        let s = w.body.xforms.alloc().unwrap();
        let occ = w.body.xforms.checksum();
        assert_ne!(occ, base, "占用位图入校验和");
        w.body.xforms.seg_slots_mut(s)[7].args[1] = 1; // 未占用语义无关——哈希全槽
        let after_slot = w.body.xforms.checksum();
        assert_ne!(after_slot, occ, "任意槽字节入校验和");
        // 段 5 全程未分配（本测试从未 alloc 到它）——"只哈希占用段"的变异体在此维度无从分辨。
        w.body.xforms.seg_slots_mut(5)[0].args[0] = 99;
        assert_ne!(
            w.body.xforms.checksum(),
            after_slot,
            "未分配段的槽也必须入哈希（P6 哈希全槽不看占用）"
        );
    }

    #[test]
    fn zeroed_pool_is_deterministic() {
        // 两个零构造 World 的段池指纹相同（alloc_zeroed 合法性的一角）
        assert_eq!(
            crate::step::World::new(1).body.xforms.checksum(),
            crate::step::World::new(2).body.xforms.checksum()
        );
    }
}
