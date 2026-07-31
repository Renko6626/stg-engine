//! 相位 5 · 积分（各池 `pos += vel` + 计时器倒数）。
//!
//! 冻结趟序（`stg-world-design.md:168`）：弹 → 自机弹 → 敌人 → 道具 → 作用区。
//!
//! 弹：delay 门 → 模式效果（POLAR/CART 互斥）→ `pos += vel` → life 倒数。
//! 自机弹：`pos += vel`。
//! 敌人：**分层**（敌人运动动词族刀 / spec §3.2、§6.1）——① 速度插值恒跑（只改速度，
//! 极坐标插 speed/angle 再刷 vx/vy，笛卡尔插 vx/vy 再回填），② `mv_active` 时位置插值
//! 接管位置（不读 vx/vy），否则照常 `pos += vel`。到点清速条件化：判据是黏滞位
//! `vel_touched`（§6.3）。另 tick `invuln`/`hit_flash`，两个计时器分支外照常。
//! 道具：触发判定（PoC / 近距磁吸）先于移动 —— 磁吸=直追终速、未锁定/解锁=重力到终速钉住。
//! 作用区：`life` 倒数 —— `life=1` 本帧减到 0、相位 6 仍参与判定、相位 9 才回收（"每帧重铺=跟随"的时序基础）。

use super::WorldBody;
use crate::math::Fx;
use crate::tables::WorldTables;

impl WorldBody {
    pub(crate) fn integrate(&mut self, tables: &WorldTables) {
        self.phase_enter(super::PH_INTEGRATE);
        let nw = self.bullets.alive.len();
        for w in 0..nw {
            let mut bits = self.bullets.alive[w];
            while bits != 0 {
                let i = w * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                if self.bullets.delay[i] > 0 {
                    self.bullets.delay[i] -= 1; // delay 期不动
                    continue;
                }
                let fl = self.bullets.flags[i];
                debug_assert_ne!(
                    fl & (crate::bullets::BULLET_POLAR_FX | crate::bullets::BULLET_CART_FX),
                    crate::bullets::BULLET_POLAR_FX | crate::bullets::BULLET_CART_FX,
                    "模式位互斥被破坏（P4-c 帧内断言）"
                );
                if fl & crate::bullets::BULLET_POLAR_FX != 0 {
                    self.bullets.angle[i] =
                        self.bullets.angle[i].add_delta(self.bullets.ang_vel[i]);
                    self.bullets.speed[i] = self.bullets.speed[i] + self.bullets.accel[i];
                    self.refresh_vel_from_polar(i);
                } else if fl & crate::bullets::BULLET_CART_FX != 0 {
                    self.bullets.vx[i] = self.bullets.vx[i] + self.bullets.ax[i];
                    self.bullets.vy[i] = self.bullets.vy[i] + self.bullets.ay[i];
                    self.backfill_polar(i);
                }
                self.bullets.x[i] = self.bullets.x[i] + self.bullets.vx[i];
                self.bullets.y[i] = self.bullets.y[i] + self.bullets.vy[i];
                // D4 反弹：位移后同帧折返（每帧每轴至多一次；速度按模式全域一致更新）
                if self.bullets.flags[i] & crate::bullets::BULLET_BOUNCE_MASK != 0 {
                    self.bounce_bullet(i);
                }
                if self.bullets.life[i] != 0xFFFF && self.bullets.life[i] > 0 {
                    self.bullets.life[i] -= 1;
                }
            }
        }
        // 自机弹：pos += vel（无 delay/life）
        let nw = self.shots.alive.len();
        for w in 0..nw {
            let mut bits = self.shots.alive[w];
            while bits != 0 {
                let i = w * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                self.shots.x[i] = self.shots.x[i] + self.shots.vx[i];
                self.shots.y[i] = self.shots.y[i] + self.shots.vy[i];
            }
        }
        // 敌人：分层 —— ① 速度插值恒跑，② move_to 插值器接管位置（D5），否则 pos += vel；
        // 计时器 tick 分支外照常。遍历按池索引升序（I4）。
        let nw = self.enemies.alive.len();
        for w in 0..nw {
            let mut bits = self.enemies.alive[w];
            while bits != 0 {
                let i = w * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                // ① 速度插值恒跑（分层：它只改速度，不决定位置归谁）
                if self.enemies.vel_active[i] != 0 {
                    self.tick_enemy_vel(i);
                }
                // ② 位置插值接管位置，否则匀速积分
                if self.enemies.mv_active[i] != 0 {
                    // D5 插值器优先：绝对插值（每帧从 from 重算，不累积误差；e≤1.0 白名单乘法）
                    self.enemies.mv_t[i] += 1;
                    if self.enemies.mv_t[i] >= self.enemies.mv_dur[i] {
                        // 到点即停（精确终点，不吃舍入；清速防残留漂移）
                        self.enemies.x[i] = self.enemies.mv_to_x[i];
                        self.enemies.y[i] = self.enemies.mv_to_y[i];
                        // 到点清速**条件化**（敌人运动动词族刀）：只在脚本从未表达过速度
                        // 意图时清。判据是黏滞位 vel_touched 而**不是** vel_active——
                        // 速度插值常常先于位置插值到期，那时 vel_active 已归 0，
                        // 拿它当判据会把刚缓好的速度误清（spec §6.3 的修订记录）。
                        if self.enemies.vel_touched[i] == 0 {
                            self.enemies.vx[i] = Fx::ZERO;
                            self.enemies.vy[i] = Fx::ZERO;
                        }
                        self.enemies.mv_active[i] = 0;
                    } else {
                        let t = Fx::from_raw(
                            (((self.enemies.mv_t[i] as i64) << 16) / self.enemies.mv_dur[i] as i64)
                                as i32,
                        );
                        let e = crate::math::easing::ease(
                            crate::math::easing::from_id(self.enemies.mv_easing[i]),
                            t,
                        );
                        let fx = self.enemies.mv_from_x[i].raw() as i64;
                        let fy = self.enemies.mv_from_y[i].raw() as i64;
                        let dx = self.enemies.mv_to_x[i].raw() as i64 - fx;
                        let dy = self.enemies.mv_to_y[i].raw() as i64 - fy;
                        // dx/dy 可为负；`>>16` 对 i64 是算术右移——确定性（同 STEP 先例）
                        self.enemies.x[i] =
                            Fx::from_raw((fx + ((dx * e.raw() as i64) >> 16)) as i32);
                        self.enemies.y[i] =
                            Fx::from_raw((fy + ((dy * e.raw() as i64) >> 16)) as i32);
                    }
                } else {
                    self.enemies.x[i] = self.enemies.x[i] + self.enemies.vx[i];
                    self.enemies.y[i] = self.enemies.y[i] + self.enemies.vy[i];
                }
                if self.enemies.invuln[i] > 0 {
                    self.enemies.invuln[i] -= 1;
                }
                if self.enemies.hit_flash[i] > 0 {
                    self.enemies.hit_flash[i] -= 1;
                }
            }
        }
        // 道具（D7）：触发判定先于移动；物理即状态（磁吸=magnet_to、下落=重力到终速）。
        let poc_player = (0..crate::MAX_PLAYERS).find(|&p| {
            self.players[p].life_state == crate::player::LIFE_ALIVE
                && self.players[p].y.raw() < Fx::from_int(super::POC_LINE_Y).raw()
        });
        let nw = self.items.alive.len();
        for w in 0..nw {
            let mut bits = self.items.alive[w];
            while bits != 0 {
                let i = w * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                self.integrate_item(i, poc_player, tables);
            }
        }
        // 作用区：寿命倒数（照抄弹的模式；life=1 → 本帧减到 0，相位6 仍参与判定，相位9 回收）
        let nw = self.fields.alive.len();
        for w in 0..nw {
            let mut bits = self.fields.alive[w];
            while bits != 0 {
                let i = w * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                if self.fields.life[i] > 0 {
                    self.fields.life[i] -= 1;
                }
            }
        }
    }

