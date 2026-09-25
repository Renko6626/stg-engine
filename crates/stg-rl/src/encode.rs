//! World → proto v1 行字节（spec §5）。只读 `w.view()` 与公开读口；每行全部字节显式写满。

use crate::layout::{ENEMIES_CAP, ITEMS_CAP, LASERS_CAP, off};
use stg_core::bullets::BulletPool;
use stg_core::enemy::{ENEMY_DYING, ENEMY_NO_BODY, EnemyHandle, pack_handle};
use stg_core::items::{MAGNET_NONE, MAGNET_PICKED};
use stg_core::math::geom::seg_box_dist_sq;
use stg_core::math::{Fx, atan2, isqrt, len_sq};
use stg_core::player::LIFE_ALIVE;
use stg_core::step::World;
use stg_core::tables::WorldTables;

pub const PHASE_IN_GAME: u32 = 1;
pub const PHASE_BOMB_ACTIVE: u32 = 1 << 4;
pub const PHASE_SPELL_ACTIVE: u32 = 1 << 5;
pub const PHASE_PLAYER_CONTROLLABLE: u32 = 1 << 6;
/// 引擎 `ITEM_*` 下标 → proto `AP_ITEM_*`（POWER, POINT, LIFE_PIECE, BOMB_PIECE, STAR）。
pub const ITEM_KIND: [u8; 5] = [1, 2, 8, 9, 7];

#[inline]
fn put_i32(b: &mut [u8], o: usize, v: i32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}
#[inline]
fn put_u32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}
#[inline]
fn put_u16(b: &mut [u8], o: usize, v: u16) {
    b[o..o + 2].copy_from_slice(&v.to_le_bytes());
}

/// 结算分从 `u64` 落到 proto 的 `U32` 字段：**饱和**到 `u32::MAX`，不回绕。
/// 抽成独立函数以便直测（公开途径造不出 `score > u32::MAX` 的世界，见 `tests/encode.rs`）。
#[inline]
fn saturate_score(score: u64) -> u32 {
    score.min(u32::MAX as u64) as u32
}

pub fn phase_bits(w: &World) -> u32 {
    let v = w.view();
    let p = &v.players()[0];
    let mut bits = PHASE_IN_GAME;
    if p.bomb_timer > 0 {
        bits |= PHASE_BOMB_ACTIVE;
    }
    if v.spells().iter().any(|s| s.active != 0) {
        bits |= PHASE_SPELL_ACTIVE;
    }
    if p.life_state == LIFE_ALIVE && v.freeze_left()[1] == 0 {
        bits |= PHASE_PLAYER_CONTROLLABLE;
    }
    bits
}

pub fn write_player(w: &World, tables: &WorldTables, row: &mut [u8]) {
    let p = &w.view().players()[0];
    let cfg = &tables.characters[p.character_id as usize];
    row.fill(0);
    put_i32(row, off::player::X, p.x.raw());
    put_i32(row, off::player::Y, p.y.raw());
    put_i32(row, off::player::HIT_R, p.hit_radius.raw());
    put_i32(row, off::player::SPEED, cfg.high_speed.raw());
    put_i32(row, off::player::SPEED_F, cfg.low_speed.raw());
    row[off::player::FOCUS] = u8::from(p.input & stg_core::input::BTN_SLOW != 0);
    row[off::player::STATE] = p.life_state;
    row[off::player::LIVES] = p.lives;
    row[off::player::BOMBS] = p.bombs;
    row[off::player::LFRAG] = p.life_pieces;
    row[off::player::BFRAG] = p.bomb_pieces;
    put_u16(row, off::player::POWER, p.power);
    put_u32(row, off::player::SCORE, saturate_score(p.score));
    put_u32(row, off::player::GRAZE, p.graze);
}

pub struct BulletStats {
    pub count: usize,
    pub total: usize,
    pub dropped: usize,
}

/// 派生量记忆的一格：`(vx, vy)` 为键，`(speed, angle)` 为其现算值。
#[derive(Clone, Copy)]
struct Kin {
    vx: i32,
    vy: i32,
    speed: i32,
    angle: u16,
}

