//! 激光写 API（P1：调用方只走这里，不直接碰池内存）。
use super::{POOL_LASER, STATUS_POOL_FULL, STATUS_STALE_HANDLE, WorldBody};
use crate::enemy::EnemyHandle;
use crate::lasers::*;
use crate::math::{Angle, Fx};

impl WorldBody {
    /// 建一条激光。P4-a：池满 → NULL + 计数。P4-b：width/start/end/start_len/speed 为负 → 钳到 0 并计一次 contract_viol。
    /// 调用方给几何、外观、时长和 omega，其余字段由本函数定：state 按 warn 是否为 0、timer 0、不挂靠、
    /// 观测字段清零、px/py/pang = 初值、born_frame = 当前帧。
    pub fn create_laser(&mut self, mut init: LaserInit) -> LaserHandle {
        let mut bad = false;
        for v in [
            &mut init.width,
            &mut init.start,
            &mut init.end,
            &mut init.start_len,
            &mut init.speed,
        ] {
            if *v < Fx::ZERO {
                *v = Fx::ZERO;
                bad = true;
            }
        }
        if bad {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
        }
        init.state = if init.warn == 0 {
            LASER_ACTIVE
        } else {
            LASER_WARN
        };
        init.timer = 0;
        init.anchor_idx = ANCHOR_NONE;
        init.anchor_gen = 0;
        init.dx = Fx::ZERO;
        init.dy = Fx::ZERO;
        init.dang = 0;
        init.px = init.ox;
        init.py = init.oy;
        init.pang = init.angle;
        init.born_frame = self.frame;
        match self.lasers.alloc(init) {
            Some(h) => h,
            None => {
                self.diag.pool_full[POOL_LASER] = self.diag.pool_full[POOL_LASER].wrapping_add(1);
                self.last_status = STATUS_POOL_FULL;
                LaserHandle::NULL
            }
        }
    }

    /// 句柄查验（P4-b）：悬垂 / 代际不符 → None + `contract_viol` 一次 + `STALE_HANDLE`。
    /// 所有写 API 统一先过这里；`laser_alive` 只读，不走。
    fn laser_slot(&mut self, h: LaserHandle) -> Option<usize> {
        if self.lasers.get(h).is_some() {
            Some(h.index as usize)
        } else {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            self.last_status = STATUS_STALE_HANDLE;
            None
        }
    }

    /// 形态二：设速率与近端长度，并从近端长出去（`end = start`）。负值钳 0，一次调用多坏字段只计一次违约。
    pub fn laser_set_speed(&mut self, h: LaserHandle, speed: Fx, start_len: Fx) -> bool {
        let Some(i) = self.laser_slot(h) else {
            return false;
        };
        let mut s = speed;
        let mut len = start_len;
        let mut bad = false;
        if s < Fx::ZERO {
            s = Fx::ZERO;
            bad = true;
        }
        if len < Fx::ZERO {
            len = Fx::ZERO;
            bad = true;
        }
        if bad {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
        }
        let l = &mut self.lasers;
        l.speed[i] = s;
        l.start_len[i] = len;
        l.end[i] = l.start[i];
        true
    }

    /// 近端留空（原作第 4 关 `start = 64`）。负值钳 0 并计一次违约。
    pub fn laser_set_start(&mut self, h: LaserHandle, s: Fx) -> bool {
        let Some(i) = self.laser_slot(h) else {
            return false;
        };
        let mut v = s;
        if v < Fx::ZERO {
            v = Fx::ZERO;
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
        }
        self.lasers.start[i] = v;
        true
    }

    /// 持续转动速率（BAM/帧）。
    pub fn laser_set_omega(&mut self, h: LaserHandle, omega: i16) -> bool {
        let Some(i) = self.laser_slot(h) else {
            return false;
        };
        self.lasers.omega[i] = omega;
        true
    }

    /// 一次性转一个角度（原作 88；回绕加）。
    pub fn laser_rotate(&mut self, h: LaserHandle, a: Angle) -> bool {
        let Some(i) = self.laser_slot(h) else {
            return false;
        };
        self.lasers.angle[i] = self.lasers.angle[i].add(a);
        true
    }