    /// 推进一只敌人的速度插值一帧（敌人运动动词族刀）。绝对插值：每帧从 `from` 重算，
    /// 不累积误差；终帧写精确终值。两条空间路径的差别是**这刀的全部要点**——
    /// 极坐标插 `speed`/`angle` 再刷 `vx/vy`（匀速扫弧），笛卡尔插 `vx/vy` 再回填
    /// `speed`/`angle`（直线穿过、中途掉速）。把笛卡尔那条改成转极坐标去插，
    /// `move_vel_xy` 就退化成 `move_vel` 的语法糖了（spec §3.3）。
    fn tick_enemy_vel(&mut self, i: usize) {
        self.enemies.vel_t[i] += 1;
        let done = self.enemies.vel_t[i] >= self.enemies.vel_dur[i];
        // e ∈ [0,1]：done 帧不参与运算（走 to/delta 直写路径），故只在未完成时求
        let e = if done {
            Fx::ZERO
        } else {
            let t = Fx::from_raw(
                (((self.enemies.vel_t[i] as i64) << 16) / self.enemies.vel_dur[i] as i64) as i32,
            );
            crate::math::easing::ease(crate::math::easing::from_id(self.enemies.vel_easing[i]), t)
        };
        let (f0, f1) = (self.enemies.vel_from_0[i], self.enemies.vel_from_1[i]);
        let (t0, t1) = (self.enemies.vel_to_0[i], self.enemies.vel_to_1[i]);
        if self.enemies.vel_space[i] == crate::enemy::VEL_SPACE_CART {
            // 笛卡尔：两个分量各自线性插（负 delta 的 `>>16` 是算术右移，两平台一致）
            let vx = if done {
                t0
            } else {
                (f0 as i64 + (((t0 as i64 - f0 as i64) * e.raw() as i64) >> 16)) as i32
            };
            let vy = if done {
                t1
            } else {
                (f1 as i64 + (((t1 as i64 - f1 as i64) * e.raw() as i64) >> 16)) as i32
            };
            self.enemies.vx[i] = Fx::from_raw(vx);
            self.enemies.vy[i] = Fx::from_raw(vy);
            self.backfill_enemy_polar(i);
        } else {
            // 极坐标：速率线性插；角度走**最短弧**（同 transform.rs 的 STEP_ANGLE）
            let sp = if done {
                t0
            } else {
                (f0 as i64 + (((t0 as i64 - f0 as i64) * e.raw() as i64) >> 16)) as i32
            };
            let start = crate::math::Angle(f1 as u16);
            let delta = (t1 as u16).wrapping_sub(start.raw()) as i16;
            let scaled = if done {
                delta
            } else {
                ((delta as i64 * e.raw() as i64) >> 16) as i16
            };
            self.enemies.speed[i] = Fx::from_raw(sp);
            self.enemies.angle[i] = start.add_delta(scaled);
            self.refresh_enemy_vel_from_polar(i);
        }
        if done {
            self.enemies.vel_active[i] = 0;
        }
    }

    /// 单颗道具的一帧：触发（PoC / 近距）→ 磁吸或下落移动。已拾取（0xFE）静置待回收。
    fn integrate_item(&mut self, i: usize, poc_player: Option<usize>, tables: &WorldTables) {
        use crate::items::{MAGNET_NONE, MAGNET_PICKED};
        let m = self.items.magnet_to[i];
        if m == MAGNET_PICKED {
            return;
        }
        let cfg = &tables.item_cfg[self.items.item_type[i] as usize];
        if m == MAGNET_NONE {
            if let Some(p) = poc_player {
                self.items.magnet_to[i] = p as u8;
            } else {
                let r2 = (cfg.attract_radius.raw() as i64) * (cfg.attract_radius.raw() as i64);
                for p in 0..crate::MAX_PLAYERS {
                    if self.players[p].life_state != crate::player::LIFE_ALIVE {
                        continue;
                    }
                    let d2 = crate::math::geom::len_sq(
                        self.players[p].x - self.items.x[i],
                        self.players[p].y - self.items.y[i],
                    );
                    if d2 <= r2 {
                        self.items.magnet_to[i] = p as u8; // 升序首个 = 低索引（I4）
                        break;
                    }
                }
            }
        }
        let m = self.items.magnet_to[i];
        if (m as usize) < crate::MAX_PLAYERS {
            let p = m as usize;
            if self.players[p].life_state != crate::player::LIFE_ALIVE {
                // 解锁回落：vx 清零、vy 保持，本帧起按下落走
                self.items.magnet_to[i] = MAGNET_NONE;
                self.items.vx[i] = Fx::ZERO;
            } else {
                // 每帧重瞄直追（磁吸速度恒定）
                let a = crate::math::cordic::atan2(
                    self.players[p].y - self.items.y[i],
                    self.players[p].x - self.items.x[i],
                );
                let (vx, vy) = crate::math::geom::polar_to_vec(cfg.magnet_speed, a);
                self.items.vx[i] = vx;
                self.items.vy[i] = vy;
                self.items.x[i] = self.items.x[i] + vx;
                self.items.y[i] = self.items.y[i] + vy;
                return;
            }
        }
        // 未锁定/刚解锁：重力到终速钉住
        let nvy = self.items.vy[i] + tables.item_gravity;
        self.items.vy[i] = if nvy.raw() > cfg.terminal_vy.raw() {
            cfg.terminal_vy
        } else {
            nvy
        };
        self.items.x[i] = self.items.x[i] + self.items.vx[i];
        self.items.y[i] = self.items.y[i] + self.items.vy[i];
    }