/// `write_bullets` 的调用方复用暂存：选弹缓冲 + 派生量逐槽记忆。每 env 一份，跨步复用。
///
/// **派生量记忆**：`speed = isqrt(len_sq(vx,vy))`、`angle = atan2(vy,vx)` 是 `(vx, vy)` 的纯函数，
/// 而直线弹（绝大多数弹幕）速度帧间不变——逐池槽记下上次的 `(vx, vy)` 与结果，键值全等才复用。
/// 键比的是**值**不是槽身份，所以槽被复用、弹转向、跨世界混用都不会读到陈旧结果（逐位等价于现算，
/// 由 `tests/encode.rs::bullet_scratch_memo_never_serves_stale_derived` 押运）。命中省掉 atan2
/// （16 轮 CORDIC）+ isqrt 两次调用；未命中只多一次比较。初值全 0 格 = `(0,0) → (0, 0)`，本身即正确映射。
pub struct BulletScratch {
    sel: Vec<(i64, u16)>,
    kin: Vec<Kin>,
}

impl BulletScratch {
    pub fn new() -> BulletScratch {
        BulletScratch {
            sel: Vec::new(),
            kin: vec![
                Kin {
                    vx: 0,
                    vy: 0,
                    speed: 0,
                    angle: 0,
                };
                BulletPool::CAP
            ],
        }
    }
}

impl Default for BulletScratch {
    fn default() -> BulletScratch {
        BulletScratch::new()
    }
}

/// 敌弹行编码：按「离自机最近」选至多 `cap` 颗，输出按池索引升序（I4）。
///
/// `scratch` 由调用方复用（每 env 一份），避免每步堆分配并承载派生量记忆（见 [`BulletScratch`]）。
///
/// # Panics
///
/// - `cap == 0`：`cap` 必须 `>= 1`（plan Global Constraints `1..=8192`，由 HELLO 校验）。
/// - `rows.len() < cap * 30`：每行 30 字节，越界切片会 panic。
pub fn write_bullets(
    w: &World,
    cap: usize,
    rows: &mut [u8],
    scratch: &mut BulletScratch,
) -> BulletStats {
    assert!(
        cap >= 1,
        "bullets cap 必须 >= 1（plan Global Constraints: 1..=8192，由 HELLO 校验）"
    );
    let v = w.view();
    let b = v.bullets();
    let p = &v.players()[0];
    let BulletScratch { sel, kin } = scratch;
    let total = b.iter_alive().count();
    sel.clear();
    if total <= cap {
        sel.extend(b.iter_alive().map(|i| (0, i as u16)));
    } else {
        sel.extend(
            b.iter_alive()
                .map(|i| (len_sq(b.x()[i] - p.x, b.y()[i] - p.y), i as u16)),
        );
        sel.select_nth_unstable(cap - 1); // 全序键 (距离², 池索引) ⇒ 结果确定
        sel.truncate(cap);
        sel.sort_unstable_by_key(|&(_, i)| i);
    }
    for (k, &(_, i)) in sel.iter().enumerate() {
        let i = i as usize;
        let r = &mut rows[k * 30..(k + 1) * 30];
        r.fill(0);
        let (vx, vy) = (b.vx()[i], b.vy()[i]);
        let m = &mut kin[i];
        if m.vx != vx.raw() || m.vy != vy.raw() {
            *m = Kin {
                vx: vx.raw(),
                vy: vy.raw(),
                speed: isqrt(len_sq(vx, vy) as u64).min(i32::MAX as u32) as i32,
                angle: atan2(vy, vx).raw(),
            };
        }
        put_i32(r, off::bullet::X, b.x()[i].raw());
        put_i32(r, off::bullet::Y, b.y()[i].raw());
        put_i32(r, off::bullet::VX, vx.raw());
        put_i32(r, off::bullet::VY, vy.raw());
        put_i32(r, off::bullet::SPEED, m.speed);
        put_u16(r, off::bullet::ANGLE, m.angle);
        put_i32(r, off::bullet::RADIUS, b.radius()[i].raw());
        let delay = b.delay()[i];
        r[off::bullet::FLAGS] =
            u8::from(delay == 0) | 2 | (u8::from(b.grazed_by()[i] & 1 != 0) << 2);
        r[off::bullet::STATE] = u8::from(delay == 0);
        put_u16(r, off::bullet::TYPE, b.sprite()[i]);
    }
    let count = sel.len();
    BulletStats {
        count,
        total,
        dropped: total - count,
    }
}

