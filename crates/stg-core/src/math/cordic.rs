//! `atan2` —— 整数 CORDIC 向量模式，迭代次数**钉死 16 轮**（次数是契约，改 = bump engine_ver）。
//! 纯整数移位加减，天然确定；免二维查表。角度常数表 `atan(2^-i)`（BAM）由 harness 生成并 commit。

use crate::math::codec::decode_u16;
use crate::math::{Angle, Fx};

const ITERS: usize = 16;

/// `ATAN_BAM[i] = round_ties_even(atan(2^-i) / (2π) · 65536)`。
const ATAN_BAM: [u16; ITERS] = decode_u16(include_bytes!("tables/atan_cordic.bin"));

/// `atan2(y, x) -> Angle`（BAM）。定义域含 (0,0) → 返回 0（约定）。
pub fn atan2(y: Fx, x: Fx) -> Angle {
    let mut x = x.raw() as i64;
    let mut y = y.raw() as i64;
    if x == 0 && y == 0 {
        return Angle::ZERO;
    }
    // 象限预旋到 x>0 半平面，累加已旋角度（BAM，i32 中间量）。
    let mut angle: i32 = 0;
    if x < 0 {
        // 绕原点转 180°（HALF）到右半平面。
        x = -x;
        y = -y;
        angle = angle.wrapping_add(Angle::HALF.raw() as i32);
    }
    // 现在 x>0。CORDIC 向量模式：把 y 逼近 0，累加/累减各级 atan(2^-i)。
    //
    // **无分支写法（stg-rl 刀 2026-09-15 提速）**：原实现每轮 `if y > 0 {…} else {…}`，走向随
    // 输入方向变化，方向各异的弹（真实弹幕）上分支预测大量失败。这里用掩码
    // `m = 0（y>0）/ -1（y<=0）` 与条件取负 `(v ^ m) - m` 合并两臂——逐位等价于原 if/else：
    // y>0：`nx = x + (y>>i)`、`ny = y - (x>>i)`、`angle += da`；
    // 否则：`nx = x - (y>>i)`、`ny = y + (x>>i)`、`angle -= da`。
    // 实测（Xeon Gold 6330，共享机，每调用）：随机方向 77ns → 44ns；全同向（分支全可预测）24ns → 44ns。
    // 取舍：模拟内调用点（瞄准）次数少，热点是 stg-rl 观测编码，而那里直线弹由派生量记忆吸收，
    // 真正落到 atan2 的正是速度在变、方向各异的弹——恰是无分支版占优的输入。
    // 移位仍是 i64 算术右移，轮数仍钉死 16（契约不变，不 bump engine_ver）；
    // 逐位等价由 `tests::branchless_matches_reference_bit_for_bit` 押运（旧实现原样留作参照）。
    for (i, &da_bam) in ATAN_BAM.iter().enumerate() {
        let m = -((y <= 0) as i64);
        let ys = ((y >> i) ^ m) - m;
        let xs = ((x >> i) ^ m) - m;
        let nx = x + ys;
        let ny = y - xs;
        x = nx;
        y = ny;
        let m32 = m as i32;
        angle = angle.wrapping_add(((da_bam as i32) ^ m32).wrapping_sub(m32));
    }
    Angle(angle as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 提速前的分支版实现，原样保留作逐位等价参照（**勿改**：它就是契约）。
    fn atan2_reference(y: Fx, x: Fx) -> Angle {
        let mut x = x.raw() as i64;
        let mut y = y.raw() as i64;
        if x == 0 && y == 0 {
            return Angle::ZERO;
        }
        let mut angle: i32 = 0;
        if x < 0 {
            x = -x;
            y = -y;
            angle = angle.wrapping_add(Angle::HALF.raw() as i32);
        }
        for (i, &da_bam) in ATAN_BAM.iter().enumerate() {
            let da = da_bam as i32;
            if y > 0 {
                let nx = x + (y >> i);
                let ny = y - (x >> i);
                x = nx;
                y = ny;
                angle = angle.wrapping_add(da);
            } else {
                let nx = x - (y >> i);
                let ny = y + (x >> i);
                x = nx;
                y = ny;
                angle = angle.wrapping_sub(da);
            }
        }
        Angle(angle as u16)
    }

    fn same(y: i32, x: i32) {
        let (fy, fx) = (Fx::from_raw(y), Fx::from_raw(x));
        assert_eq!(atan2(fy, fx), atan2_reference(fy, fx), "y={y} x={x}");
    }

    /// 无分支版与分支版**逐位相等**：边界值全组合 + 小值网格 + 一百万组伪随机（跨所有量级）。
    /// 任何一处掩码 / 取负 / 移位写错都会在前两组里立刻红；随机组兜底大数与符号组合。
    #[test]
    fn branchless_matches_reference_bit_for_bit() {
        const EDGES: [i32; 19] = [
            0,
            1,
            -1,
            2,
            -2,
            3,
            -3,
            65535,
            -65535,
            65536,
            -65536,
            1 << 20,
            -(1 << 20),
            12_345_678,
            -87_654_321,
            i32::MAX,
            i32::MIN,
            i32::MAX - 1,
            i32::MIN + 1,
        ];
        for &y in &EDGES {
            for &x in &EDGES {
                same(y, x);
            }
        }
        for y in -256..=256 {
            for x in -256..=256 {
                same(y, x);
            }
        }
        let mut rng = crate::rng::Pcg32::new(0x5EED_A7A2, 0x0C0D);
        for k in 0..1_000_000u32 {
            let shift = k % 32; // 覆盖从满 i32 到个位数的所有量级
            let y = (rng.next_u32() as i32) >> shift;
            let x = (rng.next_u32() as i32) >> (31 - shift);
            same(y, x);
        }
    }

    fn approx(a: Angle, target: u16, tol: i32) {
        // 环上最短角距
        let d = (a.raw() as i32 - target as i32).rem_euclid(65536);
        let d = d.min(65536 - d);
        assert!(d <= tol, "angle={} target={target} dist={d}", a.raw());
    }

    #[test]
    fn cardinal_directions() {
        approx(atan2(Fx::ZERO, Fx::ONE), 0, 4); // +x → 0
        approx(atan2(Fx::ONE, Fx::ZERO), 16384, 4); // +y → π/2
        approx(atan2(Fx::ZERO, -Fx::ONE), 32768, 4); // -x → π
        approx(atan2(-Fx::ONE, Fx::ZERO), 49152, 4); // -y → 3π/2
    }

    #[test]
    fn diagonals() {
        approx(atan2(Fx::ONE, Fx::ONE), 8192, 8); // 45°
        approx(atan2(Fx::ONE, -Fx::ONE), 24576, 8); // 135°
        approx(atan2(-Fx::ONE, -Fx::ONE), 40960, 8); // 225°
    }

    #[test]
    fn origin_is_zero() {
        assert_eq!(atan2(Fx::ZERO, Fx::ZERO), Angle::ZERO);
    }

    #[test]
    fn roundtrip_with_sincos() {
        use crate::math::sincos;
        // 从若干角度取单位向量，atan2 回来应接近原角
        for &a in &[1000u16, 9000, 20000, 33000, 50000] {
            let (s, c) = sincos(Angle(a));
            let back = atan2(s, c);
            let d = (back.raw() as i32 - a as i32).rem_euclid(65536);
            let d = d.min(65536 - d);
            assert!(d <= 12, "a={a} back={} dist={d}", back.raw());
        }
    }
}
