//! 世界本体（stg_core::world）—— WorldBody 字段 + `pub(crate)` 相位函数 + 写 API + PhaseGuard。
//! 最小切片（M0-4）：无 ECL/玩家/碰撞；3 输出缓冲随各自生产者再加。
//! **构造只走 `step::World::new`（堆零初始化）**——WorldBody 无 `new()`，避免 ~450KB 栈临时量。

use crate::bullets::{BulletHandle, BulletInit, BulletPool};
use crate::math::Fx;
use crate::player::PlayerState;
use crate::rng::Pcg32;
use crate::shots::{ShotHandle, ShotInit, ShotPool};

// ── 常量：池 id / 错误码 / 场界（D7 中轴原点，384×448 + 越界边距）──────────
pub const POOL_BULLET: usize = 0;
pub const POOL_SHOT: usize = 1;
pub const STATUS_OK: u16 = 0;
pub const STATUS_POOL_FULL: u16 = 1;

const FIELD_HALF_W: i32 = 192; // x ∈ [-192, 192]
const FIELD_HEIGHT: i32 = 448; // y ∈ [0, 448]
const OOB_MARGIN: i32 = 64; // 越界回收边距

// ── 相位索引（A4 v2，0-based；PhaseGuard 押运）───────────────────────────
pub(crate) const NUM_PHASES: u8 = 11;
pub(crate) const PH_BEGIN: u8 = 0;
pub(crate) const PH_DECODE: u8 = 1;
pub(crate) const PH_DIRECTOR: u8 = 2;
pub(crate) const PH_PLAYERS: u8 = 3;
pub(crate) const PH_XFORM: u8 = 4;
pub(crate) const PH_INTEGRATE: u8 = 5;
pub(crate) const PH_COLLIDE: u8 = 6;
pub(crate) const PH_SETTLE: u8 = 7;
pub(crate) const PH_ECL_HOOK: u8 = 8;
pub(crate) const PH_CLEANUP: u8 = 9;
pub(crate) const PH_ADVANCE: u8 = 10;

/// 播种流选择（PCG32 seq）；固定进 World 身份。
pub(crate) const RNG_SEQ: u64 = 0xda3e_39cb_94b9_5bdb;

/// 诊断计数器（P4；**参与校验和**——两机必须丢得一样多）。
#[repr(C)]
#[derive(Clone, Copy, Default, crate::checksum::Checksum)]
pub struct DiagCounters {
    pub pool_full: [u32; 8], // 按池 id
    pub contract_viol: u32,
}

/// 世界本体（最小切片）。构造走 `step::World::new`（堆零初始化 + 播种 rng）。
#[repr(C)]
#[derive(crate::checksum::Checksum)]
pub struct WorldBody {
    pub frame: u32,
    pub rng: Pcg32,
    pub bullets: BulletPool,
    pub players: [PlayerState; crate::MAX_PLAYERS],
    pub shots: ShotPool,
    pub diag: DiagCounters,
    pub last_status: u16,
    #[cfg(debug_assertions)]
    #[checksum(skip = "debug-only 时序护栏")]
    pub(crate) phase_guard: u8,
}

impl WorldBody {
    /// PhaseGuard：debug 断言相位保序，乱序 panic；release 零成本。
    #[inline]
    pub(crate) fn phase_enter(&mut self, p: u8) {
        #[cfg(debug_assertions)]
        {
            debug_assert_eq!(self.phase_guard, p, "step 相位乱序：期望此相 {p}");
            self.phase_guard = (p + 1) % NUM_PHASES;
        }
        let _ = p;
    }

    // ── 写 API（P1：调用方只走这里，不摸池内存）──────────────────────────
    /// 创建一颗弹（P4-a：池满 → NULL + 诊断计数 + last_status）。
    pub fn create_bullet(&mut self, init: BulletInit) -> BulletHandle {
        match self.bullets.alloc(init) {
            Some(h) => h,
            None => {
                self.diag.pool_full[POOL_BULLET] = self.diag.pool_full[POOL_BULLET].wrapping_add(1);
                self.last_status = STATUS_POOL_FULL;
                BulletHandle::NULL
            }
        }
    }

    /// 创建一发自机弹（P4-a：池满 → NULL + 诊断计数 + last_status）。
    pub fn create_player_shot(&mut self, init: ShotInit) -> ShotHandle {
        match self.shots.alloc(init) {
            Some(h) => h,
            None => {
                self.diag.pool_full[POOL_SHOT] = self.diag.pool_full[POOL_SHOT].wrapping_add(1);
                self.last_status = STATUS_POOL_FULL;
                ShotHandle::NULL
            }
        }
    }

    /// 越界判定（含边距）。
    fn out_of_bounds(x: Fx, y: Fx) -> bool {
        let xi = x.to_int_floor();
        let yi = y.to_int_floor();
        !(-FIELD_HALF_W - OOB_MARGIN..=FIELD_HALF_W + OOB_MARGIN).contains(&xi)
            || !(-OOB_MARGIN..=FIELD_HEIGHT + OOB_MARGIN).contains(&yi)
    }

    // ── 相位函数（pub(crate)，每个先 phase_enter 保序）────────────────────
    pub(crate) fn begin(&mut self) {
        self.phase_enter(PH_BEGIN); // 最小切片无输出缓冲可清；仅护栏推进
    }
    pub(crate) fn decode_input(&mut self) {
        self.phase_enter(PH_DECODE); // stub：无玩家
    }
    pub(crate) fn update_players(&mut self) {
        self.phase_enter(PH_PLAYERS); // stub
    }
    pub(crate) fn run_transforms(&mut self) {
        self.phase_enter(PH_XFORM); // stub：无变换段池
    }
    pub(crate) fn integrate(&mut self) {
        self.phase_enter(PH_INTEGRATE);
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
                self.bullets.x[i] = self.bullets.x[i] + self.bullets.vx[i];
                self.bullets.y[i] = self.bullets.y[i] + self.bullets.vy[i];
                if self.bullets.life[i] != 0xFFFF && self.bullets.life[i] > 0 {
                    self.bullets.life[i] -= 1;
                }
            }
        }
    }
    pub(crate) fn collide(&mut self) {
        self.phase_enter(PH_COLLIDE); // stub：碰撞 D8 后续
    }
    pub(crate) fn settle(&mut self) {
        self.phase_enter(PH_SETTLE); // stub
    }
    pub(crate) fn cleanup(&mut self) {
        self.phase_enter(PH_CLEANUP);
        let nw = self.bullets.alive.len();
        for w in 0..nw {
            let mut bits = self.bullets.alive[w];
            while bits != 0 {
                let i = w * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let dead = (self.bullets.life[i] != 0xFFFF && self.bullets.life[i] == 0)
                    || Self::out_of_bounds(self.bullets.x[i], self.bullets.y[i]);
                if dead {
                    self.bullets.free_index(i);
                }
            }
        }
    }
    pub(crate) fn advance(&mut self) {
        self.phase_enter(PH_ADVANCE);
        self.frame += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oob_detects_margin() {
        assert!(!WorldBody::out_of_bounds(
            Fx::from_int(0),
            Fx::from_int(200)
        ));
        assert!(WorldBody::out_of_bounds(
            Fx::from_int(1000),
            Fx::from_int(0)
        ));
        assert!(WorldBody::out_of_bounds(
            Fx::from_int(0),
            Fx::from_int(-100)
        ));
        assert!(WorldBody::out_of_bounds(Fx::from_int(0), Fx::from_int(600)));
    }
}