pub fn write_enemies(w: &World, rows: &mut [u8]) -> usize {
    let v = w.view();
    let e = v.enemies();
    let st = crate::layout::ENEMIES.stride;
    let mut k = 0;
    for i in e.iter_alive().take(ENEMIES_CAP) {
        let r = &mut rows[k * st..(k + 1) * st];
        r.fill(0);
        let h = EnemyHandle {
            index: i as u16,
            generation: e.generation_of(i),
        };
        put_i32(r, off::enemy::X, e.x()[i].raw());
        put_i32(r, off::enemy::Y, e.y()[i].raw());
        put_i32(r, off::enemy::HURT_W, e.hurtbox()[i].raw());
        put_i32(r, off::enemy::HURT_H, e.hurtbox()[i].raw());
        put_i32(r, off::enemy::HIT_W, e.radius()[i].raw());
        put_i32(r, off::enemy::HIT_H, e.radius()[i].raw());
        put_i32(r, off::enemy::HP, e.hp()[i]);
        put_i32(r, off::enemy::HP_MAX, e.hp_max()[i]);
        // 免分配：boss_ui 槽数很小，直接在敌循环里查，避免每步一次 `Vec` 堆分配。
        let boss = v.boss_ui().iter().any(|s| s.active != 0 && s.enemy == h);
        let collidable = e.flags()[i] & (ENEMY_NO_BODY | ENEMY_DYING) == 0;
        put_u16(
            r,
            off::enemy::FLAGS,
            u16::from(boss) | (u16::from(collidable) << 4),
        );
        put_u32(r, off::enemy::ID, pack_handle(h) as u32);
        put_i32(r, off::enemy::VX, e.dx()[i].raw());
        put_i32(r, off::enemy::VY, e.dy()[i].raw());
        k += 1;
    }
    k
}

/// 激光行编码：按「离自机最近」选至多 `LASERS_CAP`(64) 条，输出按 (平方距离, 池下标) 升序。
///
/// 池索引升序收集、键 `(seg_box_dist_sq, 下标)` 全序 ⇒ 结果确定（I4）。不做 CSR 压实：
/// 调用方按 env 给一块定长 `LASERS_CAP * LASERS.stride` 的行区，这里直接写前 `k` 行、其余不动。
///
/// # `t_active` 口径
///
/// 观测在帧末采集；agent 的动作在下一步生效，而下一步的相位 5 会先切换状态、相位 6 才判定，
/// 所以 `t_active == 0` 表示「对下一步已经致命」，与 proto「0 = 现在就杀」一致；`state` 列
/// 区分预警与生效。`state == 0`（预警）时 `t_active = warn − timer`（帧末 `timer` = 本状态内
/// 已过的步数：出生帧末即 1，最后一帧即 `warn`），生效 / 收缩态恒 0。举例 `warn = 3`：出生后
/// 第 0/1/2 步末 `t_active` = 2/1/0，第 3 步末切 `state 1`、`t_active = 0`，此刻开始判定。
/// 与 th06nc 抽取器公式 `+0x270 − timer` 同口径（见 renkolab-sysfix
/// `mods/th06nc/autoplay/TARGET.md`）。
pub fn write_lasers(w: &World, rows: &mut [u8]) -> usize {
    let v = w.view();
    let l = v.lasers();
    let p = &v.players()[0];
    let st = crate::layout::LASERS.stride;
    // ≤ 256 条：定长暂存 + 排序，不做堆分配。键 (平方距离, 下标) 保证确定性。
    let mut keys = [(0i64, 0u16); stg_core::lasers::LaserPool::CAP];
    let mut n = 0;
    for i in l.iter_alive() {
        let half = Fx::from_raw(l.width()[i].raw() / 2);
        keys[n] = (
            seg_box_dist_sq(
                p.x,
                p.y,
                l.ox()[i],
                l.oy()[i],
                l.angle()[i],
                l.start()[i],
                l.end()[i],
                half,
            ),
            i as u16,
        );
        n += 1;
    }
    let keys = &mut keys[..n];
    keys.sort_unstable();
    let k = n.min(LASERS_CAP);
    for (row, &(_, i)) in keys[..k].iter().enumerate() {
        let i = i as usize;
        let r = &mut rows[row * st..(row + 1) * st];
        r.fill(0);
        put_i32(r, off::laser::X, l.ox()[i].raw());
        put_i32(r, off::laser::Y, l.oy()[i].raw());
        put_u16(r, off::laser::ANGLE, l.angle()[i].raw());
        put_i32(r, off::laser::START, l.start()[i].raw());
        put_i32(r, off::laser::END, l.end()[i].raw());
        put_i32(r, off::laser::START_LEN, l.start_len()[i].raw());
        put_i32(r, off::laser::SPEED, l.speed()[i].raw());
        put_i32(r, off::laser::HALF_H, l.width()[i].raw() / 2);
        // BAM/帧 → 弧度/帧的 Q16.16：`dang · 2π`（411775 ≈ 2π·65536），整数运算。
        put_i32(
            r,
            off::laser::OMEGA,
            ((l.dang()[i] as i64 * 411_775) >> 16) as i32,
        );
        put_i32(r, off::laser::VX, l.dx()[i].raw());
        put_i32(r, off::laser::VY, l.dy()[i].raw());
        let t_active = if l.state()[i] == stg_core::lasers::LASER_WARN {
            l.warn()[i] as i32 - l.timer()[i] as i32
        } else {
            0
        };
        put_i32(r, off::laser::T_ACTIVE, t_active);
        r[off::laser::STATE] = l.state()[i];
    }
    k
}

