//! `WorldTables` —— 世界静态数据层（A3）。**owned 形态**（C11）：内部切片 `Box` 拥有，可由
//! `from_bytes` 在运行时反序列化构造，不再是编译期 `&'static`。传递形态不变：**不进 `World`**
//! （I7），`step`/相位按帧以 `&WorldTables` 引用消费。`content_hash`：组 A 恒 0，组 B 由
//! `from_bytes` 计算并校验（LIVE），coherence 守卫读它。

use std::sync::LazyLock;

// `pub use` 保留原有再导出（消费者可能引 `crate::tables::APPEARANCE_*`），且在本模块内可用。
pub use crate::consts::{APPEARANCE_LARGE, APPEARANCE_MEDIUM, APPEARANCE_SMALL, APPEARANCE_STAR};
use crate::items::{ITEM_POINT, ITEM_POWER, ITEM_TYPE_COUNT};
use crate::math::{Angle, Fx};
use crate::world::MAX_ENTITY_RADIUS;

/// 单张掉落表：`(item_type, count)` 行列表。type alias（非 newtype，结构与
/// `Box<[(u8, u8)]>` 完全等价）——纯为压 `clippy::type_complexity`，不改变字段实际类型。
pub type DropTable = Box<[(u8, u8)]>;

/// 全局静态数据层（A3；owned）。见模块文档「传递形态」。
pub struct WorldTables {
    /// 内容哈希：组 B 起 LIVE（`from_bytes` 算 body 的 FNV-1a64 并自校）；组 A 恒 0。
    pub content_hash: u64,
    /// v0 一个角色；定长数组（引擎固定计数，多角色=未来）。
    pub characters: [CharacterCfg; 1],
    pub item_cfg: [ItemTypeCfg; ITEM_TYPE_COUNT],
    pub drop_tables: Box<[DropTable]>,
    pub item_gravity: Fx,
    /// 弹外观表（索引 = appearance id）。
    pub appearances: Box<[AppearanceCfg]>,
}

#[derive(Clone, Copy, Debug)]
pub struct AppearanceCfg {
    pub radius: Fx,
    pub sprite: u16,
}

#[derive(Clone, Copy, Debug)]
pub struct ItemTypeCfg {
    pub score: u32,
    pub eject_speed: Fx,
    pub terminal_vy: Fx,
    pub magnet_speed: Fx,
    pub pickup_radius: Fx,
    pub attract_radius: Fx,
}

/// 单角色配置。owned 化后含 `ShotTypeCfg`（有 `Box`）→ **去 `Copy`、留 `Clone`**。
#[derive(Clone, Debug)]
pub struct CharacterCfg {
    pub high_speed: Fx,
    pub low_speed: Fx,
    pub inv_sqrt2: Fx,
    pub hit_radius: Fx,
    pub graze_radius: Fx,
    pub shot: ShotTypeCfg,
}

