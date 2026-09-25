//! 激光池（spec 2026-09-25-laser-pool-design）——直线激光：射线原点 + 方向 + 射线上 [start, end] 一段。
//! 每帧 `end += speed; start = max(start, end − start_len, 0)`，`angle += omega`；三态 0 预警 / 1 生效 / 2 收缩。
//! 判定半高 = width/2（画多宽判多宽），只在 state 1 判。原作口径与转写换算见 spec §2、§9.1。

use crate::define_pool;
use crate::math::{Angle, Fx};

pub const LASER_WARN: u8 = 0;
pub const LASER_ACTIVE: u8 = 1;
pub const LASER_FADE: u8 = 2;
/// `start` 越过它即回收（原作 640.0，整条已出屏）。
pub const LASER_CULL: Fx = Fx::from_int(640);
/// `flags` 位 0：收缩态改为 alpha 淡出（纯表现；0 = 变窄）。
pub const LASER_FLAG_FADE_ALPHA: u8 = 1 << 0;
/// `anchor_idx` 取这个值表示没有挂靠。
pub const ANCHOR_NONE: u16 = 0xFFFF;
/// 坐标类字段（`ox/oy/ax/ay`）的双边钳位上界（P4-b）：场内坐标的合理上界远小于它，
/// 钳住它才能让相位 5 的 `enemies.x + ax` 与判定的 `px − ox` 在 Q16.16（±32768）里加法不溢。
pub const LASER_COORD_MAX: Fx = Fx::from_int(4096);
/// 长度/速率类字段（`start/end/start_len/speed`）的上界（P4-b）：`end` 最大约 640 + 2×4096，
/// 远小于 32768，故 `end + speed` 与 `end − start_len` 不会溢出。
pub const LASER_LEN_MAX: Fx = Fx::from_int(4096);

define_pool! {
    Laser, cap = 256,
    fields {
        ox: Fx, oy: Fx, angle: Angle, omega: i16,
        start: Fx, end: Fx, start_len: Fx, speed: Fx,
        width: Fx, sprite: u16,
        warn: u16, active: u16, fade: u16, timer: u16, state: u8,
        anchor_idx: u16, anchor_gen: u16, ax: Fx, ay: Fx,
        // 观测：本帧相位 5 结束时相对上一帧同一时刻的变化（含 ECL rotate/aim/origin、omega、挂靠）。
        dx: Fx, dy: Fx, dang: i16,
        px: Fx, py: Fx, pang: Angle,
        flags: u8, born_frame: u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 写满全部字段的非零初值（`seed` 让两次调用取值不同，用于复用槽覆写判别）。
    fn full_init(seed: u16) -> LaserInit {
        let s = seed as i32;
        let t = seed as i16;
        LaserInit {
            ox: Fx::from_raw(s + 1),
            oy: Fx::from_raw(s + 2),
            angle: Angle(seed + 3),
            omega: t + 4,
            start: Fx::from_raw(s + 5),
            end: Fx::from_raw(s + 6),
            start_len: Fx::from_raw(s + 7),
            speed: Fx::from_raw(s + 8),
            width: Fx::from_raw(s + 9),
            sprite: seed + 10,
            warn: seed + 11,
            active: seed + 12,
            fade: seed + 13,
            timer: seed + 14,
            state: (seed % 200) as u8 + 15,
            anchor_idx: seed + 16,
            anchor_gen: seed + 17,
            ax: Fx::from_raw(s + 18),
            ay: Fx::from_raw(s + 19),
            dx: Fx::from_raw(s + 20),
            dy: Fx::from_raw(s + 21),
            dang: t + 22,
            px: Fx::from_raw(s + 23),
            py: Fx::from_raw(s + 24),
            pang: Angle(seed + 25),
            flags: (seed % 200) as u8 + 26,
            born_frame: seed as u32 + 27,
        }
    }

    /// 逐字段断言的宏（写满全字段 + 读回；漏一个字段就在下面清单里看得出来）。
    macro_rules! assert_fields {
        ($p:expr, $i:expr, $v:expr) => {
            let v = $v;
            let i = $i;
            assert_eq!($p.ox[i], v.ox, "ox");
            assert_eq!($p.oy[i], v.oy, "oy");
            assert_eq!($p.angle[i], v.angle, "angle");
            assert_eq!($p.omega[i], v.omega, "omega");
            assert_eq!($p.start[i], v.start, "start");
            assert_eq!($p.end[i], v.end, "end");
            assert_eq!($p.start_len[i], v.start_len, "start_len");
            assert_eq!($p.speed[i], v.speed, "speed");
            assert_eq!($p.width[i], v.width, "width");
            assert_eq!($p.sprite[i], v.sprite, "sprite");
            assert_eq!($p.warn[i], v.warn, "warn");
            assert_eq!($p.active[i], v.active, "active");
            assert_eq!($p.fade[i], v.fade, "fade");
            assert_eq!($p.timer[i], v.timer, "timer");
            assert_eq!($p.state[i], v.state, "state");
            assert_eq!($p.anchor_idx[i], v.anchor_idx, "anchor_idx");
            assert_eq!($p.anchor_gen[i], v.anchor_gen, "anchor_gen");
            assert_eq!($p.ax[i], v.ax, "ax");
            assert_eq!($p.ay[i], v.ay, "ay");
            assert_eq!($p.dx[i], v.dx, "dx");
            assert_eq!($p.dy[i], v.dy, "dy");
            assert_eq!($p.dang[i], v.dang, "dang");
            assert_eq!($p.px[i], v.px, "px");
            assert_eq!($p.py[i], v.py, "py");
            assert_eq!($p.pang[i], v.pang, "pang");
            assert_eq!($p.flags[i], v.flags, "flags");
            assert_eq!($p.born_frame[i], v.born_frame, "born_frame");
        };
    }

    /// 全覆写：写满 27 个字段 → 逐字段读回；free 后复用同槽再写另一组值 → 无陈旧遗留。
    /// 支撑「哈希全槽、不用 alive 掩码」（P6）：复用槽必须写满，否则空槽也会进指纹。
    #[test]
    fn realloc_overwrites_all_fields() {
        let mut p = LaserPool::new();
        let a = full_init(0);
        let h = p.alloc(a).unwrap();
        let i = p.get(h).unwrap();
        assert_fields!(p, i, a);

        // 复用同一槽（最低空位优先），另一组值覆写：任一字段漏写即在此红。
        assert!(p.free(h));
        let b = full_init(100);
        let h2 = p.alloc(b).unwrap();
        assert_eq!(h2.index, h.index, "最低空位应复用同槽");
        assert_ne!(h2.generation, h.generation, "复用槽代际必须前进");
        let i2 = p.get(h2).unwrap();
        assert_fields!(p, i2, b);
    }
}
