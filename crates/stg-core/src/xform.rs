//! 变换段池（D4）——手写特例：段（16 槽）整借整还、无逐槽 generation（段的生命被弹句柄
//! 的 generation 罩住）。分配 = 空闲位图最低空位（I4）。alloc **不清零**：写满 16 槽
//! （拷贝 + 尾零）是 `create_bullet_with_xform` 的义务（复用槽写满哲学）。
//! op 编号已冻结（spec 2026-07-16；改动 = 过评审 + bump engine_ver）。

use crate::checksum::{Checksum, Fnv1a64};
use crate::save::{LoadError, SaveBytes, SaveReader};

pub(crate) const SEG_CAP: usize = 2048;
pub(crate) const SLOTS_PER_SEG: usize = 16;
/// 哑弹哨兵（与 `bullets.transform_head` 既用值一致）。
pub(crate) const XFORM_NONE: u16 = 0xFFFF;

// ── op 编号（冻结；族号制 v2，2026-07-16 重排拍板——回放格式出生前的免费窗口）──
// 十位 = 族号：1x 速率 / 2x 角度 / 3x 状态 / 4x 连续效果 / 5x 控制·事件 / 6x 派生 /
// 7x 预留（笛卡尔族，若将来立项）。族内留空隙：新 op 落族内、永不乱序追加。
pub const OP_END: u8 = 0;
pub const OP_SET_SPEED: u8 = 10;
pub const OP_ADD_SPEED: u8 = 11;
pub const OP_STEP_SPEED: u8 = 12; // M0-11b
pub const OP_SET_ANGLE: u8 = 20;
pub const OP_TURN: u8 = 21;
pub const OP_AIM_PLAYER: u8 = 22;
pub const OP_STEP_ANGLE: u8 = 23; // M0-11b
pub const OP_SET_SPRITE: u8 = 30;
pub const OP_SET_LIFE: u8 = 31;
/// 只换形状、保住颜色位（颜色轴刀 T7）。`args[0]` = 形状基址，`args[1]` = 色轴宽度
/// （**由编译器从绑定表写入**——引擎不知道"颜色"是什么，只是拿两个操作数做取模）。
pub const OP_SET_SHAPE: u8 = 32;
/// 只换颜色、保住形状位（同上，`args[0]` = 色号，`args[1]` = 色轴宽度）。
pub const OP_SET_COLOR: u8 = 33;
pub const OP_SET_ANG_VEL: u8 = 40;
pub const OP_SET_ACCEL: u8 = 41;
pub const OP_SET_GRAVITY: u8 = 42;
pub const OP_STOP_FX: u8 = 43;
pub const OP_LOOP: u8 = 50;
pub const OP_WAIT_SIGNAL: u8 = 51; // M0-11b
pub const OP_BOUNCE_ARM: u8 = 52; // M0-11b
pub const OP_SPAWN_PATTERN: u8 = 60; // 预留（随图样描述符表另立一刀）

/// 本刀（M0-11a）已实现的 op 集——**按表查而非比大小**（族号制下编号非连续）。
/// 创建期用它拒收未实现 op；11b 落地时把对应 op 加进来即可。
pub(crate) const fn op_implemented(op: u8) -> bool {
    matches!(
        op,
        OP_END
            | OP_SET_SPEED
            | OP_ADD_SPEED
            | OP_SET_ANGLE
            | OP_TURN
            | OP_AIM_PLAYER
            | OP_SET_SPRITE
            | OP_SET_LIFE
            | OP_SET_SHAPE
            | OP_SET_COLOR
            | OP_SET_ANG_VEL
            | OP_SET_ACCEL
            | OP_SET_GRAVITY
            | OP_STOP_FX
            | OP_LOOP
            | OP_WAIT_SIGNAL
            | OP_STEP_SPEED
            | OP_STEP_ANGLE
            | OP_BOUNCE_ARM
    )
}

/// 扩展槽数（游标步进 = 1 + ARITY[op]）。**全 u8 域**——任意 op 值可安全索引，
/// `fire_op` 的未知臂不再是不越界的唯一屏障（终审 M-A#4 拆除）。
/// STEP 族预置 1（11b 实现时游标步进直接正确）；其余 0。
pub(crate) const ARITY: [u8; 256] = {
    let mut a = [0u8; 256];
    a[OP_STEP_SPEED as usize] = 1;
    a[OP_STEP_ANGLE as usize] = 1;
    a
};

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

