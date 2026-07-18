//! `WorldTables` —— 静态只读数据层骨架（A3；spec
//! `docs/superpowers/specs/2026-07-18-m0-17-shottype-worldtables.md`）。角色参数（原
//! player.rs 常量）+ 道具表（**M0-17 T2 起 `ItemTypeCfg` 结构体与三件内容表已从 items.rs
//! 搬入本模块**——items.rs 只留类型编号/池/账本常数）+ shottype 表全家入驻同一
//! `&'static` 结构，`step`/`step_with_director` 按帧传参消费（T2 起 `&WorldTables` 已穿线；
//! 角色参数改道见 T3）。
//!
//! 传递形态（spec 拍板）：`&'static WorldTables` **不进 `World`**（I7 无引用；快照/校验和
//! 不覆盖——两机同表由二进制同一性 + 未来内容哈希保证，不由逐帧校验和保证）。
//! `content_hash` 字段本刀占位恒 0；文件加载刀（A3 双身份链）再实现真哈希。

use crate::items::{ITEM_POINT, ITEM_POWER, ITEM_TYPE_COUNT};
use crate::math::{Angle, Fx};
use crate::world::MAX_ENTITY_RADIUS;

/// 全局静态数据层（A3 骨架）。见模块文档「传递形态」。
pub struct WorldTables {
    /// 内容哈希占位（v0 恒 0）；文件加载刀实现，本刀二进制内建表靠编译期同一性代管。
    pub content_hash: u64,
    /// v0 一个角色；数组长度即角色数（编译期事实）。
    pub characters: [CharacterCfg; 1],
    pub item_cfg: [ItemTypeCfg; ITEM_TYPE_COUNT],
    pub drop_tables: &'static [&'static [(u8, u8)]],
    pub item_gravity: Fx,
    /// 弹外观表（M1 T3；ECL syscall `create_bullet` 按 id 查表填默认 radius/sprite，
    /// 显式参数可覆盖——见 spec §3.2.1「appearance 表」既定）。索引 = appearance id。
    pub appearances: &'static [AppearanceCfg],
}

/// 单条弹外观配置：默认判定半径 + 精灵号（M1 T3 新增；`ecl::syscall::SYS_CREATE_BULLET`
/// 消费）。
#[derive(Clone, Copy, Debug)]
pub struct AppearanceCfg {
    pub radius: Fx,
    pub sprite: u16,
}

/// 每类型道具配置（M0-17 T2 从 `items.rs` 迁入——结构体定义 + 内容全归此处；`items.rs`
/// 只留类型编号/池/账本常数）。
#[derive(Clone, Copy, Debug)]
pub struct ItemTypeCfg {
    pub score: u32,
    pub eject_speed: Fx,
    pub terminal_vy: Fx,
    pub magnet_speed: Fx,
    pub pickup_radius: Fx,
    pub attract_radius: Fx,
}

/// 单角色配置：移动参数（原 player.rs 常量）+ shottype 表。
#[derive(Clone, Copy, Debug)]
pub struct CharacterCfg {
    pub high_speed: Fx,
    pub low_speed: Fx,
    pub inv_sqrt2: Fx,
    pub hit_radius: Fx,
    pub graze_radius: Fx,
    pub shot: ShotTypeCfg,
}

