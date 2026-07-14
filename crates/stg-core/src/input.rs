//! 输入抽象（§5）—— 断层线边界类型（POD，无 godot/浮点）。键位绑定在表现层，模拟核只见动作位。
//! Phase 1 住 stg-core；将来可拆 stg-input crate（同 stg-net@M4）。

/// 一人一帧的量化动作位（4B）。`parameters` 数组待将来传参需求再加。
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct ActionInput {
    pub buttons: u16,
    pub _pad: u16,
}

/// 一帧的全体输入（§5）。**不进 World 校验和**（外部输入，非世界状态；
/// 译码后的 `players[i].input` 才入校验和）。
#[repr(C)]
#[derive(Clone, Copy)]
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

// ── 按钮位约定（半冻结；表现层须同意）。坐标 y 向下为正，UP = 减 y。─────────
pub const BTN_UP: u16 = 1 << 0;
pub const BTN_DOWN: u16 = 1 << 1;
pub const BTN_LEFT: u16 = 1 << 2;
pub const BTN_RIGHT: u16 = 1 << 3;
pub const BTN_SHOT: u16 = 1 << 4;
pub const BTN_BOMB: u16 = 1 << 5;
pub const BTN_SLOW: u16 = 1 << 6;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buttons_distinct() {
        let all = [
            BTN_UP, BTN_DOWN, BTN_LEFT, BTN_RIGHT, BTN_SHOT, BTN_BOMB, BTN_SLOW,
        ];
        let or: u16 = all.iter().fold(0, |a, &b| a | b);
        assert_eq!(or.count_ones() as usize, all.len()); // 互不重叠
    }

    #[test]
    fn empty_is_no_op() {
        let f = InputFrame::empty(5);
        assert_eq!(f.frame, 5);
        assert_eq!(f.actions[0].buttons, 0);
    }
}
