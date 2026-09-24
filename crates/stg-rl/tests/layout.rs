//! HELLO 布局对拍 proto C 夹具（spec §11）：除 backend/policy/cap/actions 之外逐项相等。
use serde_json::Value;

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/proto_v1_hello.json")).unwrap()
}

#[test]
fn tables_match_proto_fixture_except_cap() {
    let ours: Value = serde_json::from_str(&stg_rl::layout::hello_json(1024)).unwrap();
    let theirs = fixture();
    let (a, b) = (
        ours["tables"].as_array().unwrap(),
        theirs["tables"].as_array().unwrap(),
    );
    assert_eq!(a.len(), b.len());
    for (x, y) in a.iter().zip(b) {
        for k in ["id", "name", "stride", "fields"] {
            assert_eq!(x[k], y[k], "table {} key {k}", y["name"]);
        }
    }
}

#[test]
fn header_fields() {
    let v: Value = serde_json::from_str(&stg_rl::layout::hello_json(777)).unwrap();
    assert_eq!(v["proto"], 1);
    assert_eq!(v["backend"], format!("stg-engine@{}", stg_core::ENGINE_VER));
    assert_eq!(v["policy"], "rl");
    assert_eq!(v["obs_timing"], "prev-frame-final");
    assert_eq!(v["tick_hz"], 60);
    assert_eq!(v["field"], fixture()["field"]);
    assert_eq!(v["number"], fixture()["number"]);
    assert_eq!(
        v["actions"],
        fixture()["actions"],
        "位 0-6，与夹具 action_bits=0x7f 相同"
    );
    let caps: Vec<u64> = v["tables"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["cap"].as_u64().unwrap())
        .collect();
    assert_eq!(caps, vec![1, 777, 256, 64, 1024]);
}

/// 显式「字段名 → off 常量」映射：逐字段核对 `off::*` 与表内 `off` 相等。
/// 映射长度与表字段数互查，防漏；末段保留原「字段不出 stride」的口径。
fn assert_offsets(t: &stg_rl::layout::TableDef, map: &[(&str, usize)]) {
    assert_eq!(map.len(), t.fields.len(), "{}: 映射须覆盖全字段", t.name);
    for (name, off) in map {
        let f = t
            .fields
            .iter()
            .find(|f| f.name == *name)
            .unwrap_or_else(|| panic!("{}: 缺字段 {name}", t.name));
        assert_eq!(f.off, *off, "{}.{name} off 常量与表不符", t.name);
    }
    for f in t.fields {
        assert!(
            map.iter().any(|(n, _)| *n == f.name),
            "{}.{} 未在映射中",
            t.name,
            f.name
        );
        assert!(f.off + f.ty.size() <= t.stride, "{}.{}", t.name, f.name);
    }
}

#[test]
fn offset_consts_agree_with_field_tables() {
    use stg_rl::layout::*;
    assert_offsets(
        &PLAYER,
        &[
            ("x", off::player::X),
            ("y", off::player::Y),
            ("hit_radius", off::player::HIT_R),
            ("speed", off::player::SPEED),
            ("speed_focus", off::player::SPEED_F),
            ("focus", off::player::FOCUS),
            ("state", off::player::STATE),
            ("lives", off::player::LIVES),
            ("bombs", off::player::BOMBS),
            ("life_frags", off::player::LFRAG),
            ("bomb_frags", off::player::BFRAG),
            ("power", off::player::POWER),
            ("score", off::player::SCORE),
            ("graze", off::player::GRAZE),
        ],
    );
    assert_offsets(
        &BULLETS,
        &[
            ("x", off::bullet::X),
            ("y", off::bullet::Y),
            ("vx", off::bullet::VX),
            ("vy", off::bullet::VY),
            ("speed", off::bullet::SPEED),
            ("angle", off::bullet::ANGLE),
            ("radius", off::bullet::RADIUS),
            ("flags", off::bullet::FLAGS),
            ("state", off::bullet::STATE),
            ("type", off::bullet::TYPE),
        ],
    );
    assert_offsets(
        &ENEMIES,
        &[
            ("x", off::enemy::X),
            ("y", off::enemy::Y),
            ("hurt_w", off::enemy::HURT_W),
            ("hurt_h", off::enemy::HURT_H),
            ("hit_w", off::enemy::HIT_W),
            ("hit_h", off::enemy::HIT_H),
            ("hp", off::enemy::HP),
            ("hp_max", off::enemy::HP_MAX),
            ("flags", off::enemy::FLAGS),
            ("id", off::enemy::ID),
            ("vx", off::enemy::VX),
            ("vy", off::enemy::VY),
        ],
    );
    assert_offsets(
        &LASERS,
        &[
            ("x", off::laser::X),
            ("y", off::laser::Y),
            ("angle", off::laser::ANGLE),
            ("start", off::laser::START),
            ("end", off::laser::END),
            ("start_len", off::laser::START_LEN),
            ("speed", off::laser::SPEED),
            ("half_h", off::laser::HALF_H),
            ("omega", off::laser::OMEGA),
            ("vx", off::laser::VX),
            ("vy", off::laser::VY),
            ("t_active", off::laser::T_ACTIVE),
            ("state", off::laser::STATE),
            ("type", off::laser::TYPE),
        ],
    );
    assert_offsets(
        &ITEMS,
        &[
            ("x", off::item::X),
            ("y", off::item::Y),
            ("vx", off::item::VX),
            ("vy", off::item::VY),
            ("kind", off::item::KIND),
            ("flags", off::item::FLAGS),
        ],
    );

    assert_eq!(FieldType::U8.size(), 1);
    assert_eq!(FieldType::U16.size(), 2);
    assert_eq!(FieldType::Angle.size(), 2);
    assert_eq!(FieldType::Fx.size(), 4);
    assert_eq!(FieldType::U32.size(), 4);
    assert_eq!(FieldType::I32.size(), 4);
}

#[test]
fn bundled_game_pack_compiles() {
    let units: Vec<(String, String)> = stg_rl::bundled::bundled_sources("game")
        .unwrap()
        .iter()
        .map(|(n, s)| (n.to_string(), s.to_string()))
        .collect();
    assert_eq!(units.len(), 5);
    assert!(units.windows(2).all(|w| w[0].0 < w[1].0), "按文件名排序");
    stg_ecl_compiler::lang::compile_units(&units).expect("内置 game 包必须能编译");
    assert!(stg_rl::bundled::bundled_sources("nope").is_none());
}
