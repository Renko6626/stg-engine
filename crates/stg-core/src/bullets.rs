//! 弹池（D3）——`define_pool!` 的首个真实实例。
//! M0-3 Task 3 在此填 `define_pool! { Bullet, cap = 8192, fields { ... } }`。
//! 运动 / 双表示【逻辑】归 M0-4，本模块只落存储 / 分配 / 校验。

use crate::define_pool;
use crate::math::{Angle, Fx};

/// `flags` 位：本帧被作用区清除（settle 趟一置、趟二读【中弹跳过=bomb 救命】、cleanup 回收）。
///
/// **生命只在同一帧的相位 7→9 之间**，从不跨帧——故 collide 无需检查此位（次帧看不到已清除的弹）。
pub const BULLET_CLEARED: u8 = 1 << 0;
/// `flags` 位：POLAR_FX 连续效果（integrate 每帧 `angle += ang_vel; speed += accel;` 后刷 v）。
pub const BULLET_POLAR_FX: u8 = 1 << 1;
/// `flags` 位：CART_FX 连续效果（integrate 每帧 `vx += ax; vy += ay;` 后按阈值回填极坐标）。
/// 与 `BULLET_POLAR_FX` 互斥：置一清另一（TH16 `c68 &= ~0x9` 语义）。位 3-4 预留反弹计数（D4）。
pub const BULLET_CART_FX: u8 = 1 << 2;
/// `flags` 位 3-4：反弹剩余次数（D4 `BOUNCE_ARM`，≤3）。walls 掩码不进弹本体——从弹自有段读。
pub const BULLET_BOUNCE_SHIFT: u32 = 3;
pub const BULLET_BOUNCE_MASK: u8 = 0b0001_1000;

