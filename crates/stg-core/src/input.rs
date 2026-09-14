//! 输入抽象（§5）—— 断层线边界类型（POD，无 godot/浮点）。键位绑定在表现层，模拟核只见动作位。
//! Phase 1 住 stg-core；将来可拆 stg-input crate（同 stg-net@M4）。

/// 一人一帧的量化输入（8B，容量约定 v2）：32 动作位 + 32 预留位。
/// `_pad` 恒 0、先占座后赋义（量化参数/更多位的第一块地）——预留与动作位同宽，
/// 结构体无隐式填充，每个字节都有名字。
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct ActionInput {
    pub buttons: u32,
    pub _pad: u32,
}

/// 一帧的全体输入（§5）。**不进 World 校验和**（外部输入，非世界状态；
/// 译码后的 `players[i].input` 才入校验和）。
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct InputFrame {
    pub frame: u32,
    pub actions: [ActionInput; crate::MAX_PLAYERS],
}

impl InputFrame {
    /// 全零输入（无操作）。
    pub fn empty(frame: u32) -> Self {
        InputFrame {
            frame,
            actions: [ActionInput::default(); crate::MAX_PLAYERS],
        }
    }
}

// ── 动作词表注册处（唯一权威）────────────────────────────────────────
//
// 绑一对新「输入-效果」的全部动作：在下面的 `define_actions!` 里加一行 + 去声明的
// 消费相位写效果逻辑（自机语义 → 相位 3）+ 测试。位约定半冻结（表现层须同意）；
// **位=0 必须等价旧行为**——旧回放该位恒 0，加位才不是回放格式的 breaking change。
//
// 词表与玩家无关："玩家 1 左移" = `actions[0]` × `BTN_LEFT`——「哪个玩家」由
// `InputFrame.actions[]` 的槽位表达；物理设备/联机 peer 绑到哪个槽是表现层/会话层
// （M2/M4）的事，core 只见槽位。
//
// ── 参数化输入预案（未实现；第一个真实参数出现时按此落地）──────────────
// 1. 量化整数唯一制：参数在表现层量化成整数（BAM 角 / u8 强度 / 定点坐标）后才准
//    过断层线，浮点/模拟量原值永不入内（I1 延伸到输入）。
// 2. 存储台阶：第一块地 = `_pad` 32 位；不够 → `ActionInput` 加具名字段（M3 回放
//    格式出生前免费，之后 = 格式变更 + engine_ver bump）；**永远定长**，不做变长参数
//    （回放 = 帧数组 memcpy、rollback 重发窗口定长，此条不可让）。
// 3. 标记 = 本注册表扩参数列：`BTN_AIM = 7, Level, params: [aim_angle: Bam16 @ 0..16]`，
//    宏展开取参 accessor + `_pad` 位段不重叠编译期断言（动作位同款纪律）+ 参数描述
//    进 `actions_vocab_hash`（参数布局变更 = 指纹变 = 有意识的契约动作）。
// 4. 门位规则：每个参数隶属一个动作位，**位=0 时参数区必须为 0**——保住"全零帧 =
//    无操作"与"0 = 旧行为"两条兼容律对参数区的自动延伸；debug 断言押运。
// 5. 译码同构：参数解包进 PlayerState 具名字段（如 `aim: Angle`）→ 自动入校验和、
//    随快照回滚 → 效果逻辑从 world 状态读。与动作位同一条生命周期，rollback 免费。

/// 动作的触发语义：消费端按此选择读位方式。
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ActionKind {
    /// 电平：按住持续生效（移动/射击/低速）。
    Level = 0,
    /// 沿：一次按下=一次效果，消费端配 `prev_input` 沿检测（bomb）。
    Edge = 1,
}

/// 词表描述表的一行：名字 + 位号 + 触发语义。
pub struct ActionDesc {
    pub name: &'static str,
    pub bit: u8,
    pub kind: ActionKind,
}