/// shottype 表（ZUN `.sht` 类似物）：5 档火力 × 2 焦点态 = 10 槽（现代作组织同款）。
#[derive(Clone, Copy, Debug)]
pub struct ShotTypeCfg {
    /// `[tier 0..=4][focus 0/1]`；v0 高低速两槽共享同一列表（`std::ptr::eq` 判同）。
    pub sets: [[&'static [Shooter]; 2]; 5],
    /// 每档位子机偏移（`option` 号 1..=len 查此表；v0 前四档空表）。
    pub option_pos: [&'static [(Fx, Fx)]; 5],
}

/// 单路发射器：基础十字段（全定点/整数；spec 拍板 4，冻结）。
#[derive(Clone, Copy, Debug)]
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
    /// 0 = 本体；1..=该档 `option_pos` 长度 = 子机号（表校验 ≤ 长度）。
    pub option: u8,
    /// 预留；bit0 = homing（本刀不解析，见 `docs/follow-ups.md`）。
    pub flags: u8,
}

// ── v0 shottype 内容：发射节奏抄现状（interval=4 对齐旧 `SHOT_CD_FRAMES`）保 T4 前后可比 ──

/// 公共基线：直上一路点射；各档在此基础上覆写 `dx`/`option`。
const BASE_SHOOTER: Shooter = Shooter {
    interval: 4,
    delay: 0,
    dx: Fx::ZERO,
    dy: Fx::ZERO,
    angle: Angle(49152), // 直上（BAM 3π/2）
    speed: Fx::from_int(12),
    damage: 1,
    radius: Fx::from_int(4),
    sprite: 0,
    option: 0,
    flags: 0,
};

/// tier 0-1：1 路直射（`dx = 0`）。
static TIER_1WAY: [Shooter; 1] = [BASE_SHOOTER];

/// tier 2-3：2 路（`dx = ∓8px`）。
static TIER_2WAY: [Shooter; 2] = [
    Shooter {
        dx: Fx::from_int(-8),
        ..BASE_SHOOTER
    },
    Shooter {
        dx: Fx::from_int(8),
        ..BASE_SHOOTER
    },
];

/// tier 4：3 路本体（`dx = -12/0/12px`）+ 1 路子机（`option = 1`，出生点见
/// `TIER4_OPTION_POS`）。
static TIER_4WAY: [Shooter; 4] = [
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
];

/// tier 4 子机出生偏移：唯一非空档（自机位左后 -20px / 上方 8px）。
static TIER4_OPTION_POS: [(Fx, Fx); 1] = [(Fx::from_int(-20), Fx::from_int(8))];

/// 0-3 档无子机——空表。
const EMPTY_OPTION_POS: &[(Fx, Fx)] = &[];

// ── 道具三件（M0-17 T2 从 items.rs 迁入；逐字节抄现值，零行为搬家）────────────

/// 全局重力（≈0.15 px/帧²；未锁定道具 vy += 至终速钉住）。
const ITEM_GRAVITY_V0: Fx = Fx::from_raw(9_830);

const STD_ITEM: ItemTypeCfg = ItemTypeCfg {
    score: 0, // 各行覆写
    eject_speed: Fx::from_int(3),
    terminal_vy: Fx::from_raw(144_179), // ≈2.2
    magnet_speed: Fx::from_int(8),
    pickup_radius: Fx::from_int(16),
    attract_radius: Fx::from_int(40),
};

// ── 弹外观表 v0（M1 T3；≥4 行：小/中/大/星形，半径 3/4/6/8px）─────────────────
pub const APPEARANCE_SMALL: u16 = 0;
pub const APPEARANCE_MEDIUM: u16 = 1;
pub const APPEARANCE_LARGE: u16 = 2;
pub const APPEARANCE_STAR: u16 = 3;

const APPEARANCES_V0: &[AppearanceCfg] = &[
    AppearanceCfg {
        radius: Fx::from_int(3),
        sprite: 0,
    }, // small
    AppearanceCfg {
        radius: Fx::from_int(4),
        sprite: 1,
    }, // medium
    AppearanceCfg {
        radius: Fx::from_int(6),
        sprite: 2,
    }, // large
    AppearanceCfg {
        radius: Fx::from_int(8),
        sprite: 3,
    }, // star-ish
];

/// 索引 = 类型编号；数组类型使"表长 == 类型数"成为编译期事实。
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
    }, // STAR（消弹转化；grill 2026-07-18 拍板 30 分）
];

/// 掉落表 v0：表 id → [(类型, 数量)]。表 0 = 空（enemy.drop_table 零默认 = 不掉）。
const DROP_TABLES_V0: &[&[(u8, u8)]] = &[
    &[],
    &[(ITEM_POWER, 2), (ITEM_POINT, 1)], // 表 1：标准杂鱼
];

/// character-0 的 shottype 表：tier0/1 共享 1 路、tier2/3 共享 2 路、tier4 独立
/// 3 路+子机；每档两焦点槽指同一列表（v0 简化：高低速不分化）。
const CHARACTER0_SHOT: ShotTypeCfg = ShotTypeCfg {
    sets: [
        [&TIER_1WAY, &TIER_1WAY], // tier 0
        [&TIER_1WAY, &TIER_1WAY], // tier 1
        [&TIER_2WAY, &TIER_2WAY], // tier 2
        [&TIER_2WAY, &TIER_2WAY], // tier 3
        [&TIER_4WAY, &TIER_4WAY], // tier 4
    ],
    option_pos: [
        EMPTY_OPTION_POS,
        EMPTY_OPTION_POS,
        EMPTY_OPTION_POS,
        EMPTY_OPTION_POS,
        &TIER4_OPTION_POS,
    ],
};

// ── character-0 移动/判定参数（M0-17 T3 从 player.rs 迁入；逐字节抄现值，零行为搬家）──