pub fn write_items(w: &World, rows: &mut [u8]) -> usize {
    let it = w.view().items();
    let mut k = 0;
    for i in it.iter_alive() {
        // MAGNET_PICKED（0xFE）只在同帧相位 7→9 之间短暂存在：相位 7 `settle` 标它并
        // 入账，相位 9 `cleanup` 回收全槽。`step` 返回后它已不在存活集里，故此分支纯属
        // 防御性——但保留它才能保证行数 = proto 侧可见道具数这条契约在相位中途也成立。
        if k == ITEMS_CAP || it.magnet_to()[i] == MAGNET_PICKED {
            continue;
        }
        let r = &mut rows[k * 18..(k + 1) * 18];
        r.fill(0);
        put_i32(r, off::item::X, it.x()[i].raw());
        put_i32(r, off::item::Y, it.y()[i].raw());
        put_i32(r, off::item::VX, it.vx()[i].raw());
        put_i32(r, off::item::VY, it.vy()[i].raw());
        r[off::item::KIND] = ITEM_KIND
            .get(it.item_type()[i] as usize)
            .copied()
            .unwrap_or(0);
        r[off::item::FLAGS] = u8::from(it.magnet_to()[i] != MAGNET_NONE);
        k += 1;
    }
    k
}

#[cfg(test)]
mod tests {
    use super::{saturate_score, write_lasers};
    use crate::layout::off;
    use stg_core::math::{Angle, Fx};
    use stg_core::step::World;
    use stg_core::tables::TABLES_V0;

    #[inline]
    fn rd_i32(b: &[u8], o: usize) -> i32 {
        i32::from_le_bytes(b[o..o + 4].try_into().unwrap())
    }
    #[inline]
    fn rd_u16(b: &[u8], o: usize) -> u16 {
        u16::from_le_bytes(b[o..o + 2].try_into().unwrap())
    }

    /// 一帧 step（空镜像：世界内无 ECL 任务）。
    fn run_step(w: &mut World, buttons: u32) {
        let mut input = stg_core::input::InputFrame::empty(w.frame());
        input.actions[0].buttons = buttons;
        stg_core::step::step(
            w,
            &TABLES_V0,
            &stg_core::ecl::image::EclImage::empty(),
            &input,
        );
    }

