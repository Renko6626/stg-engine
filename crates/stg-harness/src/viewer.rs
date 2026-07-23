//! viewer —— WebSocket 实时查看器（spec 2026-07-23）：线格式 v1 编码 + 输入映射（本文件）
//! + serve 服务（T2）。断层线以上；数据只经 stg-core 既有读出口，core 零改动。

use stg_core::World;
use stg_core::input::InputFrame;

// 本刀（刀 1/2）只落编码器/输入映射，尚无 serve 调用点——production 构建下暂时只被
// `#[cfg(test)] mod tests` 用到。`cfg_attr(not(test), allow(dead_code))` 是仓库既有先例
// （`stg_core::step::spawn_sub_internal`，同款"待下一刀接线"处境），刀 2（serve）接线后
// 这几个 allow 即可摘掉。
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const WIRE_VERSION: u8 = 1;

/// 扫存活位字逐 index 回调（A9 批量消费姿势；尾位超 cap 部分核侧保证为零）。
#[cfg_attr(not(test), allow(dead_code))]
fn for_each_alive(words: &[u64], mut f: impl FnMut(usize)) {
    for (wi, &w) in words.iter().enumerate() {
        let mut bits = w;
        while bits != 0 {
            f(wi * 64 + bits.trailing_zeros() as usize);
            bits &= bits - 1;
        }
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn alive_count(words: &[u64]) -> u32 {
    words.iter().map(|w| w.count_ones()).sum()
}

/// 通道 A/B 快照 → 线格式 v1（spec §2.3：小端、顺序拼接、无填充）。
///
/// 玩家段的生死/无敌两字段实名为 `PlayerState::life_state`/`invuln`（brief 草稿写
/// `life`/`invuln`，`life` 按实况对齐为 `life_state`，`invuln` 本就同名——机械对齐，不算
/// 偏离；`life_state` 语义见 `stg_core::player` 的 `LIFE_*` 常量）。
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn encode_frame(w: &World) -> Vec<u8> {
    let v = w.view();
    let mut out = Vec::with_capacity(16 * 1024);
    out.push(WIRE_VERSION);
    out.extend_from_slice(&w.frame().to_le_bytes());

    // 玩家段（v1 恒 1 条：玩家 0）
    out.push(1u8);
    let p = &v.players()[0];
    out.extend_from_slice(&p.x.raw().to_le_bytes());
    out.extend_from_slice(&p.y.raw().to_le_bytes());
    out.push(p.life_state);
    out.extend_from_slice(&p.invuln.to_le_bytes());

    // boss 公告板两槽
    out.push(2u8);
    for slot in &w.body.boss_ui {
        out.push(slot.active);
        out.extend_from_slice(&slot.hp_ratio.raw().to_le_bytes());
        out.extend_from_slice(&slot.spell_id.to_le_bytes());
        out.extend_from_slice(&slot.timer_frames.to_le_bytes());
    }

    // 弹池（x,y,sprite = 10 B/条）
    let b = v.bullets();
    out.extend_from_slice(&(alive_count(b.alive_words()) as u16).to_le_bytes());
    for_each_alive(b.alive_words(), |i| {
        out.extend_from_slice(&b.x()[i].raw().to_le_bytes());
        out.extend_from_slice(&b.y()[i].raw().to_le_bytes());
        out.extend_from_slice(&b.sprite()[i].to_le_bytes());
    });

    // 自机弹（同 10 B）
    let s = v.shots();
    out.extend_from_slice(&(alive_count(s.alive_words()) as u16).to_le_bytes());
    for_each_alive(s.alive_words(), |i| {
        out.extend_from_slice(&s.x()[i].raw().to_le_bytes());
        out.extend_from_slice(&s.y()[i].raw().to_le_bytes());
        out.extend_from_slice(&s.sprite()[i].to_le_bytes());
    });

    // 敌（x,y,sprite,hp_pct = 11 B）
    let e = v.enemies();
    out.extend_from_slice(&(alive_count(e.alive_words()) as u16).to_le_bytes());
    for_each_alive(e.alive_words(), |i| {
        out.extend_from_slice(&e.x()[i].raw().to_le_bytes());
        out.extend_from_slice(&e.y()[i].raw().to_le_bytes());
        out.extend_from_slice(&e.sprite()[i].to_le_bytes());
        let pct = (e.hp()[i].max(0) as i64 * 255) / (e.hp_max()[i].max(1) as i64);
        out.push(pct.min(255) as u8);
    });

    // 道具（x,y,item_type = 9 B）
    let it = v.items();
    out.extend_from_slice(&(alive_count(it.alive_words()) as u16).to_le_bytes());
    for_each_alive(it.alive_words(), |i| {
        out.extend_from_slice(&it.x()[i].raw().to_le_bytes());
        out.extend_from_slice(&it.y()[i].raw().to_le_bytes());
        out.push(it.item_type()[i]);
    });

    // 通道 B（id,seq,args[6] = 28 B）
    let reqs = w.take_requests();
    out.extend_from_slice(&(reqs.len() as u16).to_le_bytes());
    for r in reqs {
        out.extend_from_slice(&r.id.to_le_bytes());
        out.extend_from_slice(&r.seq.to_le_bytes());
        for a in r.args {
            out.extend_from_slice(&a.to_le_bytes());
        }
    }
    out
}

/// 浏览器 u32 掩码 → 玩家 0 InputFrame（位布局即 `BTN_*`，spec §2.4；
/// BOMB 的 Edge 语义由引擎 `decode_input` 自理，这里只送电平）。
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn mask_to_input(frame: u32, mask: u32) -> InputFrame {
    let mut input = InputFrame::empty(frame);
    input.actions[0].buttons = mask;
    input
}

#[cfg(test)]
mod tests {
    use super::*;
    use stg_core::input::{BTN_SHOT, BTN_SLOW, InputFrame};
    use stg_core::step;
    use stg_core::tables::TABLES_V0;

    fn read_u8(b: &[u8], o: &mut usize) -> u8 {
        let v = b[*o];
        *o += 1;
        v
    }
    fn read_u16(b: &[u8], o: &mut usize) -> u16 {
        let v = u16::from_le_bytes([b[*o], b[*o + 1]]);
        *o += 2;
        v
    }
    fn read_u32(b: &[u8], o: &mut usize) -> u32 {
        let v = u32::from_le_bytes(b[*o..*o + 4].try_into().unwrap());
        *o += 4;
        v
    }
    fn read_i32(b: &[u8], o: &mut usize) -> i32 {
        let v = i32::from_le_bytes(b[*o..*o + 4].try_into().unwrap());
        *o += 4;
        v
    }

    /// 开局静态帧逐字段手工解码（判别：错位/漏段/字节序错任一即红）。
    #[test]
    fn encode_frame_layout_v1_static_open() {
        let (w, _image, _boss) = crate::build_rainbow_world(42);
        let buf = encode_frame(&w);
        let mut o = 0;
        assert_eq!(read_u8(&buf, &mut o), WIRE_VERSION);
        assert_eq!(read_u32(&buf, &mut o), 0, "未 step，frame=0");
        assert_eq!(read_u8(&buf, &mut o), 1, "player_count");
        assert_eq!(read_i32(&buf, &mut o), w.view().players()[0].x.raw());
        assert_eq!(read_i32(&buf, &mut o), w.view().players()[0].y.raw());
        let _life = read_u8(&buf, &mut o);
        let _invuln = read_u16(&buf, &mut o);
        assert_eq!(read_u8(&buf, &mut o), 2, "boss_slots");
        o += 2 * 9; // 两槽逐字段由下一测的宽度对账兜底
        assert_eq!(read_u16(&buf, &mut o), 0, "开局零弹");
        assert_eq!(read_u16(&buf, &mut o), 0, "零自机弹");
        assert_eq!(read_u16(&buf, &mut o), 1, "唯 boss 一敌");
        let ex = read_i32(&buf, &mut o);
        let ey = read_i32(&buf, &mut o);
        let boss_i = w.view().enemies().iter_alive().next().unwrap();
        assert_eq!(ex, w.view().enemies().x()[boss_i].raw());
        assert_eq!(ey, w.view().enemies().y()[boss_i].raw());
        let _sprite = read_u16(&buf, &mut o);
        assert_eq!(read_u8(&buf, &mut o), 255, "hp==hp_max → 255");
        assert_eq!(read_u16(&buf, &mut o), 0, "零道具");
        assert_eq!(read_u16(&buf, &mut o), 0, "零请求");
        assert_eq!(o, buf.len(), "无多余尾字节");
    }

    /// 跑起来后：总长 = 头 + Σ(计数×记录宽) 精确对账（判别：任何记录宽错即红）。
    #[test]
    fn encode_frame_width_accounting_after_steps() {
        let (mut w, image, _boss) = crate::build_rainbow_world(42);
        for f in 0..240u32 {
            step(&mut w, &TABLES_V0, &image, &InputFrame::empty(f));
        }
        let v = w.view();
        let (nb, ns, ne, ni) = (
            v.bullets().iter_alive().count(),
            v.shots().iter_alive().count(),
            v.enemies().iter_alive().count(),
            v.items().iter_alive().count(),
        );
        let nr = w.take_requests().len();
        assert!(nb > 0, "240 帧风铃卡应已起弹（场景前提）");
        let buf = encode_frame(&w);
        let expect = (1 + 4)
            + (1 + 11)
            + (1 + 18)
            + (2 + nb * 10)
            + (2 + ns * 10)
            + (2 + ne * 11)
            + (2 + ni * 9)
            + (2 + nr * 28);
        assert_eq!(buf.len(), expect);
    }

    #[test]
    fn mask_to_input_places_buttons_on_player0() {
        let input = mask_to_input(7, BTN_SHOT | BTN_SLOW);
        assert_eq!(input.actions[0].buttons, BTN_SHOT | BTN_SLOW);
        assert_eq!(input.actions[1].buttons, 0, "玩家 1 不受掩码影响");
    }
}