    /// 指向自机 0 的角度 + 偏移 `off`（原作 89）。
    pub fn laser_aim(&mut self, h: LaserHandle, off: Angle) -> bool {
        let Some(i) = self.laser_slot(h) else {
            return false;
        };
        let (px, py) = (self.players[0].x, self.players[0].y);
        let to = crate::math::cordic::atan2(py - self.lasers.oy[i], px - self.lasers.ox[i]);
        self.lasers.angle[i] = to.add(off);
        true
    }

    /// 挂到敌人身上（存代际句柄；相位 5 代际相符才跟）。`e == NULL` 表示解除挂靠。
    pub fn laser_anchor(&mut self, h: LaserHandle, e: EnemyHandle, ax: Fx, ay: Fx) -> bool {
        let Some(i) = self.laser_slot(h) else {
            return false;
        };
        let l = &mut self.lasers;
        if e == EnemyHandle::NULL {
            l.anchor_idx[i] = ANCHOR_NONE;
            l.anchor_gen[i] = 0;
            l.ax[i] = Fx::ZERO;
            l.ay[i] = Fx::ZERO;
        } else {
            l.anchor_idx[i] = e.index;
            l.anchor_gen[i] = e.generation;
            l.ax[i] = ax;
            l.ay[i] = ay;
        }
        true
    }

    /// 直接设置原点，并解除挂靠。
    pub fn laser_origin(&mut self, h: LaserHandle, x: Fx, y: Fx) -> bool {
        let Some(i) = self.laser_slot(h) else {
            return false;
        };
        let l = &mut self.lasers;
        l.ox[i] = x;
        l.oy[i] = y;
        l.anchor_idx[i] = ANCHOR_NONE;
        l.anchor_gen[i] = 0;
        l.ax[i] = Fx::ZERO;
        l.ay[i] = Fx::ZERO;
        true
    }

    /// 取消：`state < 2 → 2`、`timer = 0`（已收缩则 no-op，但仍返回 true）。
    pub fn laser_cancel(&mut self, h: LaserHandle) -> bool {
        let Some(i) = self.laser_slot(h) else {
            return false;
        };
        self.cancel_laser_index(i);
        true
    }

    /// 是否存活（只读，不计数）。
    pub fn laser_alive(&self, h: LaserHandle) -> bool {
        self.lasers.get(h).is_some()
    }