    /// 场界折返镜像（D4 11b 拍板）。walls：bit0 左 / bit1 右 / bit2 上 / bit3 下。
    fn bounce_bullet(&mut self, i: usize) {
        use crate::bullets::{BULLET_BOUNCE_MASK, BULLET_BOUNCE_SHIFT, BULLET_POLAR_FX};
        let walls = self.bounce_walls_of(i);
        if walls == 0 {
            return;
        }
        let left = Fx::from_int(-super::FIELD_HALF_W);
        let right = Fx::from_int(super::FIELD_HALF_W);
        let top = Fx::ZERO;
        let bottom = Fx::from_int(super::FIELD_HEIGHT);
        // x 轴（每帧至多一次）
        let mut count = (self.bullets.flags[i] & BULLET_BOUNCE_MASK) >> BULLET_BOUNCE_SHIFT;
        let x = self.bullets.x[i];
        let hit_x = (walls & 0b0001 != 0 && x.raw() < left.raw())
            .then_some(left)
            .or((walls & 0b0010 != 0 && x.raw() > right.raw()).then_some(right));
        if let (Some(wall), true) = (hit_x, count > 0) {
            self.bullets.x[i] = Fx::from_raw(2 * wall.raw() - x.raw()); // 折返镜像
            if self.bullets.flags[i] & BULLET_POLAR_FX != 0 {
                let a = self.bullets.angle[i];
                self.bullets.angle[i] = crate::math::Angle::HALF.sub(a); // 垂直墙：HALF−θ
                self.refresh_vel_from_polar(i);
            } else {
                self.bullets.vx[i] = Fx::ZERO - self.bullets.vx[i];
                self.backfill_polar(i);
            }
            count -= 1;
        }
        // y 轴（重读计数——角撞允许同帧双轴各一次）
        let y = self.bullets.y[i];
        let hit_y = (walls & 0b0100 != 0 && y.raw() < top.raw())
            .then_some(top)
            .or((walls & 0b1000 != 0 && y.raw() > bottom.raw()).then_some(bottom));
        if let (Some(wall), true) = (hit_y, count > 0) {
            self.bullets.y[i] = Fx::from_raw(2 * wall.raw() - y.raw());
            if self.bullets.flags[i] & BULLET_POLAR_FX != 0 {
                let a = self.bullets.angle[i];
                self.bullets.angle[i] = crate::math::Angle::ZERO.sub(a); // 水平墙：−θ
                self.refresh_vel_from_polar(i);
            } else {
                self.bullets.vy[i] = Fx::ZERO - self.bullets.vy[i];
                self.backfill_polar(i);
            }
            count -= 1;
        }
        self.bullets.flags[i] =
            (self.bullets.flags[i] & !BULLET_BOUNCE_MASK) | (count << BULLET_BOUNCE_SHIFT);
    }
}

#[cfg(test)]
mod tests {
    use crate::bullets::BULLET_CART_FX;
    use crate::input::InputFrame;
    use crate::math::Angle;
    use crate::math::Fx;
    use crate::math::geom::polar_to_vec;
    use crate::world::test_support::{bullet_at, spawn_enemy};
    use crate::xform::*;

    // 与 transform.rs 测试同款助手
    fn slot(wait: u16, op: u8, a0: i32, a1: i32) -> XformSlot {
        XformSlot {
            wait,
            op,
            _pad: 0,
            args: [a0, a1],
        }
    }

    // 与 transform.rs 测试同款助手：造一颗静止带段弹（远离自机），返回池索引（单弹场景恒 0）。
    fn xf_bullet(w: &mut crate::step::World, seq: &[XformSlot]) -> usize {
        let h = w.body.create_bullet_with_xform(
            crate::bullets::BulletInit {
                x: Fx::from_int(0),
                y: Fx::from_int(100),
                vx: Fx::ZERO,
                vy: Fx::ZERO,
                speed: Fx::ZERO,
                angle: Angle::ZERO,
                ang_vel: 0,
                accel: Fx::ZERO,
                ax: Fx::ZERO,
                ay: Fx::ZERO,
                sprite: 0,
                radius: Fx::from_int(2),
                delay: 0,
                life: 0xFFFF,
                flags: 0,
                grazed_by: 0,
                transform_head: 0,
                xform_wait: 0,
                xform_next: 0,
            },
            seq,
        );
        w.body.bullets.get(h).unwrap()
    }

    /// `delay` 门：delay 期弹只倒数、不移动；delay 尽后才开始积分。
    ///
    /// 走真实 `step`（而非直接调 `collide`）—— 这是唯一能触达 integrate 里那个 delay 门的路径：
    /// M0-9 复审实测，既有的 delay 测试直接调 `collide()`，根本到不了相位 5。
    #[test]
    fn integrate_delay_gate_holds_bullet_then_releases() {
        let mut w = crate::step::World::new(1);
        bullet_at(&mut w, 0, 100);
        w.body.bullets.vx[0] = Fx::from_int(3);
        w.body.bullets.delay[0] = 2;

        // delay 期：不动，只倒数
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(0));
        assert_eq!(w.body.bullets.x[0], Fx::ZERO, "delay 期弹不该移动");
        assert_eq!(w.body.bullets.delay[0], 1);

        crate::world::test_support::step_t(&mut w, &InputFrame::empty(1));
        assert_eq!(w.body.bullets.x[0], Fx::ZERO, "delay 期弹不该移动");
        assert_eq!(w.body.bullets.delay[0], 0);

