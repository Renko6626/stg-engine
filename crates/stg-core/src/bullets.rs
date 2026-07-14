//! 弹池（D3）——`define_pool!` 的首个真实实例。
//! M0-3 Task 3 在此填 `define_pool! { Bullet, cap = 8192, fields { ... } }`。
//! 运动 / 双表示【逻辑】归 M0-4，本模块只落存储 / 分配 / 校验。

#[cfg(test)]
mod tests {
    use crate::define_pool;

    // 小测试池：cap=130 故意非 64 倍数，验末字掩码（NW=3，末字有效位=2）。
    define_pool! { Tp, cap = 130, fields { a: u32, b: u16 } }

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
}
