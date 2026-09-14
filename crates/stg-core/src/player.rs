//! 自机（D6/A8）—— 世界侧确定性状态机。PlayerState + 角色常量 + character-0 火力（"shottype 类似物"）。

use crate::math::Fx;

// ── 生死状态（本块只用 ABSENT/ALIVE；其余待碰撞那块）──────────────────
pub const LIFE_ABSENT: u8 = 0; // 全零默认 = 不在场
pub const LIFE_ALIVE: u8 = 1;
pub const LIFE_DEATHWINDOW: u8 = 2; // 决死窗口（中弹后可停止救，deathstop）
// 3 退役（原 LIFE_RESPAWNING 场底重生，玩法刀 2026-09-14：死亡即遡行）：值不复用。
pub const LIFE_GAMEOVER: u8 = 4; // 命尽、不再重生
/// 跳躍中（时间机制内核刀 2026-09-07）：自机**缺席**——不动、不射、不用能力、不碰撞、
/// 不擦弹、不拾取、不可瞄；`state_timer` 从 `JUMP_FRAMES` 倒数到 0 回 ALIVE。
/// 不复用 `LIFE_ABSENT`：那个态连 C 组计时都跳过，本态需要倒计时。
pub const LIFE_JUMPING: u8 = 5;
pub const DEATHBOMB_WINDOW: u16 = 8; // 决死窗口帧
pub const RESPAWN_INVULN: u16 = 120; // 续关无敌帧（2 秒 @60Hz；玩法刀起唯一消费者是 try_continue）

/// 停止（时间停止）的固定时长（帧）。**引擎常量而非表数据**——它不在任何表里。若将来
/// "每个自机时停时长不同"成为内容需求，再迁进 `CharacterCfg`。
pub const TIMESTOP_FRAMES: u16 = 180;

/// 停止库存上限（gameplay-design §1）。三个入口钳它：碎片进位、`SYS_ADD_BOMBS`、`new_game_at`。
pub const STOP_STOCK_MAX: u8 = 5;

/// 停止中触碰消弹每颗的得分（gameplay-design §5）。
pub const STOP_TOUCH_SCORE: u64 = 10;

/// 跳躍跨过的帧数（时间机制内核刀 spec §2.2）。也是宿主影子世界（観測）的预览步数——
/// 影子 = 克隆 + 喂一帧 `BTN_JUMP` + step 本数，与真跳同一条代码路径。先 30（0.5 s），
/// 手感要 1 s 再提 60。
pub const JUMP_FRAMES: u16 = 30;
/// 跳躍冷却（帧，gameplay-design §2）：落地那帧写满，C 组逐帧减，`== 0` 才能再跳。
pub const JUMP_COOLDOWN: u16 = 600;
/// 遡行落地后的无敌帧（spec §2.3）——策划案 2.3「落点附加短暂无敌帧」那条退路的实现，
/// 由 `WorldBody::rewind_landed` 与 `commit_death`（无 timeline 宿主的原地继续）写入（玩法刀）。
pub const REWIND_INVULN: u16 = 30;

// ── 角色配置：M0-17 T3 起移速/半径五常量已迁 `crate::tables::CharacterCfg`
// （`TABLES_V0.characters[..]`）；`PlayerState::spawn` 从表取值，`update_players` 移动逻辑
// 读 `tables.characters[character_id]`。行 1/2/3（弹×自机中弹、弹×自机擦弹、敌体×自机中弹）
// 的**被动**操作数就是 `hit_radius`/`graze_radius` 两半径——它们不经任何 `create_*` 写 API
// 的双边钳制，上限校验已由 `WorldTables::validate()`（`radius_in_range`）接管，取代原编译期
// 断言（T1 起）。