    /// 全字段 `LaserInit` 助手（形态一的初值；state/timer/anchor 等派生字段由 `create_laser` 覆写）。
    #[allow(clippy::too_many_arguments)]
    fn laser_init(
        ox: i32,
        oy: i32,
        angle: u16,
        len: i32,
        width: i32,
        warn: u16,
        active: u16,
        fade: u16,
    ) -> stg_core::lasers::LaserInit {
        use stg_core::lasers::ANCHOR_NONE;
        stg_core::lasers::LaserInit {
            ox: Fx::from_int(ox),
            oy: Fx::from_int(oy),
            angle: Angle(angle),
            omega: 0,
            start: Fx::ZERO,
            end: Fx::from_int(len),
            start_len: Fx::from_int(len),
            speed: Fx::ZERO,
            width: Fx::from_int(width),
            sprite: 0,
            warn,
            active,
            fade,
            timer: 0,
            state: 0,
            anchor_idx: ANCHOR_NONE,
            anchor_gen: 0,
            ax: Fx::ZERO,
            ay: Fx::ZERO,
            dx: Fx::ZERO,
            dy: Fx::ZERO,
            dang: 0,
            px: Fx::ZERO,
            py: Fx::ZERO,
            pang: Angle::ZERO,
            flags: 0,
            born_frame: 0,
        }
    }

    /// `score` 是 `u64`，proto 字段是 `U32` —— 溢出必须饱和、不得回绕。
    /// 判别力：任何 `as u32`（截断）或 `+1` 回绕实现都会让 0x1_0000_0001 变成 1，测试红。
    #[test]
    fn saturate_score_clamps_to_u32_max() {
        assert_eq!(saturate_score(0), 0);
        assert_eq!(saturate_score(123), 123);
        assert_eq!(saturate_score(u32::MAX as u64), u32::MAX);
        assert_eq!(saturate_score(0x1_0000_0000), u32::MAX, "u32::MAX + 1");
        assert_eq!(saturate_score(0x1_0000_0001), u32::MAX, "u32::MAX + 2");
        assert_eq!(saturate_score(u64::MAX), u32::MAX);
    }