/// shottype 表：5 档 × 2 焦点 = 10 槽。owned 化后每槽独立 `Box`（**去 `Copy`、留 `Clone`**）。
#[derive(Clone, Debug)]
pub struct ShotTypeCfg {
    /// `[tier 0..=4][focus 0/1]`；owned 后两焦点槽各持独立分配、内容相等（不再 `ptr::eq` 同一）。
    pub sets: [[Box<[Shooter]>; 2]; 5],
    /// 每档子机偏移（`option` 号 1..=len 查此表；v0 前四档空）。
    pub option_pos: [Box<[(Fx, Fx)]>; 5],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shooter {
    pub interval: u16,
    pub delay: u16,
    pub dx: Fx,
    pub dy: Fx,
    pub angle: Angle,
    pub speed: Fx,
    pub damage: u16,
    pub radius: Fx,
    pub sprite: u16,
    pub option: u8,
    pub flags: u8,
}

// ── v0 内容基元（保持现值逐字节不变）─────────────────────────────────────────

const BASE_SHOOTER: Shooter = Shooter {
    interval: 4,
    delay: 0,
    dx: Fx::ZERO,
    dy: Fx::ZERO,
    angle: Angle(49152),
    speed: Fx::from_int(12),
    damage: 1,
    radius: Fx::from_int(4),
    sprite: 0,
    option: 0,
    flags: 0,
};

const ITEM_GRAVITY_V0: Fx = Fx::from_raw(9_830);

const STD_ITEM: ItemTypeCfg = ItemTypeCfg {
    score: 0,
    eject_speed: Fx::from_int(3),
    terminal_vy: Fx::from_raw(144_179),
    magnet_speed: Fx::from_int(8),
    pickup_radius: Fx::from_int(16),
    attract_radius: Fx::from_int(40),
};

const ITEM_CFG_V0: [ItemTypeCfg; ITEM_TYPE_COUNT] = [
    ItemTypeCfg {
        score: 10,
        ..STD_ITEM
    }, // POWER
    ItemTypeCfg {
        score: 100,
        ..STD_ITEM
    }, // POINT
    ItemTypeCfg {
        score: 50,
        ..STD_ITEM
    }, // LIFE_PIECE
    ItemTypeCfg {
        score: 50,
        ..STD_ITEM
    }, // BOMB_PIECE
    ItemTypeCfg {
        score: 30,
        ..STD_ITEM
    }, // STAR
];

const CHAR0_HIGH_SPEED: Fx = Fx::from_raw(294_912);
const CHAR0_LOW_SPEED: Fx = Fx::from_raw(131_072);
const CHAR0_INV_SQRT2: Fx = Fx::from_raw(46_341);
const CHAR0_HIT_RADIUS: Fx = Fx::from_raw(163_840);
const CHAR0_GRAZE_RADIUS: Fx = Fx::from_int(16);

/// tier 4 子机出生偏移（唯一非空档）。
const TIER4_OPT: (Fx, Fx) = (Fx::from_int(-20), Fx::from_int(8));

/// v0 全内容 owned 构造（**单一真相源**）：harness 烘焙与运行期 `from_bytes` round-trip 皆以此为准。
/// `content_hash` 置 0（组 B 起由 `from_bytes` 计算填真值）。
pub fn build_tables_v0() -> WorldTables {
    // 各档发射器列表（owned；每次调用新分配，两焦点槽各自持有）。
    let tier_1way = || -> Box<[Shooter]> { Box::new([BASE_SHOOTER]) };
    let tier_2way = || -> Box<[Shooter]> {
        Box::new([
            Shooter {
                dx: Fx::from_int(-8),
                ..BASE_SHOOTER
            },
            Shooter {
                dx: Fx::from_int(8),
                ..BASE_SHOOTER
            },
        ])
    };
    let tier_4way = || -> Box<[Shooter]> {
        Box::new([
            Shooter {
                dx: Fx::from_int(-12),
                ..BASE_SHOOTER
            },
            BASE_SHOOTER,
            Shooter {
                dx: Fx::from_int(12),
                ..BASE_SHOOTER
            },
            Shooter {
                option: 1,
                ..BASE_SHOOTER
            },
        ])
    };
    let empty_opt = || -> Box<[(Fx, Fx)]> { Box::new([]) };

    let shot = ShotTypeCfg {
        sets: [
            [tier_1way(), tier_1way()],
            [tier_1way(), tier_1way()],
            [tier_2way(), tier_2way()],
            [tier_2way(), tier_2way()],
            [tier_4way(), tier_4way()],
        ],
        option_pos: [
            empty_opt(),
            empty_opt(),
            empty_opt(),
            empty_opt(),
            Box::new([TIER4_OPT]),
        ],
    };

    // appearances 按 `②` const 下标赋值（防 FM2：const 即下标，结构上无法错序）。
    let mut appearances = [AppearanceCfg {
        radius: Fx::ZERO,
        sprite: 0,
    }; 4];
    appearances[APPEARANCE_SMALL as usize] = AppearanceCfg {
        radius: Fx::from_int(3),
        sprite: 0,
    };
    appearances[APPEARANCE_MEDIUM as usize] = AppearanceCfg {
        radius: Fx::from_int(4),
        sprite: 1,
    };
    appearances[APPEARANCE_LARGE as usize] = AppearanceCfg {
        radius: Fx::from_int(6),
        sprite: 2,
    };
    appearances[APPEARANCE_STAR as usize] = AppearanceCfg {
        radius: Fx::from_int(8),
        sprite: 3,
    };

    let drop_tables: Box<[DropTable]> = Box::new([
        Box::new([]) as DropTable,
        Box::new([(ITEM_POWER, 2u8), (ITEM_POINT, 1u8)]) as DropTable,
    ]);

    WorldTables {
        content_hash: 0,
        characters: [CharacterCfg {
            high_speed: CHAR0_HIGH_SPEED,
            low_speed: CHAR0_LOW_SPEED,
            inv_sqrt2: CHAR0_INV_SQRT2,
            hit_radius: CHAR0_HIT_RADIUS,
            graze_radius: CHAR0_GRAZE_RADIUS,
            shot,
        }],
        item_cfg: ITEM_CFG_V0,
        drop_tables,
        item_gravity: ITEM_GRAVITY_V0,
        appearances: Box::new(appearances),
    }
}

/// 内建默认表。组 A：Rust 直构（owned）；组 B 换 `from_bytes(include_bytes!)` 走字节路径。
pub static TABLES_V0: LazyLock<WorldTables> = LazyLock::new(build_tables_v0);

impl WorldTables {
    /// 表校验（debug/测试用）：`interval > 0`、`radius` 双边入 `[0, MAX_ENTITY_RADIUS]`、
    /// `option` 号 `<=` 该档子机数、`drop_tables` 条目类型合法、角色判定/擦弹半径同域、
    /// appearance 表逐行半径同域（M1 T3）。
    pub fn validate(&self) -> bool {
        if !self.appearances.iter().all(|a| radius_in_range(a.radius)) {
            return false;
        }
        for c in &self.characters {
            if !radius_in_range(c.hit_radius) || !radius_in_range(c.graze_radius) {
                return false;
            }
            for tier in 0..5 {
                let opt_len = c.shot.option_pos[tier].len();
                for focus in 0..2 {
                    for shooter in c.shot.sets[tier][focus].iter() {
                        if shooter.interval == 0 {
                            return false;
                        }
                        if !radius_in_range(shooter.radius) {
                            return false;
                        }
                        if shooter.option != 0 && shooter.option as usize > opt_len {
                            return false;
                        }
                    }
                }
            }
        }
        if !self
            .drop_tables
            .iter()
            .flat_map(|t| t.iter())
            .all(|&(ty, _)| (ty as usize) < ITEM_TYPE_COUNT)
        {
            return false;
        }
        true
    }
}

fn radius_in_range(r: Fx) -> bool {
    r.raw() >= 0 && r.raw() <= MAX_ENTITY_RADIUS.raw()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `TABLES_V0` 必须过表校验——v0 内容自洽的钉死。
    #[test]
    fn tables_v0_validates() {
        assert!(TABLES_V0.validate());
    }

    /// 形状 + v0 内容量拍板：5 档×2 态槽全非悬垂、同档两焦点槽内容相等
    /// （owned 后各自独立分配，非 `std::ptr::eq`）、tier0 一路 / tier2 两路 / tier4
    /// 三路+恰一个子机 shooter。
    #[test]
    fn tables_v0_shape() {
        let shot = &TABLES_V0.characters[0].shot;

        for tier in 0..5 {
            let [a, b] = &shot.sets[tier];
            assert!(!a.is_empty(), "tier {tier} focus0 非悬垂空表");
            assert_eq!(a, b, "tier {tier} 两焦点槽内容相等（owned 后各自独立分配）");
        }

        assert_eq!(shot.sets[0][0].len(), 1, "tier0 一路直射");
        assert_eq!(shot.sets[1][0].len(), 1, "tier1 同 tier0（1 路）");
        assert_eq!(shot.sets[2][0].len(), 2, "tier2 两路");
        assert_eq!(shot.sets[3][0].len(), 2, "tier3 同 tier2（两路）");
        assert_eq!(shot.sets[4][0].len(), 4, "tier4 三路本体 + 1 路子机");

        // tier2 两路对称偏移 ∓8px。
        let t2dx: Vec<i32> = shot.sets[2][0].iter().map(|s| s.dx.raw()).collect();
        assert_eq!(t2dx, vec![Fx::from_int(-8).raw(), Fx::from_int(8).raw()]);

        // tier4：恰一个子机 shooter（option != 0），其余三路 option == 0。
        let option_shooters: Vec<&Shooter> =
            shot.sets[4][0].iter().filter(|s| s.option != 0).collect();
        assert_eq!(option_shooters.len(), 1, "tier4 恰一个子机 shooter");
        assert_eq!(option_shooters[0].option, 1);
        assert_eq!(option_shooters[0].dx, Fx::ZERO);
        assert_eq!(option_shooters[0].dy, Fx::ZERO);

        // 子机出生偏移表：tier4 恰一行 (-20px, 8px)，tier0..=3 空。
        assert_eq!(shot.option_pos[4].len(), 1, "tier4 子机出生点表恰一行");
        assert_eq!(shot.option_pos[4][0], (Fx::from_int(-20), Fx::from_int(8)));
        for tier in 0..4 {
            assert!(shot.option_pos[tier].is_empty(), "tier{tier} 无子机");
        }
    }

    /// 配置表逐行健全（迁自 `items.rs`：M0-17 T2 道具表搬家）：分值/物理参数为正、
    /// 拾取半径 ≤ MAX_ENTITY_RADIUS（行 5 加法证明前提）。
    #[test]
    fn item_cfg_rows_sane() {
        use crate::items::ITEM_POINT;
        for (t, cfg) in TABLES_V0.item_cfg.iter().enumerate() {
            assert!(cfg.score > 0, "type {t}");
            assert!(cfg.eject_speed.raw() > 0 && cfg.terminal_vy.raw() > 0);
            assert!(cfg.magnet_speed.raw() > 0 && cfg.attract_radius.raw() > 0);
            assert!(
                cfg.pickup_radius.raw() > 0
                    && cfg.pickup_radius.raw() <= crate::world::MAX_ENTITY_RADIUS.raw(),
                "type {t} 拾取半径越出行 5 加法安全域"
            );
        }
        assert_eq!(TABLES_V0.item_cfg[ITEM_POINT as usize].score, 100);
    }

    /// 掉落表（迁自 `items.rs`）：表 0 恒空（enemy.drop_table 零初始化默认 = 不掉）；
    /// 表 1 = 标准杂鱼。
    #[test]
    fn drop_tables_shape() {
        use crate::items::{ITEM_POINT, ITEM_POWER, ITEM_TYPE_COUNT};
        assert!(TABLES_V0.drop_tables[0].is_empty());
        assert_eq!(
            &*TABLES_V0.drop_tables[1],
            &[(ITEM_POWER, 2), (ITEM_POINT, 1)]
        );
        assert!(
            TABLES_V0
                .drop_tables
                .iter()
                .flat_map(|t| t.iter())
                .all(|&(ty, _)| (ty as usize) < ITEM_TYPE_COUNT),
            "掉落表条目类型必须合法——扩展四步第④步的脚下网"
        );
    }

    /// 判别腿：interval=0 / radius 超上限 / option 号越界的坏表各自 `validate() == false`。
    #[test]
    fn validate_rejects_bad() {
        fn bad_with_shot(shot: ShotTypeCfg) -> WorldTables {
            let mut t = build_tables_v0();
            t.characters[0].shot = shot;
            t
        }

        // interval=0：tier0 换成一个 interval 为 0 的坏 shooter。
        {
            let bad: Box<[Shooter]> = Box::new([Shooter {
                interval: 0,
                ..BASE_SHOOTER
            }]);
            let mut shot = build_tables_v0().characters[0].shot.clone();
            shot.sets[0] = [bad.clone(), bad];
            assert!(
                !bad_with_shot(shot).validate(),
                "interval=0 必须被 validate 拒绝"
            );
        }

        // radius 超上限：MAX_ENTITY_RADIUS = 1024px，给 2000px。
        {
            let bad: Box<[Shooter]> = Box::new([Shooter {
                radius: Fx::from_int(2000),
                ..BASE_SHOOTER
            }]);
            let mut shot = build_tables_v0().characters[0].shot.clone();
            shot.sets[0] = [bad.clone(), bad];
            assert!(
                !bad_with_shot(shot).validate(),
                "radius 超 MAX_ENTITY_RADIUS 必须被 validate 拒绝"
            );
        }

        // option 号越界：tier0 的 option_pos[0] 是空表，option=1 越界（1 > 0）。
        {
            let bad: Box<[Shooter]> = Box::new([Shooter {
                option: 1,
                ..BASE_SHOOTER
            }]);
            let mut shot = build_tables_v0().characters[0].shot.clone();
            shot.sets[0] = [bad.clone(), bad];
            assert!(
                !bad_with_shot(shot).validate(),
                "option 号超出该档子机数必须被 validate 拒绝"
            );
        }
    }

    /// appearance 表 v0 形状（M1 T3）：恰 4 行，半径 3/4/6/8px 递增，sprite 逐行不同。
    #[test]
    fn appearances_v0_shape() {
        assert_eq!(TABLES_V0.appearances.len(), 4);
        let radii: Vec<i32> = TABLES_V0
            .appearances
            .iter()
            .map(|a| a.radius.raw())
            .collect();
        assert_eq!(
            radii,
            vec![
                Fx::from_int(3).raw(),
                Fx::from_int(4).raw(),
                Fx::from_int(6).raw(),
                Fx::from_int(8).raw(),
            ],
            "半径递增 3/4/6/8px"
        );
        let sprites: Vec<u16> = TABLES_V0.appearances.iter().map(|a| a.sprite).collect();
        assert_eq!(sprites, vec![0, 1, 2, 3], "sprite 逐行不同");
        assert_eq!(APPEARANCE_SMALL, 0);
        assert_eq!(APPEARANCE_MEDIUM, 1);
        assert_eq!(APPEARANCE_LARGE, 2);
        assert_eq!(APPEARANCE_STAR, 3);
    }

    /// appearance 表 validate 判别腿：半径超上限的坏行必须被拒绝。
    #[test]
    fn validate_rejects_bad_appearance_radius() {
        let mut bad = build_tables_v0();
        bad.appearances = Box::new([AppearanceCfg {
            radius: Fx::from_int(2000), // 超 MAX_ENTITY_RADIUS(1024)
            sprite: 0,
        }]);
        assert!(!bad.validate(), "appearance 半径超上限必须被 validate 拒绝");
    }
}