/// 声明动作词表，展开出：`pub const $名字: u32` 位常量、`EDGE_MASK`（沿触发位并集）、
/// `ACTIONS` 描述表、位互不重叠的编译期断言。
macro_rules! define_actions {
    ( $( $(#[$doc:meta])* $name:ident = $bit:literal, $kind:ident; )+ ) => {
        $( $(#[$doc])* pub const $name: u32 = 1 << $bit; )+

        /// 全部沿触发位的并集——`prev_input` 沿检测的统一消费面。
        pub const EDGE_MASK: u32 = 0 $( | ((ActionKind::$kind as u32) * (1 << $bit)) )+;

        /// 动作词表总数。
        pub const ACTION_COUNT: usize = { let a = [$($bit as u8),+]; a.len() };

        /// 动作词表描述表（表即地图；`actions_vocab_hash` 的原料）。
        pub static ACTIONS: [ActionDesc; ACTION_COUNT] = [
            $( ActionDesc { name: stringify!($name), bit: $bit, kind: ActionKind::$kind }, )+
        ];

        // 位互不重叠（编译期钉死；重叠时并集 popcount < 词条数）。
        const _: () = assert!(
            (0u32 $( | (1 << $bit) )+).count_ones() as usize == ACTION_COUNT,
            "动作位重叠"
        );

        /// 词表实际用到的最高位号（编译期求值，容量哨兵的观测量）。
        pub const MAX_BIT_USED: u8 = {
            let bits = [$($bit as u8),+];
            let mut max = 0;
            let mut i = 0;
            while i < bits.len() {
                if bits[i] > max {
                    max = bits[i];
                }
                i += 1;
            }
            max
        };

        // ── 容量哨兵：词表溢出 u32 容器时在此编译失败。──────────────────
        // **故意不自动加宽**——容器宽度是线上格式（回放/网络包）的一部分，按词表
        // 自动派生会让"加一个动作"静默改格式。溢出必须是编译错误：升级容量 =
        // 有意识的约定变更（v2→v3：buttons 加宽或加字，M3 后属格式变更须 bump
        // engine_ver + 过评审）。
        const _: () = assert!(
            (MAX_BIT_USED as u32) < u32::BITS,
            "动作词表溢出 u32 容器：升级容量约定属线上格式变更，须过评审（见注册处注释）"
        );
    };
}

define_actions! {
    /// 上移。坐标 y 向下为正，UP = 减 y。
    BTN_UP = 0, Level;
    /// 下移。
    BTN_DOWN = 1, Level;
    /// 左移。
    BTN_LEFT = 2, Level;
    /// 右移。
    BTN_RIGHT = 3, Level;
    /// 射击（消费者：`world/player.rs::char0_update_shot`，`shot_cd` 整流为连发）。
    BTN_SHOT = 4, Level;
    /// 停止（沿触发；消费者：`world/player.rs::try_stop`）。玩法刀 2026-09-14：时停 + 触碰消弹合一，
    /// 库存 = `PlayerState.bombs`。位 7（旧 `BTN_TIMESTOP`）退役不复用。
    BTN_BOMB = 5, Edge;
    /// 低速（消费者：`world/player.rs::move_player`）。
    BTN_SLOW = 6, Level;
    /// 跳躍（沿触发；消费者：`world/player.rs::try_jump`）。时间机制内核刀 2026-09-07。
    /// 観測（未来预览）**不进世界**——它是宿主侧影子世界的纯表现，两次按键协议由宿主管，
    /// 宿主只在第二下把本位送进来一帧。
    BTN_JUMP = 8, Edge;
    /// 续关（沿触发；消费者：`world/player.rs::try_continue`）。壳子刀 2026-09-11：只在
    /// `LIFE_GAMEOVER` 下响应，世界内做续关（残机回默认、分数 = 续关次数、重生），可回放。
    BTN_CONTINUE = 10, Edge;
}

/// 词表指纹：FNV-1a 遍历 (name, bit, kind)。将来进回放头/联机握手，
/// 校验双方输入语义一致（烘焙表哈希同款纪律）。
pub fn actions_vocab_hash() -> u64 {
    let mut h = crate::checksum::Fnv1a64::new();
    for a in &ACTIONS {
        h.write_bytes(a.name.as_bytes());
        h.write_u8(0xFF); // 名字定界，防拼接歧义
        h.write_u8(a.bit);
        h.write_u8(a.kind as u8);
    }
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    // （原 buttons_distinct 运行时测试已被 define_actions! 的编译期断言取代。）

    #[test]
    fn empty_is_no_op() {
        let f = InputFrame::empty(5);
        assert_eq!(f.frame, 5);
        assert_eq!(f.actions[0].buttons, 0);
    }

    /// 容量约定 v2：每人 32 动作位 + 32 预留位，字节布局全显式（无隐式填充）。
    #[test]
    fn action_input_layout_v2() {
        use core::mem::size_of;
        let a = ActionInput::default();
        assert_eq!(core::mem::size_of_val(&a.buttons), 4, "动作位应为 u32");
        assert_eq!(
            core::mem::size_of_val(&a._pad),
            4,
            "预留位应为 u32（免隐式填充）"
        );
        assert_eq!(size_of::<ActionInput>(), 8, "8B/人，无编译器暗插填充");
        assert_eq!(size_of::<InputFrame>(), 4 + 8 * crate::MAX_PLAYERS);
        // 译码目的地必须同宽，否则位 16..32 在 decode 时被静默截断
        assert_eq!(
            core::mem::size_of_val(
                &crate::player::PlayerState::spawn(0, &crate::tables::TABLES_V0.characters[0])
                    .input
            ),
            4
        );
    }

    /// 位值冻结（回放契约）：位号是回放文件与将来网络包的语义坐标，注册表重排不得改值。
    #[test]
    fn action_bit_values_frozen() {
        assert_eq!(BTN_UP, 1 << 0);
        assert_eq!(BTN_DOWN, 1 << 1);
        assert_eq!(BTN_LEFT, 1 << 2);
        assert_eq!(BTN_RIGHT, 1 << 3);
        assert_eq!(BTN_SHOT, 1 << 4);
        assert_eq!(BTN_BOMB, 1 << 5);
        assert_eq!(BTN_SLOW, 1 << 6);
        // 位 7 退役（旧 BTN_TIMESTOP，玩法刀）：不复用
        assert_eq!(BTN_JUMP, 1 << 8);
        // 位 9 退役（旧 BTN_REWIND，玩法刀：死亡即遡行）：不复用
        assert_eq!(BTN_CONTINUE, 1 << 10);
    }

    /// 容量哨兵可观测面：词表当前最高位 = 10（BTN_CONTINUE），且在 u32 容器内。
    /// （溢出情形无法用运行时测试压——那是编译失败，由宏内 const 断言把守。）
    #[test]
    fn max_bit_used_pinned() {
        assert_eq!(MAX_BIT_USED, 10);
        assert!((MAX_BIT_USED as u32) < u32::BITS);
    }

    /// `EDGE_MASK` = 全部沿触发位的并集；当前词表中 BOMB / TIMESTOP / JUMP / REWIND 是沿语义。
    #[test]
    fn edge_mask_is_exactly_the_edge_actions() {
        assert_eq!(EDGE_MASK, BTN_BOMB | BTN_JUMP | BTN_CONTINUE);
    }

    /// ACTIONS 描述表与位常量逐项一致（表即地图：名字/位/语义三列齐全、顺序按位号）。
    #[test]
    fn actions_table_matches_constants() {
        let expected: [(&str, u32, ActionKind); 9] = [
            ("BTN_UP", BTN_UP, ActionKind::Level),
            ("BTN_DOWN", BTN_DOWN, ActionKind::Level),
            ("BTN_LEFT", BTN_LEFT, ActionKind::Level),
            ("BTN_RIGHT", BTN_RIGHT, ActionKind::Level),
            ("BTN_SHOT", BTN_SHOT, ActionKind::Level),
            ("BTN_BOMB", BTN_BOMB, ActionKind::Edge),
            ("BTN_SLOW", BTN_SLOW, ActionKind::Level),
            ("BTN_JUMP", BTN_JUMP, ActionKind::Edge),
            ("BTN_CONTINUE", BTN_CONTINUE, ActionKind::Edge),
        ];
        assert_eq!(ACTIONS.len(), expected.len());
        for (a, (name, mask, kind)) in ACTIONS.iter().zip(expected) {
            assert_eq!(a.name, name);
            assert_eq!(1u32 << a.bit, mask);
            assert_eq!(a.kind, kind);
        }
    }

    /// 词表指纹钉死：对 (name, bit, kind) 的 FNV-1a。词表任何增删改都必须换值——
    /// 这是将来回放头/联机握手校验"双方输入语义一致"的原料（烘焙表哈希同款纪律）。
    #[test]
    fn vocab_hash_pinned() {
        // 壳子刀：加 BTN_CONTINUE=10（Edge），指纹随之变化（实测值，非手算）。
        // 前一次：时间机制内核刀加 BTN_JUMP=8 / BTN_REWIND=9 → 0x6433_59C3_B41F_121D。
        // 玩法刀：删 BTN_TIMESTOP=7 与 BTN_REWIND=9 → 0xFE04_C7CD_7FE1_0485；前一次壳子刀 0x0786_DE58_5E72_3383。
        assert_eq!(actions_vocab_hash(), 0xFE04_C7CD_7FE1_0485); // 词表变更须有意识地更新此值
    }
}