    /// 激光行逐列核对：三形态各一条（形态一挂 omega、形态三挂 speed），第 5 帧前挪形态二原点制造
    /// dx/dy。判别力：半高必须 `width/2`、`t_active` 只对 state 0 计 `warn−timer`、state/type
    /// 直写——任一列错位或公式错都红；所有绝对值互异非零。omega 不在此循环断言（避免与实现同式
    /// 自指），改由下方绝对值 `628` 单独钉死。
    #[test]
    fn laser_rows_carry_pool_fields_omega_and_t_active() {
        use stg_core::lasers::{LASER_ACTIVE, LASER_WARN};

        let mut w = World::new(1);
        // 形态一（预警线 → 扫射）：预警 30、生效 120、收缩 16；omega = 100 bam/帧。
        let l0 = w
            .body
            .create_laser(laser_init(10, 100, 16384, 500, 32, 30, 120, 16));
        assert!(w.body.laser_set_omega(l0, 100));
        // 形态二（自机狙）：预警 24；出生即 aim 到自机方向 + 约 36°。
        let l1 = w
            .body
            .create_laser(laser_init(-40, 120, 0, 400, 20, 24, 90, 12));
        assert!(w.body.laser_aim(l1, Angle(6554)));
        // 形态三（飞出去的棒子）：warn 0 出生即生效，speed 4、棒长 192。
        let l2 = w
            .body
            .create_laser(laser_init(60, 100, 16384, 0, 8, 0, 9999, 0));
        assert!(
            w.body
                .laser_set_speed(l2, Fx::from_int(4), Fx::from_int(192))
        );

        for _ in 0..4 {
            run_step(&mut w, 0);
        }
        // 第 5 帧前挪形态二的原点：本帧 dx 应为新原点 − 旧原点（90px），dy = 0。
        assert!(w.body.laser_origin(l1, Fx::from_int(50), Fx::from_int(120)));
        run_step(&mut w, 0);

        let v = w.view();
        let l = v.lasers();
        let (i0, i1, i2) = (l0.index as usize, l1.index as usize, l2.index as usize);
        // 前置：三条都真的按预期演化（否则后面的行断言可能退化成"比 0"）。
        assert_eq!((l.state()[i0], l.timer()[i0]), (LASER_WARN, 5));
        assert_eq!((l.state()[i1], l.timer()[i1]), (LASER_WARN, 5));
        assert_eq!(l.state()[i2], LASER_ACTIVE);
        assert_eq!(l.omega()[i0], 100, "形态一真的挂了 omega");
        assert_ne!(l.angle()[i1], Angle(0), "形态二真的被 aim 改了角度");
        assert_eq!(l.speed()[i2], Fx::from_int(4), "形态三真的挂了 speed");

        let st = crate::layout::LASERS.stride;
        let mut rows = vec![0u8; 64 * st];
        assert_eq!(write_lasers(&w, &mut rows), 3);

        // half_h 识别行（16 / 10 / 4 互异；列值是 Q16.16，比较 raw）。
        let find = |half: i32| -> usize {
            let want = Fx::from_int(half).raw();
            (0..3)
                .find(|&k| rd_i32(&rows[k * st..], off::laser::HALF_H) == want)
                .unwrap_or_else(|| panic!("找不到 half_h = {half} 的行"))
        };
        // 每条激光的期望（i, half_h, state, t_active, dx_px, dy_px）。
        // 第 5 步末 timer == 5 ⇒ t_active = warn − 5：30→25、24→19；生效态恒 0。
        let want = [
            (i0, 16, LASER_WARN, 25, 0, 0),
            (i1, 10, LASER_WARN, 19, 90, 0),
            (i2, 4, LASER_ACTIVE, 0, 0, 0),
        ];
        for &(i, half, state, t_active, dx, dy) in &want {
            let k = find(half);
            let r = &rows[k * st..(k + 1) * st];
            assert_eq!(rd_i32(r, off::laser::X), l.ox()[i].raw(), "x");
            assert_eq!(rd_i32(r, off::laser::Y), l.oy()[i].raw(), "y");
            assert_eq!(rd_u16(r, off::laser::ANGLE), l.angle()[i].raw(), "angle");
            assert_eq!(rd_i32(r, off::laser::START), l.start()[i].raw(), "start");
            assert_eq!(rd_i32(r, off::laser::END), l.end()[i].raw(), "end");
            assert_eq!(
                rd_i32(r, off::laser::START_LEN),
                l.start_len()[i].raw(),
                "start_len"
            );
            assert_eq!(rd_i32(r, off::laser::SPEED), l.speed()[i].raw(), "speed");
            assert_eq!(
                rd_i32(r, off::laser::HALF_H),
                l.width()[i].raw() / 2,
                "half_h = width/2"
            );
            assert_eq!(rd_i32(r, off::laser::VX), Fx::from_int(dx).raw(), "vx = dx");
            assert_eq!(rd_i32(r, off::laser::VY), Fx::from_int(dy).raw(), "vy = dy");
            assert_eq!(rd_i32(r, off::laser::T_ACTIVE), t_active, "t_active");
            assert_eq!(r[off::laser::STATE], state, "state");
            assert_eq!(r[off::laser::TYPE], 0, "type");
        }
        // 绝对取值再钉一遍关键列（防"行 ↔ 池"整体错位也自洽）。
        let r0 = &rows[find(16) * st..][..st];
        assert_eq!(rd_i32(r0, off::laser::X), Fx::from_int(10).raw());
        assert_eq!(rd_i32(r0, off::laser::Y), Fx::from_int(100).raw());
        assert_eq!(rd_i32(r0, off::laser::END), Fx::from_int(500).raw());
        assert_eq!(rd_i32(r0, off::laser::START_LEN), Fx::from_int(500).raw());
        assert_eq!(rd_i32(r0, off::laser::OMEGA), 628);
        let r1 = &rows[find(10) * st..][..st];
        assert_eq!(rd_i32(r1, off::laser::X), Fx::from_int(50).raw());
        assert_eq!(rd_i32(r1, off::laser::VX), Fx::from_int(90).raw());
        let r2 = &rows[find(4) * st..][..st];
        assert_eq!(rd_i32(r2, off::laser::END), Fx::from_int(20).raw());
        assert_eq!(rd_i32(r2, off::laser::SPEED), Fx::from_int(4).raw());
    }

