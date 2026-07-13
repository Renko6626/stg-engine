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

#[cfg(test)]
mod tests {
    use super::*;

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
