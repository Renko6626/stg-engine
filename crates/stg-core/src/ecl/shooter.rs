//! ShooterSlot（预存发射参数集）——ECL 层的每任务发射器状态（shooter 刀 2026-07-31）。
//!
//! 参照 ZUN ECL 的 `et*` 族弹幕管理器（600-641）：`sh_reset` 重置编号槽 → 一堆以 `id` 打头的
//! setter 逐项配 → `sh_fire(id)` 开火。改一个字段再开一次火就是下一波。
//!
//! **为什么住 ECL 层而不是 `WorldBody`**：P1 明写 world 不 import 任何 ECL 类型、不知道
//! "任务"存在。shooter 按任务键 ⇒ 天然是 ECL 层的东西。存储挂在 `TaskPool` 的并行数组上，
//! 顺带白捡"槽复用时的重置有现成挂点"（`TaskPool::spawn`）。
//!
//! **为什么叫 `ShooterSlot` 而不是 `Shooter`**：`crate::tables::Shooter` 已经占了后者，而且
//! 它是个**几乎同概念**的东西（自机 shottype 的子发射器描述：`interval/delay/dx/dy/angle/
//! speed/...`）——两者都是"存起来的发射器参数"，只差"静态表数据 vs 每任务可变状态"。同概念
//! 撞名比无关撞名难受得多，故取 `-Slot` 后缀：既落进本仓 `XformSlot`/`SpellSlot`/`BossUiSlot`
//! 的家规，又自带"可变的槽"语义。模块名与 `TaskPool.shooters` 字段名保持复数、不带后缀。
//!
//! **字段顺序按 4 字节对齐紧排**（`Fx` 在前、`u16` 居中、`u8` 收尾），`repr(C)` 下恰好 44 B。
//! 改字段顺序会改槽宽 → 改快照尺寸 → 改存档格式，动前先看 `size_of` 那条测试。

use crate::math::{Angle, Fx};

/// 每任务的 shooter 槽数（spec D-1）。脚本用 `id ∈ 0..SHOOTERS_PER_TASK`。
/// 4 是"一个 boss 典型并发 2-3 种弹（主环/点射/收尾）再留一格"的取值；
/// 容量账：44 B × 4 × 256 = 45056 B，见 spec §4.2。
pub const SHOOTERS_PER_TASK: usize = 4;

/// `flags` 位：`angle0` 是**相对自机方向的偏移**（而非绝对方向）。
pub const SH_AIMED: u8 = 1 << 0;
/// `flags` 位：`n_angle` 颗**自动均分整周**，`angle_step` 转义成**逐层**偏移；
/// 否则 fan（`angle_step` 逐弹、且**以基准方向为中心**对称展开）。
pub const SH_RING: u8 = 1 << 1;
/// `flags` 位：`off_x/off_y` 是**绝对**坐标（而非相对 owner）。
pub const SH_ABS_OFFSET: u8 = 1 << 2;

/// `task_script` 的"无挂弹任务"哨兵——与既有 `crate::xform::XFORM_NONE` 同惯例。
pub const SH_NO_TASK: u16 = 0xFFFF;

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, crate::checksum::Checksum, crate::save::SaveBytes)]
pub struct ShooterSlot {
    pub off_x: Fx,
    pub off_y: Fx,
    /// 极坐标偏移的半径；与 `off_x/off_y` **永远叠加**，不存在覆盖关系（ZUN 626 明写 stacks）。
    pub polar_r: Fx,
    /// 出生后沿**各自角度**推出去的距离（ZUN 627）——逐颗方向不同，不是整体平移。
    pub dist: Fx,
    pub speed0: Fx,
    pub speed_step: Fx,
    /// 形 × color_stride + 色，折叠值（同 `fire`/`batch` 的颜色轴糖）。
    pub appearance: u16,
    pub polar_ang: Angle,
    pub angle0: Angle,
    pub angle_step: Angle,
    /// 指向**本任务 `locals`** 的 xform 区间起点。
    pub xform_off: u16,
    /// `SH_NO_TASK` = 不挂。
    pub task_script: u16,
    /// 开火后要发的通道 B 请求 id；`0` = 不发（ZUN 608 的 sound1 归并进通道 B，spec §8）。
    pub on_fire_req: u16,
    pub n_angle: u8,
    pub n_speed: u8,
    /// `0` = 无 xform。
    pub xform_cnt: u8,
    pub flags: u8,
}

impl Default for ShooterSlot {
    fn default() -> Self {
        ShooterSlot {
            off_x: Fx::ZERO,
            off_y: Fx::ZERO,
            polar_r: Fx::ZERO,
            dist: Fx::ZERO,
            speed0: Fx::ZERO,
            speed_step: Fx::ZERO,
            appearance: 0,
            polar_ang: Angle::ZERO,
            angle0: Angle::ZERO,
            angle_step: Angle::ZERO,
            xform_off: 0,
            task_script: SH_NO_TASK,
            on_fire_req: 0,
            // **1×1 而不是 0**：刚重置的 shooter 开火发一颗弹,是个有意义的退化,
            // 不是"什么也不发"这种要 debug 半天的静默。
            n_angle: 1,
            n_speed: 1,
            xform_cnt: 0,
            flags: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 槽宽是容量账的一部分（spec §4.2：44 B × 4 × 256 = 45056 B）。变了就要重算预算表。
    #[test]
    fn shooter_is_44_bytes() {
        assert_eq!(core::mem::size_of::<ShooterSlot>(), 44);
    }

    /// 默认值：`n_angle`/`n_speed` 是 **1×1** 而不是 0——刚重置的 shooter 开火发**一颗**弹，
    /// 是个有意义的退化，不是"什么也不发"这种要 debug 半天的静默（spec §5）。
    #[test]
    fn shooter_default_fires_exactly_one_bullet() {
        let s = ShooterSlot::default();
        assert_eq!((s.n_angle, s.n_speed), (1, 1), "默认 1×1,不是 0");
        assert_eq!(s.flags, 0, "aimed/ring/abs_offset 三位全关");
        assert_eq!(s.xform_cnt, 0, "无 xform");
        assert_eq!(s.task_script, SH_NO_TASK, "无挂弹任务");
        assert_eq!(s.on_fire_req, 0, "不发请求");
        assert_eq!((s.off_x, s.off_y), (Fx::ZERO, Fx::ZERO));
        assert_eq!(s.polar_r, Fx::ZERO);
        assert_eq!(s.dist, Fx::ZERO);
        assert_eq!((s.speed0, s.speed_step), (Fx::ZERO, Fx::ZERO));
    }
}