        // delay 尽 → 开始积分
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(2));
        assert_eq!(w.body.bullets.x[0], Fx::from_int(3), "delay 尽后应开始移动");
    }

    /// 螺旋判别式：ω=1024 BAM/帧 × 16 帧 = 1/4 圈，vx/vy 与查表参考逐位相等。
    #[test]
    fn polar_fx_spiral_matches_table_after_quarter_turn() {
        let mut w = crate::step::World::new(1);
        let h = bullet_at(&mut w, 0, 100);
        let i = w.body.bullets.get(h).unwrap();
        w.body.bullets.speed[i] = Fx::from_int(2);
        w.body.bullets.angle[i] = Angle::ZERO;
        w.body.refresh_vel_from_polar(i);
        w.body.set_ang_vel_at(i, 1024);
        for f in 0..16u32 {
            crate::world::test_support::step_t(&mut w, &InputFrame::empty(f));
        }
        assert_eq!(w.body.bullets.angle[i], Angle::QUARTER);
        let (rvx, rvy) = polar_to_vec(Fx::from_int(2), Angle::QUARTER);
        assert_eq!(w.body.bullets.vx[i], rvx);
        assert_eq!(w.body.bullets.vy[i], rvy);
    }

    /// 沿向加速判别式：speed 线性累加，v 与查表参考一致。
    #[test]
    fn polar_fx_accel_grows_speed() {
        let mut w = crate::step::World::new(1);
        let h = bullet_at(&mut w, 0, 100);
        let i = w.body.bullets.get(h).unwrap();
        w.body.bullets.speed[i] = Fx::from_int(1);
        w.body.bullets.angle[i] = Angle::ZERO;
        w.body.refresh_vel_from_polar(i);
        w.body.set_accel_at(i, Fx::from_raw(3277)); // ~0.05 px/帧²
        for f in 0..10u32 {
            crate::world::test_support::step_t(&mut w, &InputFrame::empty(f));
        }
        assert_eq!(w.body.bullets.speed[i].raw(), 65536 + 10 * 3277);
        let (rvx, _) = polar_to_vec(Fx::from_raw(65536 + 10 * 3277), Angle::ZERO);
        assert_eq!(w.body.bullets.vx[i], rvx);
    }

    /// delay 门冻结 POLAR：delay 期 angle/speed/位置全不动（D3：变换不走）。
    #[test]
    fn delay_gate_freezes_polar_fx() {
        let mut w = crate::step::World::new(1);
        let h = bullet_at(&mut w, 0, 100);
        let i = w.body.bullets.get(h).unwrap();
        w.body.bullets.speed[i] = Fx::from_int(2);
        w.body.bullets.angle[i] = Angle::ZERO;
        w.body.refresh_vel_from_polar(i);
        w.body.set_ang_vel_at(i, 1024);
        w.body.bullets.delay[i] = 2;
        for f in 0..2u32 {
            crate::world::test_support::step_t(&mut w, &InputFrame::empty(f));
        }
        assert_eq!(w.body.bullets.angle[i], Angle::ZERO, "delay 期角度不得推进");
        assert_eq!(w.body.bullets.x[i], Fx::ZERO, "delay 期不得移动");
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(2));
        assert_eq!(
            w.body.bullets.angle[i],
            Angle(1024),
            "delay 尽后首帧推进一步"
        );
    }

    /// 重力弹判别式：上抛过顶点 vy 翻号、angle 每帧跟随 atan2 参考（几何可判对错）。
    #[test]
    fn cart_fx_gravity_parabola_flips_vy_and_tracks_angle() {
        let mut w = crate::step::World::new(1);
        let h = bullet_at(&mut w, 0, 200);
        let i = w.body.bullets.get(h).unwrap();
        w.body.bullets.vx[i] = Fx::from_int(1);
        w.body.bullets.vy[i] = Fx::from_int(-3); // 上抛（y 向下为正）
        w.body.set_gravity_at(i, Fx::ZERO, Fx::from_raw(16384)); // ay = 0.25 px/帧²
        assert_ne!(
            w.body.bullets.flags[i] & BULLET_CART_FX,
            0,
            "应已开 CART_FX"
        );
        for f in 0..20u32 {
            crate::world::test_support::step_t(&mut w, &InputFrame::empty(f));
        }
        // vy = -3 + 20×0.25 = +2：过了顶点
        assert_eq!(w.body.bullets.vy[i].raw(), -3 * 65536 + 20 * 16384);
        assert!(w.body.bullets.vy[i].raw() > 0);
        // angle/speed 每帧回填：与参考逐位相等
        let (vx, vy) = (w.body.bullets.vx[i], w.body.bullets.vy[i]);
        assert_eq!(w.body.bullets.angle[i], crate::math::cordic::atan2(vy, vx));
        let sp = crate::math::isqrt::isqrt(crate::math::geom::len_sq(vx, vy) as u64) as i32;
        assert_eq!(w.body.bullets.speed[i].raw(), sp);
    }

    /// 右墙折返判别式（哑弹）：x 越界量镜像 + vx 翻号 + 计数递减。
    /// 场界 x=+192；弹从 190 以 vx=+5 一帧到 195 → 折返到 189、vx=-5。
    #[test]
    fn bounce_right_wall_folds_position_and_flips_vx() {
        use crate::bullets::BULLET_BOUNCE_MASK;
        let mut w = crate::step::World::new(1);
        let i = xf_bullet(
            &mut w,
            &[
                slot(0, OP_BOUNCE_ARM, 0b0010, 2), // walls=右；n=2
            ],
        );
        w.body.bullets.x[i] = Fx::from_int(190);
        w.body.bullets.y[i] = Fx::from_int(100);
        w.body.bullets.vx[i] = Fx::from_int(5);
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(0)); // BOUNCE_ARM 发射 + 位移 195 → 折返
        assert_eq!(w.body.bullets.x[i], Fx::from_int(189), "2·192−195 = 189");
        assert_eq!(w.body.bullets.vx[i], Fx::from_int(-5));
        assert_eq!(
            (w.body.bullets.flags[i] & BULLET_BOUNCE_MASK) >> 3,
            1,
            "计数 2→1"
        );
    }

    /// POLAR 弹镜像走角度域：右墙后 angle = HALF − θ，且 vx/vy 与查表参考一致。
    #[test]
    fn bounce_polar_bullet_mirrors_angle() {
        let mut w = crate::step::World::new(1);
        let i = xf_bullet(
            &mut w,
            &[
                slot(0, OP_SET_SPEED, Fx::from_int(6).raw(), 0),
                slot(0, OP_SET_ANGLE, 8192, 0), // 45°（右下）
                slot(0, OP_SET_ANG_VEL, 0, 0),  // 开 POLAR（ω=0：只为进角度域）
                slot(0, OP_BOUNCE_ARM, 0b0010, 1),
            ],
        );
        w.body.bullets.x[i] = Fx::from_int(189);
        w.body.bullets.y[i] = Fx::from_int(100);
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(0));
        assert_eq!(
            w.body.bullets.angle[i],
            Angle::HALF.sub(Angle(8192)),
            "垂直墙：HALF−θ"
        );
        let (rvx, rvy) = polar_to_vec(Fx::from_int(6), Angle::HALF.sub(Angle(8192)));
        assert_eq!((w.body.bullets.vx[i], w.body.bullets.vy[i]), (rvx, rvy));
    }

    /// 未武装的墙不反弹：只武装右墙的弹撞上墙照常越界、被 OOB 回收。
    #[test]
    fn unarmed_wall_does_not_bounce() {
        let mut w = crate::step::World::new(1);
        let i = xf_bullet(&mut w, &[slot(0, OP_BOUNCE_ARM, 0b0010, 3)]); // 只右墙
        w.body.bullets.x[i] = Fx::from_int(-190);
        w.body.bullets.y[i] = Fx::from_int(100);
        w.body.bullets.vx[i] = Fx::from_int(-8); // 左飞
        for f in 0..12u32 {
            crate::world::test_support::step_t(&mut w, &InputFrame::empty(f)); // −190−8k，越 OOB(−256) 即回收
        }
        assert!(!w.body.bullets.is_alive(i), "未武装左墙：照常越界回收");
    }

    /// 计数耗尽墙失效：n=1 弹第一次反弹后第二次撞墙直接穿出。
    #[test]
    fn bounce_count_exhausts() {
        let mut w = crate::step::World::new(1);
        let i = xf_bullet(&mut w, &[slot(0, OP_BOUNCE_ARM, 0b0011, 1)]); // 左右墙 n=1
        w.body.bullets.x[i] = Fx::from_int(190);
        w.body.bullets.y[i] = Fx::from_int(100);
        w.body.bullets.vx[i] = Fx::from_int(60); // 大步伐来回撞
        let mut bounced_once = false;
        for f in 0..20u32 {
            crate::world::test_support::step_t(&mut w, &InputFrame::empty(f));
            if w.body.bullets.is_alive(i) && w.body.bullets.vx[i].raw() < 0 {
                bounced_once = true;
            }
        }
        assert!(bounced_once, "第一次必须反弹");
        assert!(!w.body.bullets.is_alive(i), "耗尽后必须穿出被回收");
    }

    /// 次帧首动 + 重力到终速钉住：vy 从 -1.0 逐帧 +0.15，越过 2.2 即恒 2.2。
    #[test]
    fn item_falls_with_gravity_clamped_at_terminal() {
        let mut w = crate::step::World::new(1);
        w.body
            .items
            .alloc(crate::items::ItemInit {
                x: Fx::ZERO,
                y: Fx::from_int(100),
                vx: Fx::ZERO,
                vy: Fx::from_int(-1),
                item_type: crate::items::ITEM_POWER,
                magnet_to: crate::items::MAGNET_NONE,
                timer: 0,
            })
            .unwrap();
        let mut prev_vy = w.body.items.vy[0];
        for f in 0..40u32 {
            crate::world::test_support::step_t(&mut w, &InputFrame::empty(f));
            let vy = w.body.items.vy[0];
            assert!(vy.raw() >= prev_vy.raw(), "重力单调");
            assert!(
                vy.raw() <= crate::tables::TABLES_V0.item_cfg[0].terminal_vy.raw(),
                "永不超终速"
            );
            prev_vy = vy;
        }
        assert_eq!(
            prev_vy,
            crate::tables::TABLES_V0.item_cfg[0].terminal_vy,
            "40 帧后必达终速"
        );
    }

    /// 近距磁吸：道具进磁吸圈（40px）即锁定并每帧重瞄直追；圈外不锁。
    ///
    /// **直调 `integrate()`（相位 5）而非全量 `step()`**：行 5 拾取落地后（Task 5），
    /// 拾取半径和 32 恰等于 attract_radius(40) − magnet_speed(8)，故任何本帧新锁定的道具，
    /// 追一步后必然落进拾取圈——用全量 `step()` 会在同一帧里把"锁定"和"拾取"叠在一起，
    /// 测不出本测试想孤立验证的纯粹磁吸触发/直追几何。直调本相位跳过 collide/settle，
    /// 与 `collide.rs`/`settle.rs` 里"设 phase_guard 后直调单相位函数"的先例同构。
    #[test]
    fn item_attracts_within_radius_only() {
        let mut w = crate::step::World::new(1);
        // 自机在 (0,384)；道具 A 在 (0, 350)（距 34 < 40）、B 在 (0, 300)（距 84 > 40）
        for y in [350, 300] {
            w.body
                .items
                .alloc(crate::items::ItemInit {
                    x: Fx::ZERO,
                    y: Fx::from_int(y),
                    vx: Fx::ZERO,
                    vy: Fx::ZERO,
                    item_type: crate::items::ITEM_POINT,
                    magnet_to: crate::items::MAGNET_NONE,
                    timer: 0,
                })
                .unwrap();
        }
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = crate::world::PH_INTEGRATE;
        }
        w.body.integrate(&crate::tables::TABLES_V0);
        assert_eq!(w.body.items.magnet_to[0], 0, "圈内锁定自机 0");
        assert_eq!(
            w.body.items.magnet_to[1],
            crate::items::MAGNET_NONE,
            "圈外不锁"
        );
        // 锁定后向自机推进（y 增大、速率 = 磁吸速度）
        let y0 = w.body.items.y[0];
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = crate::world::PH_INTEGRATE;
        }
        w.body.integrate(&crate::tables::TABLES_V0);
        assert!(w.body.items.y[0].raw() > y0.raw(), "朝自机（下方）追");
    }

    /// PoC：自机 y < 128 → 全场未锁定道具锁定；已拾取（0xFE）不受扰。
    #[test]
    fn poc_line_attracts_all_unlocked() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].y = Fx::from_int(100); // 过线
        for k in 0..3 {
            w.body
                .items
                .alloc(crate::items::ItemInit {
                    x: Fx::from_int(k * 50 - 50),
                    y: Fx::from_int(200),
                    vx: Fx::ZERO,
                    vy: Fx::ZERO,
                    item_type: crate::items::ITEM_POWER,
                    magnet_to: crate::items::MAGNET_NONE,
                    timer: 0,
                })
                .unwrap();
        }
        w.body.items.magnet_to[2] = crate::items::MAGNET_PICKED; // 已拾取哨兵
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(0));
        assert_eq!(w.body.items.magnet_to[0], 0);
        assert_eq!(w.body.items.magnet_to[1], 0);
        assert_eq!(
            w.body.items.magnet_to[2],
            crate::items::MAGNET_PICKED,
            "0xFE 不受扰"
        );
    }

    /// 解锁回落：磁吸中目标死亡 → MAGNET_NONE + vx 清零，重力接管。
    #[test]
    fn magnet_unlocks_when_target_dies() {
        let mut w = crate::step::World::new(1);
        w.body
            .items
            .alloc(crate::items::ItemInit {
                x: Fx::from_int(50),
                y: Fx::from_int(200),
                vx: Fx::ZERO,
                vy: Fx::ZERO,
                item_type: crate::items::ITEM_POWER,
                magnet_to: 0, // 已锁定自机 0
                timer: 0,
            })
            .unwrap();
        w.body.players[0].life_state = crate::player::LIFE_DEATHWINDOW; // 非 ALIVE
        w.body.players[0].state_timer = 8;
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(0));
        assert_eq!(w.body.items.magnet_to[0], crate::items::MAGNET_NONE, "解锁");
        assert_eq!(w.body.items.vx[0], Fx::ZERO, "垂直续落");
    }

    /// attract_all_items：合法目标全场上锁；坏索引/非 ALIVE → P4-b no-op + 计数。
    #[test]
    fn attract_all_items_api_contract() {
        let mut w = crate::step::World::new(1);
        w.body
            .items
            .alloc(crate::items::ItemInit {
                x: Fx::ZERO,
                y: Fx::from_int(50),
                vx: Fx::ZERO,
                vy: Fx::ZERO,
                item_type: crate::items::ITEM_POINT,
                magnet_to: crate::items::MAGNET_NONE,
                timer: 0,
            })
            .unwrap();
        w.body.attract_all_items(0);
        assert_eq!(w.body.items.magnet_to[0], 0);
        let cv0 = w.body.diag.contract_viol;
        w.body.attract_all_items(7); // 坏索引
        assert_eq!(w.body.diag.contract_viol, cv0 + 1);
    }

    /// 线性中点判别：dur=4 从 (0,100) 到 (80,180)——每帧走 1/4 路程，逐位相等。
    #[test]
    fn move_to_linear_waypoints_exact() {
        let mut w = crate::step::World::new(1);
        let h = spawn_enemy(&mut w, 0, 100, 5);
        w.body
            .move_enemy_to(h, Fx::from_int(80), Fx::from_int(180), 4, 0);
        let i = w.body.enemies.get(h).unwrap();
        for k in 1..=4i32 {
            crate::world::test_support::step_t(&mut w, &InputFrame::empty(k as u32));
            assert_eq!(
                w.body.enemies.x[i].raw(),
                (80 * 65536 * k) / 4,
                "第 {k} 帧 x"
            );
            assert_eq!(
                w.body.enemies.y[i].raw(),
                100 * 65536 + (80 * 65536 * k) / 4,
                "第 {k} 帧 y"
            );
        }
    }

    /// 到点即停：完成帧精确终点 + vx/vy 清零 + mv_active 清；此前残留速度不得泄漏。
    #[test]
    fn move_to_arrival_stops_dead() {
        let mut w = crate::step::World::new(1);
        let h = spawn_enemy(&mut w, 0, 100, 5);
        let i = w.body.enemies.get(h).unwrap();
        w.body.enemies.vx[i] = Fx::from_int(7); // 残留速度——插值期必须被无视、到点必须被清
        w.body
            .move_enemy_to(h, Fx::from_int(50), Fx::from_int(150), 3, 2); // QuadOut
        for f in 0..3u32 {
            crate::world::test_support::step_t(&mut w, &InputFrame::empty(f));
        }
        assert_eq!(w.body.enemies.x[i], Fx::from_int(50), "精确到点");
        assert_eq!(w.body.enemies.y[i], Fx::from_int(150));
        assert_eq!(
            w.body.enemies.vx[i],
            Fx::ZERO,
            "到点清速（未表达速度意图的路径；表达过的走 \
             arrival_preserves_velocity_when_script_expressed_intent_earlier）"
        );
        assert_eq!(w.body.enemies.mv_active[i], 0);
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(3));
        assert_eq!(w.body.enemies.x[i], Fx::from_int(50), "到点后不得漂移");
    }

    /// dur=0 瞬移（合法退化不计数）。
    #[test]
    fn move_to_zero_dur_teleports() {
        let mut w = crate::step::World::new(1);
        let h = spawn_enemy(&mut w, 0, 100, 5);
        let cv0 = w.body.diag.contract_viol;
        w.body
            .move_enemy_to(h, Fx::from_int(-30), Fx::from_int(40), 0, 0);
        let i = w.body.enemies.get(h).unwrap();
        assert_eq!(w.body.enemies.x[i], Fx::from_int(-30));
        assert_eq!(w.body.enemies.mv_active[i], 0, "瞬移不置插值态");
        assert_eq!(w.body.diag.contract_viol, cv0);
    }

    /// 瞬移必须硬停在飞插值：进行中 dur=0 瞬移后，旧轨迹不得次帧复活盖掉新位置。
    #[test]
    fn move_to_teleport_overrides_inflight_interpolation() {
        let mut w = crate::step::World::new(1);
        let h = spawn_enemy(&mut w, 0, 100, 5);
        w.body
            .move_enemy_to(h, Fx::from_int(100), Fx::from_int(100), 10, 0);
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(0)); // 在飞
        w.body
            .move_enemy_to(h, Fx::from_int(-80), Fx::from_int(30), 0, 0); // 瞬移
        let i = w.body.enemies.get(h).unwrap();
        assert_eq!(w.body.enemies.mv_active[i], 0, "瞬移清除在飞插值");
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(1));
        assert_eq!(w.body.enemies.x[i], Fx::from_int(-80), "旧轨迹不得复活");
        assert_eq!(w.body.enemies.y[i], Fx::from_int(30));
    }

    /// 进行中重下 = 覆盖重启（from 取当前位置）。
    #[test]
    fn move_to_reissue_restarts_from_current() {
        let mut w = crate::step::World::new(1);
        let h = spawn_enemy(&mut w, 0, 100, 5);
        w.body
            .move_enemy_to(h, Fx::from_int(100), Fx::from_int(100), 10, 0);
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(0)); // 走 1/10 → x=10
        let i = w.body.enemies.get(h).unwrap();
        let mid_x = w.body.enemies.x[i];
        w.body.move_enemy_to(h, Fx::ZERO, Fx::from_int(100), 2, 0); // 掉头回 x=0
        assert_eq!(w.body.enemies.mv_from_x[i], mid_x, "重启 from = 当前位置");
        for f in 1..=2u32 {
            crate::world::test_support::step_t(&mut w, &InputFrame::empty(f));
        }
        assert_eq!(w.body.enemies.x[i], Fx::ZERO, "2 帧回到 0");
    }

    /// 【招牌判别式】极坐标插值与笛卡尔插值**走的不是同一条路**。两者都从「朝右 5.0」
    /// 插到「朝下 5.0」（屏幕坐标 y 向下，故 QUARTER=90° 是朝下），取 t=0.5 那帧看速率：
    ///   极坐标 → 匀速扫弧，速率恒为 5.0
    ///   笛卡尔 → 直线穿过 (2.5, 2.5)，速率掉到 2.5·√2 ≈ 3.54
    /// 这一条同时逮住两个错法：笛卡尔实现写成了极坐标；move_vel_xy 被做成 move_vel 的糖。
    #[test]
    fn polar_and_cartesian_velocity_interpolation_take_different_paths() {
        // —— 极坐标腿：速率全程恒定 ——
        let mut w = crate::step::World::new(1);
        let h = spawn_enemy(&mut w, 0, 0, 5);
        w.body
            .set_enemy_vel_polar(h, Angle::ZERO, Fx::from_int(5), 0, 0);
        w.body
            .set_enemy_vel_polar(h, Angle::QUARTER, Fx::from_int(5), 4, 0); // Linear
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(0));
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(1)); // t = 2/4 = 0.5
        let i = w.body.enemies.get(h).unwrap();
        assert_eq!(
            w.body.enemies.speed[i],
            Fx::from_int(5),
            "极坐标插值：速率是被直接插的量，5.0→5.0 全程恒定"
        );

        // —— 笛卡尔腿：同样两端，中途速率必须掉下来 ——
        let mut w2 = crate::step::World::new(1);
        let h2 = spawn_enemy(&mut w2, 0, 0, 5);
        w2.body
            .set_enemy_vel_cart(h2, Fx::from_int(5), Fx::ZERO, 0, 0);
        w2.body
            .set_enemy_vel_cart(h2, Fx::ZERO, Fx::from_int(5), 4, 0); // Linear
        crate::world::test_support::step_t(&mut w2, &InputFrame::empty(0));
        crate::world::test_support::step_t(&mut w2, &InputFrame::empty(1));
        let i2 = w2.body.enemies.get(h2).unwrap();
        // 分量是精确的线性中点
        assert_eq!(
            w2.body.enemies.vx[i2],
            Fx::from_raw(163840),
            "5.0 的一半 = 2.5"
        );
        assert_eq!(w2.body.enemies.vy[i2], Fx::from_raw(163840));
        // 回填出来的速率落在 3.5~3.6（2.5·√2 = 3.5355；isqrt 舍入留余量）
        let sp = w2.body.enemies.speed[i2];
        assert!(
            sp > Fx::from_raw(229376) && sp < Fx::from_raw(235930),
            "笛卡尔插值中点速率应约 3.54，实得 {sp:?}——若这里是 5.0 说明走了极坐标空间"
        );
    }

    /// 最短弧：350° → 10° 应走 **+20°**（顺时针跨 0° 缝），而不是 −340°。
    /// 取 dur=2、Linear，中点应落在 0°（即 360°）附近而非 180° 那边。
    #[test]
    fn angle_interpolation_takes_shortest_arc_across_the_seam() {
        let mut w = crate::step::World::new(1);
        let h = spawn_enemy(&mut w, 0, 0, 5);
        let a350 = Angle(63715); // 350° ≈ 65536*350/360
        let a10 = Angle(1820); //  10° ≈ 65536*10/360
        w.body.set_enemy_vel_polar(h, a350, Fx::from_int(3), 0, 0);
        w.body.set_enemy_vel_polar(h, a10, Fx::from_int(3), 2, 0);
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(0)); // t = 1/2
        let i = w.body.enemies.get(h).unwrap();
        let mid = w.body.enemies.angle[i].raw();
        // 中点应在缝上（接近 0 或接近 65536），绝不在 180° 附近
        assert!(
            !(1500..=64000).contains(&mid),
            "中点应落在 0° 缝附近，实得 {mid}——若在 32768 附近说明走了长弧"
        );
    }

    /// 仲裁腿 (a)：move_to 单独 → 到点**仍清速**（守住原契约，一字不变）。
    #[test]
    fn arrival_still_clears_velocity_when_script_never_touched_it() {
        let mut w = crate::step::World::new(1);
        let h = spawn_enemy(&mut w, 0, 100, 5);
        let i = w.body.enemies.get(h).unwrap();
        w.body.enemies.vx[i] = Fx::from_int(7); // 残留速度
        w.body
            .move_enemy_to(h, Fx::from_int(80), Fx::from_int(180), 2, 0);
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(0));
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(1)); // 到点
        assert_eq!(w.body.enemies.mv_active[i], 0, "已到点");
        assert_eq!(w.body.enemies.vx[i], Fx::ZERO, "未表达速度意图 ⇒ 到点清速");
    }

    /// 仲裁腿 (b)：move_to 途中调速度动词，且**速度插值先于位置插值到期**
    /// → 到点**不清速**、速度立刻接管。
    /// 这条正是「拿 vel_active 当判据」会漏掉的那格——速度那条 dur=2 在第 2 帧就结束、
    /// vel_active 归 0，而位置那条 dur=4 到第 4 帧才到点。必须让 dur 严格不等。
    #[test]
    fn arrival_preserves_velocity_when_script_expressed_intent_earlier() {
        let mut w = crate::step::World::new(1);
        let h = spawn_enemy(&mut w, 0, 100, 5);
        w.body
            .move_enemy_to(h, Fx::from_int(80), Fx::from_int(180), 4, 0);
        w.body
            .set_enemy_vel_polar(h, Angle::QUARTER, Fx::from_int(3), 2, 0); // 先到期
        for k in 0..4 {
            crate::world::test_support::step_t(&mut w, &InputFrame::empty(k));
        }
        let i = w.body.enemies.get(h).unwrap();
        assert_eq!(w.body.enemies.mv_active[i], 0, "位置插值已到点");
        assert_eq!(w.body.enemies.vel_active[i], 0, "速度插值早已到期");
        assert_eq!(
            w.body.enemies.vy[i],
            Fx::from_int(3),
            "到点不得清速——速度意图表达在先，落地即接管"
        );
    }

    /// 仲裁腿 (c)：速度动词在前、move_to 在后 → 到点**清速**（武装时 vel_touched 归零）。
    /// 缺了这条的话，「move_enemy_to 忘了清 vel_touched」这个错法照样绿。
    #[test]
    fn move_to_rearm_resets_touched_so_arrival_clears_again() {
        let mut w = crate::step::World::new(1);
        let h = spawn_enemy(&mut w, 0, 100, 5);
        w.body
            .set_enemy_vel_polar(h, Angle::QUARTER, Fx::from_int(3), 0, 0); // 先设速度
        w.body
            .move_enemy_to(h, Fx::from_int(80), Fx::from_int(180), 2, 0); // 再 move_to
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(0));
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(1));
        let i = w.body.enemies.get(h).unwrap();
        assert_eq!(
            w.body.enemies.vy[i],
            Fx::ZERO,
            "move_to 武装即归零 vel_touched ⇒ 到点照清"
        );
    }

    /// 死代码通电的正面证据：move_vel 之后，位置纯靠 `x += vx` 推进。
    /// 这条分支此前永远在加零（没有任何 syscall 能写 vx/vy），世界层测试绿的是够不着的代码。
    #[test]
    fn uniform_velocity_actually_moves_the_enemy_now() {
        let mut w = crate::step::World::new(1);
        let h = spawn_enemy(&mut w, 0, 0, 5);
        w.body
            .set_enemy_vel_polar(h, Angle::QUARTER, Fx::from_int(2), 0, 0);
        let i = w.body.enemies.get(h).unwrap();
        let y0 = w.body.enemies.y[i];
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(0));
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(1));
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(2));
        assert_eq!(
            w.body.enemies.y[i],
            y0 + Fx::from_int(6),
            "3 帧 × 2.0/帧 = 6.0"
        );
    }

    /// 速度插值到期写**精确终值**（不吃插值舍入）并清 vel_active。
    #[test]
    fn velocity_interpolation_lands_on_exact_target_and_disarms() {
        let mut w = crate::step::World::new(1);
        let h = spawn_enemy(&mut w, 0, 0, 5);
        w.body
            .set_enemy_vel_polar(h, Angle::ZERO, Fx::from_int(1), 0, 0);
        w.body
            .set_enemy_vel_polar(h, Angle::QUARTER, Fx::from_int(5), 3, 4); // CubicIn
        for k in 0..3 {
            crate::world::test_support::step_t(&mut w, &InputFrame::empty(k));
        }
        let i = w.body.enemies.get(h).unwrap();
        assert_eq!(w.body.enemies.speed[i], Fx::from_int(5), "终帧精确终值");
        assert_eq!(w.body.enemies.angle[i], Angle::QUARTER);
        assert_eq!(w.body.enemies.vel_active[i], 0, "到期即解除武装");
    }

    /// 上一条的**笛卡尔孪生**：终帧写精确终值 + 解除武装，在笛卡尔路径上同样成立。
    ///
    /// 单独一条而不是把招牌判别式跑完——招牌判别式只要中点（`dur=4` 只走 2 帧），
    /// 复审实测：对调笛卡尔 `done` 分支的 `t0`/`t1`（vx 写成 vy 的终值）全仓 570 条**全绿**，
    /// spec §6.2 的「终帧写精确终值」在笛卡尔腿上是无守卫的存活变异。
    ///
    /// **两个分量必须取不相等的值**（2.0 / 6.0）：取相等值的话 `t0`/`t1` 对调仍不可辨，
    /// 等于补了个假守卫。
    #[test]
    fn cartesian_velocity_interpolation_lands_on_exact_target_and_disarms() {
        let mut w = crate::step::World::new(1);
        let h = spawn_enemy(&mut w, 0, 0, 5);
        w.body
            .set_enemy_vel_cart(h, Fx::from_int(-4), Fx::from_int(1), 0, 0);
        w.body
            .set_enemy_vel_cart(h, Fx::from_int(2), Fx::from_int(6), 3, 4); // CubicIn
        for k in 0..3 {
            crate::world::test_support::step_t(&mut w, &InputFrame::empty(k));
        }
        let i = w.body.enemies.get(h).unwrap();
        assert_eq!(w.body.enemies.vx[i], Fx::from_int(2), "终帧精确终值 vx");
        assert_eq!(w.body.enemies.vy[i], Fx::from_int(6), "终帧精确终值 vy");
        assert_eq!(w.body.enemies.vel_active[i], 0, "到期即解除武装");
    }

    /// P4：悬垂句柄计数；easing≥8 拒绝 no-op。
    #[test]
    fn move_to_bad_args_contract() {
        let mut w = crate::step::World::new(1);
        let h = spawn_enemy(&mut w, 0, 100, 5);
        w.body.enemies.free(h);
        let cv0 = w.body.diag.contract_viol;
        w.body.move_enemy_to(h, Fx::ZERO, Fx::ZERO, 10, 0);
        assert_eq!(w.body.diag.contract_viol, cv0 + 1);
        assert_eq!(w.body.last_status, crate::world::STATUS_STALE_HANDLE);
        let h2 = spawn_enemy(&mut w, 0, 100, 5);
        let i2 = w.body.enemies.get(h2).unwrap();
        w.body.move_enemy_to(h2, Fx::ZERO, Fx::ZERO, 10, 8); // easing 越界
        assert_eq!(w.body.diag.contract_viol, cv0 + 2);
        assert_eq!(w.body.last_status, crate::world::STATUS_BAD_ARGS);
        assert_eq!(w.body.enemies.mv_active[i2], 0, "拒绝即 no-op");
    }
}
