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

// ── 角色配置（暂 const；WorldTables 角色配置表将来接管）────────────────
pub const HIGH_SPEED: Fx = Fx::from_raw(294_912); // 4.5 px/帧
pub const LOW_SPEED: Fx = Fx::from_raw(131_072); // 2.0 px/帧
pub const INV_SQRT2: Fx = Fx::from_raw(46_341); // 0.7071（对角归一）
pub const HIT_RADIUS: Fx = Fx::from_raw(163_840); // 2.5 px
pub const GRAZE_RADIUS: Fx = Fx::from_int(16);
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
    pub input: u16,
    pub life_state: u8,
    pub state_timer: u16,
    pub invuln: u16,
    pub bomb_phase: u8,
    pub bomb_timer: u16,
    pub shot_cd: u8,
    pub power: u16,
    pub lives: u8,
    pub bombs: u8,
    pub life_pieces: u8,
    pub bomb_pieces: u8,
    pub score: u64,
    pub graze: u32,
}

impl PlayerState {
    /// 出场初值（场底中心，Alive，3 命 3 弹）。
    pub fn spawn(character_id: u8) -> Self {
        PlayerState {
            x: Fx::ZERO,
            y: Fx::from_int(384),
            character_id,
            facing: 0,
            hit_radius: HIT_RADIUS,
            graze_radius: GRAZE_RADIUS,
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
}