const CHAR0_HIGH_SPEED: Fx = Fx::from_raw(294_912); // 4.5 px/帧
const CHAR0_LOW_SPEED: Fx = Fx::from_raw(131_072); // 2.0 px/帧
const CHAR0_INV_SQRT2: Fx = Fx::from_raw(46_341); // 0.7071（对角归一）
const CHAR0_HIT_RADIUS: Fx = Fx::from_raw(163_840); // 2.5 px
const CHAR0_GRAZE_RADIUS: Fx = Fx::from_int(16);

/// v0 全内容表（编译进二进制；`content_hash` 占位 0）。角色参数/道具三件逐字节抄现值。
pub static TABLES_V0: WorldTables = WorldTables {
    content_hash: 0,
    characters: [CharacterCfg {
        high_speed: CHAR0_HIGH_SPEED,
        low_speed: CHAR0_LOW_SPEED,
        inv_sqrt2: CHAR0_INV_SQRT2,
        hit_radius: CHAR0_HIT_RADIUS,
        graze_radius: CHAR0_GRAZE_RADIUS,
        shot: CHARACTER0_SHOT,
    }],
    item_cfg: ITEM_CFG_V0,
    drop_tables: DROP_TABLES_V0,
    item_gravity: ITEM_GRAVITY_V0,
    appearances: APPEARANCES_V0,
};

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
                    for shooter in c.shot.sets[tier][focus] {
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

    /// 形状 + v0 内容量拍板：5 档×2 态槽全非悬垂、同档两焦点槽指同一列表
    /// （`std::ptr::eq`）、tier0 一路 / tier2 两路 / tier4 三路+恰一个子机 shooter。
    #[test]
    fn tables_v0_shape() {
        let shot = &TABLES_V0.characters[0].shot;

        for tier in 0..5 {
            let [a, b] = shot.sets[tier];
            assert!(!a.is_empty(), "tier {tier} focus0 非悬垂空表");
            assert!(
                std::ptr::eq(a, b),
                "tier {tier} 两焦点槽必须指同一列表（v0 简化）"
            );
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
            TABLES_V0.drop_tables[1],
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
            let mut character = TABLES_V0.characters[0];
            character.shot = shot;
            WorldTables {
                content_hash: 0,
                characters: [character],
                item_cfg: ITEM_CFG_V0,
                drop_tables: DROP_TABLES_V0,
                item_gravity: ITEM_GRAVITY_V0,
                appearances: APPEARANCES_V0,
            }
        }

        // interval=0：tier0 换成一个 interval 为 0 的坏 shooter。
        {
            static BAD_INTERVAL: [Shooter; 1] = [Shooter {
                interval: 0,
                ..BASE_SHOOTER
            }];
            let mut shot = CHARACTER0_SHOT;
            shot.sets[0] = [&BAD_INTERVAL, &BAD_INTERVAL];
            assert!(
                !bad_with_shot(shot).validate(),
                "interval=0 必须被 validate 拒绝"
            );
        }

        // radius 超上限：MAX_ENTITY_RADIUS = 1024px，给 2000px。
        {
            static BAD_RADIUS: [Shooter; 1] = [Shooter {
                radius: Fx::from_int(2000),
                ..BASE_SHOOTER
            }];
            let mut shot = CHARACTER0_SHOT;
            shot.sets[0] = [&BAD_RADIUS, &BAD_RADIUS];
            assert!(
                !bad_with_shot(shot).validate(),
                "radius 超 MAX_ENTITY_RADIUS 必须被 validate 拒绝"
            );
        }

        // option 号越界：tier0 的 option_pos[0] 是空表，option=1 越界（1 > 0）。
        {
            static BAD_OPTION: [Shooter; 1] = [Shooter {
                option: 1,
                ..BASE_SHOOTER
            }];
            let mut shot = CHARACTER0_SHOT;
            shot.sets[0] = [&BAD_OPTION, &BAD_OPTION];
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
        static BAD_APPEARANCES: &[AppearanceCfg] = &[AppearanceCfg {
            radius: Fx::from_int(2000), // 超 MAX_ENTITY_RADIUS(1024)
            sprite: 0,
        }];
        let bad = WorldTables {
            content_hash: 0,
            characters: TABLES_V0.characters,
            item_cfg: ITEM_CFG_V0,
            drop_tables: DROP_TABLES_V0,
            item_gravity: ITEM_GRAVITY_V0,
            appearances: BAD_APPEARANCES,
        };
        assert!(!bad.validate(), "appearance 半径超上限必须被 validate 拒绝");
    }
}