    /// `t_active` = `warn − timer`（观测在帧末采集）：warn = 3 的激光在出生后第 0、1、2 步末
    /// 依次为 2、1、0，第 3 步末切 `state 1`、`t_active = 0`。`t_active == 0` 表示「对下一步
    /// 已经致命」（下一步相位 5 先切态、相位 6 判定），`state` 列区分预警与生效。
    #[test]
    fn t_active_is_warn_minus_timer() {
        use stg_core::lasers::{LASER_ACTIVE, LASER_WARN};

        let mut w = World::new(1);
        let h = w
            .body
            .create_laser(laser_init(0, 100, 16384, 500, 32, 3, 120, 16));
        let i = h.index as usize;
        let st = crate::layout::LASERS.stride;
        let mut rows = vec![0u8; st];
        // 编码一行并回读 (state, t_active)。
        let probe = |w: &World, rows: &mut [u8]| -> (u8, i32) {
            assert_eq!(write_lasers(w, rows), 1);
            (rows[off::laser::STATE], rd_i32(rows, off::laser::T_ACTIVE))
        };

        for (step, want) in [2, 1, 0].into_iter().enumerate() {
            run_step(&mut w, 0);
            assert_eq!(
                w.view().lasers().state()[i],
                LASER_WARN,
                "第 {step} 步末在预警"
            );
            assert_eq!(
                probe(&w, &mut rows),
                (LASER_WARN, want),
                "第 {step} 步末 t_active 应为 {want}"
            );
        }
        run_step(&mut w, 0);
        assert_eq!(w.view().lasers().state()[i], LASER_ACTIVE, "第 3 步末生效");
        assert_eq!(
            probe(&w, &mut rows),
            (LASER_ACTIVE, 0),
            "生效态 t_active 恒 0"
        );
    }

    /// 条数上限：70 条，只写出最近的 64 条（`ox=0, oy=i` ⇒ 距离² = (384−i)²，i 越大越近），
    /// 输出按 (距离, 下标) 升序；重复调用逐字节相同（含未被写的尾行）。
    #[test]
    fn lasers_cap_keeps_nearest_and_is_reproducible() {
        let mut w = World::new(1);
        for i in 0..70i32 {
            // len=0、angle 竖直：线段退化成点，half_h = i+1 可作行识别（宽度不影响距离）。
            let h = w
                .body
                .create_laser(laser_init(0, i, 16384, 0, 2 * (i + 1), 0, 9999, 0));
            assert_ne!(h, stg_core::lasers::LaserHandle::NULL, "第 {i} 条应建成");
        }
        let st = crate::layout::LASERS.stride;
        let mut a = vec![0xAAu8; 64 * st];
        let mut b = vec![0xAAu8; 64 * st];
        assert_eq!(write_lasers(&w, &mut a), 64, "只写出 cap=64 条");
        assert_eq!(write_lasers(&w, &mut b), 64);
        assert_eq!(a, b, "重复调用逐字节相同");
        let halfs: Vec<i32> = (0..64)
            .map(|k| rd_i32(&a[k * st..], off::laser::HALF_H))
            .collect();
        // 最近 64 条 = i ∈ 6..=69，距离升序 = i 降序 ⇒ half_h = 70..=7（Q16.16 raw）。
        let want: Vec<i32> = (7..=70).rev().map(|v| Fx::from_int(v).raw()).collect();
        assert_eq!(halfs, want, "应取最近 64 条且按 (距离, 下标) 升序");
    }

    /// 等距平局按下标升序（keys = `(距离², 下标)`）；行序错则 half_h 序列不是 1,2,3。
    #[test]
    fn lasers_tie_breaks_by_pool_index() {
        let mut w = World::new(1);
        for i in 0..3i32 {
            w.body
                .create_laser(laser_init(0, 300, 16384, 0, 2 * (i + 1), 0, 9999, 0));
        }
        let st = crate::layout::LASERS.stride;
        let mut rows = vec![0u8; 3 * st];
        assert_eq!(write_lasers(&w, &mut rows), 3);
        let halfs: Vec<i32> = (0..3)
            .map(|k| rd_i32(&rows[k * st..], off::laser::HALF_H))
            .collect();
        assert_eq!(
            halfs,
            vec![
                Fx::from_int(1).raw(),
                Fx::from_int(2).raw(),
                Fx::from_int(3).raw()
            ],
            "等距按下标升序"
        );
    }
}