/// 自机状态（D6 全字段；本块仅移动 + 发弹活跃，余字段随快照/入校验和）。
#[repr(C)]
#[derive(Clone, Copy, Default, crate::checksum::Checksum, crate::save::SaveBytes)]
pub struct PlayerState {
    pub x: Fx,
    pub y: Fx,
    pub character_id: u8,
    pub facing: i8, // 纯表现，照样入校验和（P6）
    pub hit_radius: Fx,
    pub graze_radius: Fx,
    pub input: u32,
    /// 上一帧的原始动作位（相位 1 `decode_input` 滚存）。**沿检测的唯一原料**：
    /// `EDGE_MASK` 声明了哪些位是"沿"语义，但词表本身不做译码期沿处理（当帧原始电平
    /// 直接搬进 `input`）——真正的"按下瞬间"要靠对比 `input` 与 `prev_input` 求出，
    /// 见 `world/player.rs::WorldBody::pressed_edge`。随快照/回滚（P6 全量入校验和，
    /// 无例外）——rollback 后重放沿检测必须逐位一致，否则重演会在错误的帧上补触发。
    pub prev_input: u32,
    pub life_state: u8,
    pub state_timer: u16,
    pub invuln: u16,
    /// 跳躍冷却剩余帧（玩法刀）。落地写 `JUMP_COOLDOWN`，C 组计时（停止冻结期间不走）。
    pub jump_cd: u16,
    /// 发弹相位计时器（M0-17 T4：取代旧 `shot_cd` 倒计时）：持 SHOT 逐帧 `wrapping_add(1)`，
    /// 松手清零；相位 3 解释器用**自增前**的值判 `shot_timer % interval == delay % interval`
    /// （先判后加——见 `world/player.rs::char0_update_shot` 钉死注记 + 判别测试
    /// `shot_timer_phase_and_release_reset`）。
    pub shot_timer: u16,
    /// 火力，单位 = 0.01（厘火力）：0..=`POWER_MAX`(400) ↔ 显示 0.00-4.00（M0-16 定标）。
    pub power: u16,
    pub lives: u8,
    pub bombs: u8,
    pub life_pieces: u8,
    pub bomb_pieces: u8,
    pub score: u64,
    pub graze: u32,
    /// 进入决死窗口那一帧的 `World.frame`（时间机制内核刀）。`EVT_REWIND_REQUESTED` 把它带
    /// 给 timeline 算遡行落点（`hit_frame − REWIND_DEPTH`）。ALIVE 期间保留上次值，无消费者。
    pub hit_frame: u32,
    /// 续关次数（壳子刀 2026-09-11）：`try_continue` 饱和加一；`== 0` 通关 = 不续关通关
    /// （策划案 4.7 的 EX 解锁判据）。东方惯例续关后 `score = continues`。
    pub continues: u8,
    /// 偏差值 = 本局死亡次数（玩法刀，gameplay-design §3）。纯叙事计数，不进任何战斗数值；
    /// `commit_death` 与 `rewind_landed` 各加一（前者在死分支、后者在恢复出的世界），续关不清。
    pub deaths: u8,
}

/// 开局装备面（整局流程刀 spec §2.2/§3）——回放/握手身份组成部分之一
/// （`(seed, rank, start, loadout, image_hash)`，见 `step::new_game_at`）。
/// `power` 由消费方（`new_game_at`）钳到 [`crate::items::POWER_MAX`]；`lives`/`bombs`
/// 是 `u8` 全域即合法域——引擎不为它们造上限常量（比赛规则/关卡设计的事，非引擎不变量）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Loadout {
    pub character: u8,
    pub power: u16,
    pub lives: u8,
    pub bombs: u8,
}

impl Default for Loadout {
    /// 正典默认 = 机体0/0火力/3残/2停止——`PlayerState::spawn` 的硬编码收编为此单一来源。
    fn default() -> Self {
        Loadout {
            character: 0,
            power: 0,
            lives: 3,
            bombs: 2,
        }
    }
}

impl PlayerState {
    /// 出场初值（场底中心，Alive，3 命 3 弹）。判定/擦弹半径从角色配置表取
    /// （M0-17 T3：迁表零行为搬家，值逐位同源）。默认装备（火力/命/雷）收编自
    /// [`Loadout::default`]（整局流程刀 spec §2.2）——单一来源，改默认值只改那一处。
    pub fn spawn(character_id: u8, cfg: &crate::tables::CharacterCfg) -> Self {
        let ld = Loadout::default();
        PlayerState {
            x: Fx::ZERO,
            y: Fx::from_int(384),
            character_id,
            facing: 0,
            hit_radius: cfg.hit_radius,
            graze_radius: cfg.graze_radius,
            input: 0,
            prev_input: 0,
            life_state: LIFE_ALIVE,
            state_timer: 0,
            invuln: 0,
            jump_cd: 0,
            hit_frame: 0,
            continues: 0,
            deaths: 0,
            shot_timer: 0,
            power: ld.power,
            lives: ld.lives,
            bombs: ld.bombs,
            life_pieces: 0,
            bomb_pieces: 0,
            score: 0,
            graze: 0,
        }
    }

    /// 火力整数档位 0..=4（M0-16 定标：`power`/100 向下取整，**上钳 4**）。换弹幕形态/
    /// 张数的阈值语义——0.99 仍是 0 档、1.00 起跳 1 档、满 4.00 = 4 档。世界侧唯一
    /// 授权的档位换算入口（表现层/将来的火力接线都从这走，不自行除 100）。
    ///
    /// 上钳是 P4-b：`power` 是 pub 字段，导演/ECL 直写超 `POWER_MAX` 的非常规值时，
    /// 档位钳到满档而非让 shottype 表 `sets[tier]` 越界 panic（M0-17 终审实证：
    /// power=500 未钳时 release 下 index OOB——确定性安全结果优先于炸）。
    #[inline]
    pub fn power_tier(&self) -> u8 {
        ((self.power / 100) as u8).min(4)
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
        p.power = 999; // 越 POWER_MAX 的非常规直写（P4-b）
        assert_eq!(p.power_tier(), 4, "越界火力钳到满档，不越 sets 表界");
    }

    #[test]
    fn default_loadout_starts_with_two_stops() {
        assert_eq!(Loadout::default().bombs, 2, "gameplay-design §1：初始 2");
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
