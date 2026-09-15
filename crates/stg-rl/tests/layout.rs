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

#[test]
fn offset_consts_agree_with_field_tables() {
    use stg_rl::layout::*;
    let find = |t: &TableDef, n: &str| t.fields.iter().find(|f| f.name == n).unwrap().off;
    assert_eq!(find(&PLAYER, "score"), off::player::SCORE);
    assert_eq!(find(&BULLETS, "type"), off::bullet::TYPE);
    assert_eq!(find(&ENEMIES, "id"), off::enemy::ID);
    assert_eq!(find(&ITEMS, "flags"), off::item::FLAGS);
    for t in TABLES {
        for f in t.fields {
            assert!(f.off + f.ty.size() <= t.stride, "{}.{}", t.name, f.name);
        }
    }
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