// 弹池（D3 定稿，19 字段）。哑弹 / 变换弹 / 任务弹**共池**；变换【段】另存 XformSegPool（M0-4 手写）。
// `transform_head == 0xFFFF` 即哑弹（无段、不付段内存，只付这几字节游标）。运动 / 双表示逻辑归 M0-4。
define_pool! {
    Bullet, cap = 8192,
    fields {
        x: Fx, y: Fx, vx: Fx, vy: Fx,
        speed: Fx, angle: Angle,
        ang_vel: i16, accel: Fx,
        ax: Fx, ay: Fx,
        sprite: u16, radius: Fx,
        delay: u8, life: u16, flags: u8, grazed_by: u8,
        transform_head: u16, xform_wait: u16, xform_next: u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checksum::Checksum;

    // 小测试池：cap=130 故意非 64 倍数，验末字掩码（NW=3，末字有效位=2）。
    define_pool! { Tp, cap = 130, fields { a: u32, b: u16 } }

    /// 裸切片访问器（通道 A）判别式：切片指向真 SoA + alive_words 反映位掩码 + 位扫 ≡ iter_alive。
    #[test]
    fn view_slices_point_to_soa_and_alive_words_reflect_bitmap() {
        let mut p = TpPool::new();
        let h0 = p.alloc(TpInit { a: 10, b: 1 }).unwrap();
        let h1 = p.alloc(TpInit { a: 20, b: 2 }).unwrap();
        let h2 = p.alloc(TpInit { a: 30, b: 3 }).unwrap();
        p.free(h1); // 中间释放：位掩码留洞

        // 裸切片指向真 SoA（判别："a() 错返 b() 切片" 即红）
        let a = p.a();
        assert_eq!(a.len(), TpPool::CAP);
        assert_eq!(a[h0.index as usize], 10);
        assert_eq!(a[h2.index as usize], 30);

        // alive_words 反映位掩码：popcount == 活跃数(2)，h0/h2 位 1、h1 位 0
        let aw = p.alive_words();
        let popcount: u32 = aw.iter().map(|w| w.count_ones()).sum();
        assert_eq!(popcount, 2);
        let bit = |i: usize| (aw[i / 64] >> (i % 64)) & 1;
        assert_eq!(bit(h0.index as usize), 1);
        assert_eq!(bit(h1.index as usize), 0, "已 free 位为 0");
        assert_eq!(bit(h2.index as usize), 1);

        // 位扫 ≡ iter_alive（钉死批量路径与便利迭代一致——A9 "过滤是消费者义务"）
        let scanned: Vec<usize> = (0..TpPool::CAP).filter(|&i| bit(i) == 1).collect();
        let iterated: Vec<usize> = p.iter_alive().collect();
        assert_eq!(scanned, iterated);
    }

    #[test]
    fn alloc_get_free_roundtrip() {
        let mut p = TpPool::new();
        let h = p.alloc(TpInit { a: 7, b: 9 }).unwrap();
        let i = p.get(h).unwrap();
        assert_eq!(p.a[i], 7);
        assert_eq!(p.b[i], 9);
        assert!(p.free(h));
        assert_eq!(p.get(h), None); // 释放后失效
        assert!(!p.free(h)); // 二次 free no-op
    }

    #[test]
    fn zeroed_handle_invalid() {
        let p = TpPool::new();
        assert_eq!(
            p.get(TpHandle {
                index: 0,
                generation: 0
            }),
            None
        );
        assert_eq!(p.get(TpHandle::NULL), None);
    }

    #[test]
    fn generation_defeats_stale_handle() {
        let mut p = TpPool::new();
        let h1 = p.alloc(TpInit { a: 1, b: 1 }).unwrap();
        assert!(p.free(h1));
        let h2 = p.alloc(TpInit { a: 2, b: 2 }).unwrap();
        assert_eq!(h2.index, h1.index); // 最低空位 → 复用同槽
        assert_eq!(p.get(h1), None); // 旧句柄 gen 失配
        assert_eq!(p.get(h2).map(|i| p.a[i]), Some(2));
    }

    #[test]
    fn full_pool_returns_none_and_tail_mask() {
        let mut p = TpPool::new();
        for k in 0..130u32 {
            assert!(p.alloc(TpInit { a: k, b: 0 }).is_some(), "第 {k} 个应成功");
        }
        assert_eq!(p.alloc(TpInit { a: 0, b: 0 }), None); // 满
        // 幽灵位（130..192）绝不被分配 → 全部 index < 130
        assert!(p.iter_alive().all(|i| i < 130));
        assert_eq!(p.iter_alive().count(), 130);
    }

    #[test]
    fn iter_alive_ascending() {
        let mut p = TpPool::new();
        let _h0 = p.alloc(TpInit { a: 0, b: 0 }).unwrap();
        let h1 = p.alloc(TpInit { a: 1, b: 0 }).unwrap();
        let _h2 = p.alloc(TpInit { a: 2, b: 0 }).unwrap();
        p.free(h1); // 释放中间
        let got: Vec<usize> = p.iter_alive().collect();
        assert_eq!(got, vec![0, 2]); // 升序、跳过死槽
        // 再分配 → 最低空位 = 1
        let h = p.alloc(TpInit { a: 9, b: 0 }).unwrap();
        assert_eq!(h.index, 1);
    }

    #[test]
    fn realloc_overwrites_all_fields() {
        let mut p = TpPool::new();
        let h = p.alloc(TpInit { a: 111, b: 222 }).unwrap();
        p.free(h);
        let h2 = p.alloc(TpInit { a: 333, b: 444 }).unwrap();
        let i = p.get(h2).unwrap();
        assert_eq!((p.a[i], p.b[i]), (333, 444)); // 无陈旧遗留
    }

    // 一个哑弹的 Init（transform_head=0xFFFF）。
    fn dumb_bullet(x: i32, y: i32) -> BulletInit {
        BulletInit {
            x: Fx::from_int(x),
            y: Fx::from_int(y),
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
            transform_head: 0xFFFF,
            xform_wait: 0,
            xform_next: 0,
        }
    }

    #[test]
    fn bullet_pool_checksum_reacts_to_state() {
        let mut p = BulletPool::new();
        let empty = p.checksum();
        let h = p.alloc(dumb_bullet(10, 20)).unwrap();
        assert_ne!(p.checksum(), empty); // 分配改变指纹
        let before = p.checksum();
        let i = p.get(h).unwrap();
        p.x[i] = Fx::from_int(11);
        assert_ne!(p.checksum(), before); // 改一个字段 → 指纹变
    }

    #[test]
    fn bullet_pool_new_is_zero_and_deterministic() {
        // 两个新池指纹相同（全零确定）——跨机金向量的基石。
        let a = BulletPool::new();
        let b = BulletPool::new();
        assert_eq!(a.checksum(), b.checksum());
    }

    #[test]
    fn bullet_dumb_is_transform_head_sentinel() {
        let mut p = BulletPool::new();
        let h = p.alloc(dumb_bullet(0, 0)).unwrap();
        let i = p.get(h).unwrap();
        assert_eq!(p.transform_head[i], 0xFFFF); // 哑弹哨兵
    }

    #[test]
    fn copy_into_roundtrip() {
        let mut a = BulletPool::new();
        let h = a.alloc(dumb_bullet(5, 6)).unwrap();
        let snap = a.checksum();
        let mut b = BulletPool::new();
        a.copy_into(&mut b);
        assert_eq!(b.checksum(), snap); // 拷贝后指纹相同
        // 改 a 不影响 b
        let i = a.get(h).unwrap();
        a.x[i] = Fx::from_int(999);
        assert_ne!(a.checksum(), snap);
        assert_eq!(b.checksum(), snap);
    }
}
