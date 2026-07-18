//! 自机（D6/A8）—— 世界侧确定性状态机。PlayerState + 角色常量 + character-0 火力（"shottype 类似物"）。

use crate::math::Fx;

// ── 生死状态（本块只用 ABSENT/ALIVE；其余待碰撞那块）──────────────────
pub const LIFE_ABSENT: u8 = 0; // 全零默认 = 不在场
pub const LIFE_ALIVE: u8 = 1;
pub const LIFE_DEATHWINDOW: u8 = 2; // 决死窗口（中弹后可 bomb 救）
pub const LIFE_RESPAWNING: u8 = 3; // 场底重生、无敌
pub const LIFE_GAMEOVER: u8 = 4; // 命尽、不再重生
pub const DEATHBOMB_WINDOW: u16 = 8; // 决死窗口帧
pub const RESPAWN_INVULN: u16 = 120; // 重生无敌帧（2 秒 @60Hz）

// ── 角色配置：M0-17 T3 起移速/半径五常量已迁 `crate::tables::CharacterCfg`
// （`TABLES_V0.characters[..]`）；`PlayerState::spawn` 从表取值，`update_players` 移动逻辑
// 读 `tables.characters[character_id]`。行 1/2/3（弹×自机中弹、弹×自机擦弹、敌体×自机中弹）
// 的**被动**操作数就是 `hit_radius`/`graze_radius` 两半径——它们不经任何 `create_*` 写 API
// 的双边钳制，上限校验已由 `WorldTables::validate()`（`radius_in_range`）接管，取代原编译期
// 断言（T1 起）。

pub const SHOT_SPEED: Fx = Fx::from_int(12);
pub const SHOT_RADIUS: Fx = Fx::from_int(4);
pub const SHOT_CD_FRAMES: u8 = 4;
pub const SHOT_DAMAGE: u16 = 1;

/// 自机状态（D6 全字段；本块仅移动 + 发弹活跃，余字段随快照/入校验和）。
#[repr(C)]
#[derive(Clone, Copy, Default, crate::checksum::Checksum)]
pub struct PlayerState {
    pub x: Fx,
    pub y: Fx,
    pub character_id: u8,
    pub facing: i8, // 纯表现，照样入校验和（P6）
    pub hit_radius: Fx,
    pub graze_radius: Fx,
    pub input: u32,
    pub life_state: u8,
    pub state_timer: u16,
    pub invuln: u16,
    pub bomb_phase: u8,
    pub bomb_timer: u16,
    pub shot_cd: u8,
    /// 火力，单位 = 0.01（厘火力）：0..=`POWER_MAX`(400) ↔ 显示 0.00-4.00（M0-16 定标）。
    pub power: u16,
    pub lives: u8,
    pub bombs: u8,
    pub life_pieces: u8,
    pub bomb_pieces: u8,
    pub score: u64,
    pub graze: u32,
}

impl PlayerState {
    /// 出场初值（场底中心，Alive，3 命 3 弹）。判定/擦弹半径从角色配置表取
    /// （M0-17 T3：迁表零行为搬家，值逐位同源）。
    pub fn spawn(character_id: u8, cfg: &crate::tables::CharacterCfg) -> Self {
        PlayerState {
            x: Fx::ZERO,
            y: Fx::from_int(384),
            character_id,
            facing: 0,
            hit_radius: cfg.hit_radius,
            graze_radius: cfg.graze_radius,
            input: 0,
            life_state: LIFE_ALIVE,
            state_timer: 0,
            invuln: 0,
            bomb_phase: 0,
            bomb_timer: 0,
            shot_cd: 0,
            power: 0,
            lives: 3,
            bombs: 3,
            life_pieces: 0,
            bomb_pieces: 0,
            score: 0,
            graze: 0,
        }
    }

    /// 火力整数档位 0..=4（M0-16 定标：`power`/100 向下取整）。换弹幕形态/张数的
    /// 阈值语义——0.99 仍是 0 档、1.00 起跳 1 档、满 4.00 = 4 档。世界侧唯一
    /// 授权的档位换算入口（表现层/将来的火力接线都从这走，不自行除 100）。
    #[inline]
    pub fn power_tier(&self) -> u8 {
        (self.power / 100) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 火力定标钉死（M0-16）：满 4.00、一格 0.01、档位边界 0.99/1.00 判别。
    #[test]
    fn power_scale_and_tier_boundaries() {
        assert_eq!(crate::items::POWER_MAX, 400, "满火力 = 4.00（一格 0.01）");
        let mut p = PlayerState::spawn(0, &crate::tables::TABLES_V0.characters[0]);
        assert_eq!(p.power_tier(), 0);
        p.power = 99; // 0.99
        assert_eq!(p.power_tier(), 0, "0.99 仍 0 档");
        p.power = 100; // 1.00
        assert_eq!(p.power_tier(), 1, "1.00 起跳 1 档");
        p.power = 399; // 3.99
        assert_eq!(p.power_tier(), 3);
        p.power = crate::items::POWER_MAX; // 4.00
        assert_eq!(p.power_tier(), 4, "满火力 4 档");
    }

    /// 迁表回归（M0-17 T3）：`spawn` 判定/擦弹半径与 `TABLES_V0` 表值逐位相等——
    /// 零行为搬家的判别腿（若 `spawn` 漏接表值/接错角色槽，本测试就会红）。
    #[test]
    fn spawn_radii_match_tables_v0_bitwise() {
        let cfg = &crate::tables::TABLES_V0.characters[0];
        let p = PlayerState::spawn(0, cfg);
        assert_eq!(p.hit_radius, cfg.hit_radius, "hit_radius 逐位同源");
        assert_eq!(p.graze_radius, cfg.graze_radius, "graze_radius 逐位同源");
    }
}