/// 手写 SaveBytes（镜像上面手写 Checksum 的字段序，同一份"占用位图 + 全部槽、无 skip"
/// 纪律——`define_pool!` 池的 SaveBytes 是宏生成，本池手写特例故手写这半边）。
impl SaveBytes for XformSegPool {
    fn write_bytes(&self, out: &mut Vec<u8>) {
        for w in &self.occupied {
            w.write_bytes(out);
        }
        for s in &self.slots {
            s.wait.write_bytes(out);
            s.op.write_bytes(out);
            s._pad.write_bytes(out);
            s.args[0].write_bytes(out);
            s.args[1].write_bytes(out);
        }
    }
    fn read_bytes(&mut self, r: &mut SaveReader<'_>) -> Result<(), LoadError> {
        for w in &mut self.occupied {
            w.read_bytes(r)?;
        }
        for s in &mut self.slots {
            s.wait.read_bytes(r)?;
            s.op.read_bytes(r)?;
            s._pad.read_bytes(r)?;
            s.args[0].read_bytes(r)?;
            s.args[1].read_bytes(r)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checksum::Checksum;

    /// 编号冻结钉死（族号制 v2：十位 = 族号——1x 速率 / 2x 角度 / 3x 状态 / 4x 连续 /
    /// 5x 控制 / 6x 派生）。重排 = 契约变更，本测试逼改动者有意识确认。
    #[test]
    fn op_numbering_frozen_v2() {
        assert_eq!(OP_END, 0);
        assert_eq!((OP_SET_SPEED, OP_ADD_SPEED, OP_STEP_SPEED), (10, 11, 12));
        assert_eq!(
            (OP_SET_ANGLE, OP_TURN, OP_AIM_PLAYER, OP_STEP_ANGLE),
            (20, 21, 22, 23)
        );
        assert_eq!((OP_SET_SPRITE, OP_SET_LIFE), (30, 31));
        assert_eq!((OP_SET_SHAPE, OP_SET_COLOR), (32, 33));
        assert_eq!(
            (OP_SET_ANG_VEL, OP_SET_ACCEL, OP_SET_GRAVITY, OP_STOP_FX),
            (40, 41, 42, 43)
        );
        assert_eq!((OP_LOOP, OP_WAIT_SIGNAL, OP_BOUNCE_ARM), (50, 51, 52));
        assert_eq!(OP_SPAWN_PATTERN, 60);
    }

    /// 有效性按表查而非比大小：11a+11b(WAIT_SIGNAL/STEP_SPEED/STEP_ANGLE/BOUNCE_ARM) +
    /// 颜色轴刀 T7(OP_SET_SHAPE/OP_SET_COLOR) 已实现集恰为 19 个；未实现/垃圾值一律 false；
    /// ARITY 全 u8 域可索引（终审 M-A#4 的脆弱性就此拆除）。
    #[test]
    fn op_implemented_table_and_arity_full_domain() {
        let implemented = [
            OP_END,
            OP_SET_SPEED,
            OP_ADD_SPEED,
            OP_STEP_SPEED,
            OP_SET_ANGLE,
            OP_TURN,
            OP_AIM_PLAYER,
            OP_STEP_ANGLE,
            OP_SET_SPRITE,
            OP_SET_LIFE,
            OP_SET_SHAPE,
            OP_SET_COLOR,
            OP_SET_ANG_VEL,
            OP_SET_ACCEL,
            OP_SET_GRAVITY,
            OP_STOP_FX,
            OP_LOOP,
            OP_WAIT_SIGNAL,
            OP_BOUNCE_ARM,
        ];
        for op in 0..=255u8 {
            assert_eq!(
                op_implemented(op),
                implemented.contains(&op),
                "op {op} 的有效性判定错误"
            );
        }
        // ARITY 任意 u8 可索引；STEP 族预置 1，其余 0
        assert_eq!(ARITY[OP_STEP_SPEED as usize], 1);
        assert_eq!(ARITY[OP_STEP_ANGLE as usize], 1);
        assert_eq!(ARITY[255], 0);
        assert_eq!(ARITY[OP_LOOP as usize], 0);
    }

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