    /// 按索引取消（`pub(crate)`，Task 3 的行 10 用）：`state < 2 → 2`、`timer = 0`。
    pub(crate) fn cancel_laser_index(&mut self, i: usize) {
        if self.lasers.state[i] < LASER_FADE {
            self.lasers.state[i] = LASER_FADE;
            self.lasers.timer[i] = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::input::InputFrame;
    use crate::lasers::{
        ANCHOR_NONE, LASER_ACTIVE, LASER_FADE, LASER_WARN, LaserHandle, LaserInit, LaserPool,
    };
    use crate::math::{Angle, Fx};
    use crate::step::World;
    use crate::world::{POOL_LASER, STATUS_POOL_FULL};

    /// 一条可用的激光初始值（`warn` 决定出生状态）。
    fn init(warn: u16) -> LaserInit {
        LaserInit {
            ox: Fx::from_int(10),
            oy: Fx::from_int(20),
            angle: Angle::QUARTER,
            omega: 0,
            start: Fx::ZERO,
            end: Fx::from_int(100),
            start_len: Fx::from_int(100),
            speed: Fx::ZERO,
            width: Fx::from_int(16),
            sprite: 0,
            warn,
            active: 60,
            fade: 0,
            timer: 0,
            state: 0,
            anchor_idx: 0,
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

    /// `warn == 0` 出生即 state 1；`warn != 0` 出生 state 0。顺带钉死 `create_laser` 的
    /// 派生字段：timer 0、不挂靠、观测清零、px/py/pang = 初值、born_frame = 当前帧。
    #[test]
    fn warn_zero_spawns_active_else_warn() {
        let mut w = World::new(1);
        w.body.frame = 7;
        let ha = w.body.create_laser(init(0));
        let p = w.body.view().lasers();
        let ia = p.get(ha).unwrap();
        assert_eq!(p.state[ia], LASER_ACTIVE, "warn==0 出生即生效");
        assert_eq!(p.timer[ia], 0);
        assert_eq!(p.anchor_idx[ia], ANCHOR_NONE, "出生不挂靠");
        assert_eq!(p.anchor_gen[ia], 0);
        assert_eq!(p.px[ia], Fx::from_int(10), "px = ox");
        assert_eq!(p.py[ia], Fx::from_int(20), "py = oy");
        assert_eq!(p.pang[ia], Angle::QUARTER, "pang = angle");
        assert_eq!(p.born_frame[ia], 7, "born_frame = 当前帧");

        let hw = w.body.create_laser(init(30));
        let iw = w.body.lasers.get(hw).unwrap();
        assert_eq!(w.body.lasers.state[iw], LASER_WARN);
    }

    /// P4-a：池满第 257 条返回 NULL，`pool_full[POOL_LASER]` 加 1，last_status = POOL_FULL。
    #[test]
    fn pool_full_returns_null_and_counts() {
        let mut w = World::new(1);
        for k in 0..LaserPool::CAP {
            assert_ne!(
                w.body.create_laser(init(0)),
                LaserHandle::NULL,
                "第 {k} 条应成功"
            );
        }
        let h = w.body.create_laser(init(0));
        assert_eq!(h, LaserHandle::NULL, "池满 → NULL");
        assert_eq!(w.body.view().diag().pool_full[POOL_LASER], 1);
        assert_eq!(w.body.view().last_status(), STATUS_POOL_FULL);
    }

    /// P4-b：width/start/end/start_len/speed 为负 → 逐个钳到 0，`contract_viol` 只加 1
    /// （一次调用多个坏字段不重复计数）。
    #[test]
    fn negative_params_clamped_and_counted_once() {
        let mut w = World::new(1);
        let mut it = init(0);
        it.width = Fx::from_int(-5);
        it.start = Fx::from_int(-1);
        it.end = Fx::from_int(-2);
        it.start_len = Fx::from_int(-3);
        it.speed = Fx::from_int(-4);
        let h = w.body.create_laser(it);
        let i = w.body.lasers.get(h).unwrap();
        let p = &w.body.lasers;
        assert_eq!(p.width[i], Fx::ZERO);
        assert_eq!(p.start[i], Fx::ZERO);
        assert_eq!(p.end[i], Fx::ZERO);
        assert_eq!(p.start_len[i], Fx::ZERO);
        assert_eq!(p.speed[i], Fx::ZERO);
        assert_eq!(w.body.view().diag().contract_viol, 1, "多坏字段只计一次");
    }

    /// 快照往返 + 校验和敏感（照 `step.rs::snapshot_covers_item_pool` 写法）：
    /// `copy_into` 漏拷 `lasers` 即红，字段不入校验和也红。
    #[test]
    fn snapshot_covers_laser_pool() {
        let mut w = World::new(3);
        w.body.create_laser(init(0));
        let ck = w.checksum();
        let mut snap = World::new(3);
        w.copy_into(&mut snap);
        assert_eq!(snap.checksum(), ck, "快照必须带 lasers 全部字节");
        snap.body.lasers.width[0] = snap.body.lasers.width[0] - Fx::ONE;
        assert_ne!(snap.checksum(), ck, "lasers 必须真的参与校验和");
    }

    // ── Task 2：相位 5 推进 / 写 API ─────────────────────────────────────

    fn step(w: &mut crate::step::World, f: u32) {
        crate::world::test_support::step_t(w, &InputFrame::empty(f));
    }

    /// 原点 (0,100)，朝下（BAM 16384），形态一：start 0、end = start_len = len、speed 0。
    fn laser_init(warn: u16, active: u16, fade: u16, len: i32, width: i32) -> LaserInit {
        LaserInit {
            ox: Fx::ZERO,
            oy: Fx::from_int(100),
            angle: Angle(16384),
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
            pang: Angle(0),
            flags: 0,
            born_frame: 0,
        }
    }

    /// 逐帧跑 `frames` 步，断言每步之后的 state，返回下一个帧号。
    fn run_state(w: &mut crate::step::World, i: usize, f0: u32, frames: u32, want: u8) -> u32 {
        for f in f0..f0 + frames {
            step(w, f);
            assert_eq!(w.body.lasers.state[i], want, "帧 {f}");
        }
        f0 + frames
    }

    /// s1 Sub12（宽 32 → 16）：预警 30、生效 120、收缩 16，speed 0，end 恒为 500。
    #[test]
    fn sub12_warn_then_active_then_fade_frame_exact() {
        let mut w = World::new(1);
        let h = w.body.create_laser(laser_init(30, 120, 16, 500, 16));
        let i = h.index as usize;
        let f = run_state(&mut w, i, 0, 30, LASER_WARN);
        let f = run_state(&mut w, i, f, 120, LASER_ACTIVE);
        let f = run_state(&mut w, i, f, 16, LASER_FADE);
        assert_eq!(
            (w.body.lasers.start[i], w.body.lasers.end[i]),
            (Fx::ZERO, Fx::from_int(500))
        );
        step(&mut w, f);
        assert!(!w.body.laser_alive(h), "收缩 16 帧后回收");
    }

    /// s1 Sub22（宽 16 → 8）：预警 120、生效 60、收缩 16。
    #[test]
    fn sub22_long_warning() {
        let mut w = World::new(1);
        let h = w.body.create_laser(laser_init(120, 60, 16, 500, 8));
        let i = h.index as usize;
        let f = run_state(&mut w, i, 0, 120, LASER_WARN);
        let f = run_state(&mut w, i, f, 60, LASER_ACTIVE);
        run_state(&mut w, i, f, 16, LASER_FADE);
    }

    /// s2 Sub27（宽 6 → 3）：warn 0 出生即生效；speed 4、start_len 192。
    /// 第 f 步后 end = 4(f+1)，start = max(0, end − 192)；start 到 640（end 832）的那一步回收，即 f = 207。
    #[test]
    fn sub27_sliding_bar_until_cull() {
        let mut w = World::new(1);
        let h = w.body.create_laser(laser_init(0, 9999, 30, 0, 3));
        assert!(
            w.body
                .laser_set_speed(h, Fx::from_int(4), Fx::from_int(192))
        );
        let i = h.index as usize;
        assert_eq!(w.body.lasers.state[i], LASER_ACTIVE, "warn 0 出生即生效");
        for f in 0..207u32 {
            step(&mut w, f);
            let end = 4 * (f as i32 + 1);
            assert_eq!(w.body.lasers.end[i], Fx::from_int(end), "帧 {f}");
            assert_eq!(
                w.body.lasers.start[i],
                Fx::from_int((end - 192).max(0)),
                "帧 {f}"
            );
            assert_eq!(w.body.lasers.state[i], LASER_ACTIVE, "帧 {f}");
        }
        step(&mut w, 207);
        assert!(!w.body.laser_alive(h), "start 到 640 回收");
    }

    /// omega 与一次性 rotate 叠加后的 dang：omega=100，某帧 rotate 1000 →
    /// 该帧 dang == 1100，其余帧 == 100（rotate 只作用一帧）。
    #[test]
    fn omega_and_rotate_both_land_in_dang() {
        let mut w = World::new(1);
        let h = w.body.create_laser(laser_init(0, 9999, 0, 500, 16));
        assert!(w.body.laser_set_omega(h, 100));
        let i = h.index as usize;
        step(&mut w, 0);
        assert_eq!(w.body.lasers.dang[i], 100, "omega 单帧增量");
        assert!(w.body.laser_rotate(h, Angle(1000)));
        step(&mut w, 1);
        assert_eq!(
            w.body.lasers.dang[i], 1100,
            "omega + 一次性 rotate 同帧叠加"
        );
        step(&mut w, 2);
        assert_eq!(w.body.lasers.dang[i], 100, "rotate 只作用一帧");
    }

    /// 挂靠跟随敌人（相位 5 在敌人之后读本帧新位置）；死亡回收后脱钩、原点留在原地。
    #[test]
    fn anchor_follows_enemy_then_detaches_on_death() {
        use crate::world::test_support::spawn_enemy;
        let mut w = World::new(1);
        let e = spawn_enemy(&mut w, 0, 100, 5);
        let ei = w.body.enemies.get(e).unwrap();
        w.body.enemies.vx[ei] = Fx::from_int(2);
        let h = w.body.create_laser(laser_init(0, 9999, 0, 500, 16));
        assert!(w.body.laser_anchor(h, e, Fx::ZERO, Fx::from_int(8)));
        let i = h.index as usize;

        step(&mut w, 0);
        assert_eq!(w.body.lasers.ox[i], Fx::from_int(2), "跟随本帧敌人新 x");
        assert_eq!(w.body.lasers.oy[i], Fx::from_int(108), "y + 偏移 8");
        assert_eq!(w.body.lasers.dx[i], Fx::from_int(2), "dx = 敌人本帧位移");

        step(&mut w, 1);
        assert_eq!(w.body.lasers.ox[i], Fx::from_int(4));

        // 杀死：杀死帧敌人仍活（cleanup 才收尸），激光最后跟一帧；下一帧敌人已回收 → 脱钩。
        w.body.kill_enemy_by_handle(e, &crate::tables::TABLES_V0);
        step(&mut w, 2);
        let ox_last = w.body.lasers.ox[i];
        assert_eq!(ox_last, Fx::from_int(6), "死亡帧仍跟到最后位置");
        assert_eq!(
            w.body.lasers.anchor_idx[i], e.index,
            "回收帧仍挂着（脱钩推迟到下一帧相位 5）"
        );
        step(&mut w, 3);
        assert_eq!(w.body.lasers.anchor_idx[i], ANCHOR_NONE, "敌人回收后脱钩");
        assert_eq!(w.body.lasers.ox[i], ox_last, "脱钩后原点不动");
        assert_eq!(w.body.lasers.dx[i], Fx::ZERO, "无位移 → dx = 0");
    }

    /// Review Focus 1：死亡回收的槽被新敌复用（代际不符）→ 脱钩，原点不跳到新敌。
    #[test]
    fn anchor_does_not_jump_to_recycled_slot() {
        use crate::world::test_support::spawn_enemy;
        let mut w = World::new(1);
        let e1 = spawn_enemy(&mut w, 0, 100, 5);
        let ei = w.body.enemies.get(e1).unwrap();
        w.body.enemies.vx[ei] = Fx::from_int(2);
        let gen_before = w.body.enemies.generation_of(ei);
        let h = w.body.create_laser(laser_init(0, 9999, 0, 500, 16));
        assert!(w.body.laser_anchor(h, e1, Fx::ZERO, Fx::from_int(8)));
        let i = h.index as usize;
        step(&mut w, 0);

        // 杀死 → step 一帧让 cleanup 回收（激光跟到敌人最后一帧位置）。
        w.body.kill_enemy_by_handle(e1, &crate::tables::TABLES_V0);
        step(&mut w, 1);
        let ox_at_recycle = w.body.lasers.ox[i];
        assert_eq!(
            w.body.lasers.anchor_idx[i], e1.index,
            "前提：回收帧仍挂着旧下标（脱钩在下一帧）"
        );

        // 最低空位复用同一下标，代际前进。
        let e2 = spawn_enemy(&mut w, 500, 100, 5);
        assert_eq!(e2.index, e1.index, "create_enemy 取最低空位，必然复用");
        assert_ne!(
            w.body.enemies.generation_of(ei),
            gen_before,
            "复用槽代际必须前进"
        );

        step(&mut w, 2);
        assert_eq!(w.body.lasers.anchor_idx[i], ANCHOR_NONE, "代际不符 → 脱钩");
        assert_eq!(w.body.lasers.ox[i], ox_at_recycle, "原点不跳到新敌位置");
        assert_ne!(w.body.lasers.ox[i], Fx::from_int(500));
    }

    /// `laser_anchor` 传 `EnemyHandle::NULL` 表示解除挂靠（原点不动）。
    #[test]
    fn anchor_null_detaches() {
        use crate::enemy::EnemyHandle;
        use crate::world::test_support::spawn_enemy;
        let mut w = World::new(1);
        let e = spawn_enemy(&mut w, 30, 100, 5);
        let h = w.body.create_laser(laser_init(0, 9999, 0, 500, 16));
        assert!(w.body.laser_anchor(h, e, Fx::from_int(1), Fx::from_int(2)));
        let i = h.index as usize;
        assert_ne!(w.body.lasers.anchor_idx[i], ANCHOR_NONE);
        let (ox, oy) = (w.body.lasers.ox[i], w.body.lasers.oy[i]);

        assert!(
            w.body
                .laser_anchor(h, EnemyHandle::NULL, Fx::ZERO, Fx::ZERO)
        );
        assert_eq!(w.body.lasers.anchor_idx[i], ANCHOR_NONE, "NULL 解除挂靠");
        assert_eq!(
            (w.body.lasers.ox[i], w.body.lasers.oy[i]),
            (ox, oy),
            "解除挂靠不动原点"
        );
    }

    /// `laser_cancel`：`state < 2 → 2`、`timer = 0`；已收缩则 no-op（timer 保持）。
    #[test]
    fn cancel_switches_to_fade_and_resets_timer() {
        let mut w = World::new(1);
        let h = w.body.create_laser(laser_init(30, 120, 16, 500, 16));
        let i = h.index as usize;
        for f in 0..3 {
            step(&mut w, f); // 让 timer 非零
        }
        assert_eq!(w.body.lasers.state[i], LASER_WARN);
        assert_ne!(w.body.lasers.timer[i], 0);
        assert!(w.body.laser_cancel(h));
        assert_eq!(w.body.lasers.state[i], LASER_FADE, "state < 2 → 2");
        assert_eq!(w.body.lasers.timer[i], 0, "取消时 timer 归零");

        w.body.lasers.timer[i] = 7;
        assert!(w.body.laser_cancel(h));
        assert_eq!(w.body.lasers.timer[i], 7, "已收缩态再取消不改 timer");
    }

    /// `laser_origin` 直接设原点并解除挂靠；`laser_set_start` 写近端留空。
    #[test]
    fn origin_sets_position_and_detaches_anchor() {
        use crate::world::test_support::spawn_enemy;
        let mut w = World::new(1);
        let e = spawn_enemy(&mut w, 30, 100, 5);
        let h = w.body.create_laser(laser_init(0, 9999, 0, 500, 16));
        assert!(w.body.laser_anchor(h, e, Fx::from_int(1), Fx::from_int(2)));
        let i = h.index as usize;
        assert_ne!(w.body.lasers.anchor_idx[i], ANCHOR_NONE);

        assert!(w.body.laser_origin(h, Fx::from_int(7), Fx::from_int(9)));
        assert_eq!(w.body.lasers.anchor_idx[i], ANCHOR_NONE, "origin 解除挂靠");
        assert_eq!(w.body.lasers.ox[i], Fx::from_int(7));
        assert_eq!(w.body.lasers.oy[i], Fx::from_int(9));

        assert!(w.body.laser_set_start(h, Fx::from_int(64)));
        assert_eq!(w.body.lasers.start[i], Fx::from_int(64));
        // set_speed 令 end = start（不是 0）：start 已置 64 时必须从 64 长出去。
        assert!(
            w.body
                .laser_set_speed(h, Fx::from_int(4), Fx::from_int(192))
        );
        assert_eq!(
            w.body.lasers.end[i],
            Fx::from_int(64),
            "end = start，不是 0"
        );
    }

    /// `laser_aim`：角度 = 指向自机 0 的 atan2 + 偏移。
    #[test]
    fn aim_points_at_player_zero_plus_offset() {
        let mut w = World::new(1);
        let h = w.body.create_laser(laser_init(0, 9999, 0, 500, 16));
        let i = h.index as usize;
        assert!(w.body.laser_aim(h, Angle(1000)));
        let want = crate::math::cordic::atan2(
            w.body.players[0].y - Fx::from_int(100),
            w.body.players[0].x - Fx::ZERO,
        )
        .add(Angle(1000));
        assert_eq!(w.body.lasers.angle[i], want);
    }

    /// Review Focus 5：时停期间不推进、观测清零（照 integrate.rs 敌人 dx 时停测试冻结场景）。
    #[test]
    fn frozen_scene_does_not_advance() {
        let mut w = World::new(1);
        let h = w.body.create_laser(laser_init(30, 120, 16, 500, 16));
        assert!(
            w.body
                .laser_set_speed(h, Fx::from_int(4), Fx::from_int(192))
        );
        assert!(w.body.laser_set_omega(h, 100));
        // 先移一次原点，让观测字段在非时停帧确实非零（否则"清零"断言无判别力）。
        assert!(w.body.laser_origin(h, Fx::from_int(10), Fx::ZERO));
        let i = h.index as usize;
        step(&mut w, 0);
        assert_ne!(w.body.lasers.dx[i], Fx::ZERO, "前提：非时停帧确实在动");
        assert_ne!(w.body.lasers.dang[i], 0, "前提：非时停帧确实在转");

        let (end0, timer0, state0) = (
            w.body.lasers.end[i],
            w.body.lasers.timer[i],
            w.body.lasers.state[i],
        );
        // 照 integrate.rs 敌人 dx 时停测试：freeze_left[0] > 0 = 冻 C（场景）。
        w.body.freeze_left = [5, 0];
        step(&mut w, 1);
        assert_eq!(w.body.lasers.end[i], end0, "时停期间 end 不推进");
        assert_eq!(w.body.lasers.timer[i], timer0, "时停期间 timer 不推进");
        assert_eq!(w.body.lasers.state[i], state0, "时停期间 state 不变");
        assert_eq!(w.body.lasers.dx[i], Fx::ZERO, "时停帧 dx 报 0");
        assert_eq!(w.body.lasers.dy[i], Fx::ZERO, "时停帧 dy 报 0");
        assert_eq!(w.body.lasers.dang[i], 0, "时停帧 dang 报 0");
    }

    /// P4-b：回收后所有写 API no-op + 每次计一次违约；`laser_alive` 只读不计数。
    #[test]
    fn stale_handle_setters_are_noop_and_counted() {
        use crate::enemy::EnemyHandle;
        let mut w = World::new(1);
        let h = w.body.create_laser(laser_init(0, 9999, 0, 500, 16));
        let i = h.index as usize;
        w.body.lasers.free_index(i); // 模拟相位 5 的回收
        assert!(!w.body.laser_alive(h), "回收后 laser_alive = false");

        let cv0 = w.body.diag.contract_viol;
        assert!(!w.body.laser_set_speed(h, Fx::ONE, Fx::ONE));
        assert!(!w.body.laser_set_start(h, Fx::ONE));
        assert!(!w.body.laser_set_omega(h, 100));
        assert!(!w.body.laser_rotate(h, Angle(1000)));
        assert!(!w.body.laser_aim(h, Angle(1000)));
        assert!(
            !w.body
                .laser_anchor(h, EnemyHandle::NULL, Fx::ZERO, Fx::ZERO)
        );
        assert!(!w.body.laser_origin(h, Fx::ONE, Fx::ONE));
        assert!(!w.body.laser_cancel(h));
        assert_eq!(w.body.diag.contract_viol, cv0 + 8, "每次调用计一次");

        let cv1 = w.body.diag.contract_viol;
        assert!(!w.body.laser_alive(h));
        assert_eq!(w.body.diag.contract_viol, cv1, "laser_alive 只读不计数");
    }

    /// 负参数钳 0 并计一次违约（同 `create_laser` 口径：一次调用多坏字段只计一次）。
    #[test]
    fn negative_setter_args_clamp_and_count() {
        let mut w = World::new(1);
        let h = w.body.create_laser(laser_init(0, 9999, 0, 500, 16));
        let i = h.index as usize;
        let cv0 = w.body.diag.contract_viol;
        assert!(
            w.body
                .laser_set_speed(h, Fx::from_int(-1), Fx::from_int(-2))
        );
        assert_eq!(w.body.lasers.speed[i], Fx::ZERO);
        assert_eq!(w.body.lasers.start_len[i], Fx::ZERO);
        assert_eq!(
            w.body.diag.contract_viol,
            cv0 + 1,
            "一次调用多坏字段只计一次"
        );
        assert!(w.body.laser_set_start(h, Fx::from_int(-5)));
        assert_eq!(w.body.lasers.start[i], Fx::ZERO);
        assert_eq!(w.body.diag.contract_viol, cv0 + 2);
    }
}
