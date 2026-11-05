//! harness 的编译检查与整局脚本集成测试。

use stg_core::events::EVT_ITEM_PICKED;
use stg_core::input::{BTN_LEFT, BTN_UP, InputFrame};
use stg_core::step::step_with_director;

/// A1 行为不变性判别式:仅 item sprite 值不同的两份 owned 表(content_hash 同为 0)
/// → 同种子世界含道具全生命周期(掉落/下坠/磁吸/拾取)演化,逐帧校验和相等;
/// 序列化字节则必须不同(sprite 参与身份)。自机全程 `BTN_LEFT|BTN_UP`:自机从
/// 场底(y=384)持续上冲,越过 `POC_LINE_Y`(world.rs:109,=128)即触发全场道具磁吸
/// （对 x 坐标细节鲁棒,不依赖精调初始几何贴近)——真实走到 `settle.rs` 拾取结算
/// (`credit_item`,`item_cfg` 唯一的运行期读点),用 `EVT_ITEM_PICKED` 计数自证。
#[test]
fn item_sprite_values_never_affect_world_evolution() {
    const DROP_SRC: &str = r#"
sub main() {
loop {
    _ = drop_item(96.0fx, 64.0fx, 0);
    _ = drop_item(128.0fx, 64.0fx, 1);
    _ = drop_item(160.0fx, 64.0fx, 2);
    _ = drop_item(192.0fx, 64.0fx, 3);
    _ = drop_item(224.0fx, 64.0fx, 4);
    wait(40);
}
}
"#;
    let image = match stg_ecl_compiler::lang::compile(DROP_SRC, "drop.ecl") {
        Ok(i) => i,
        Err(errors) => {
            let msg: Vec<String> = errors.iter().map(|e| e.render("drop.ecl")).collect();
            panic!("drop.ecl 编译失败:\n{}", msg.join("\n\n"));
        }
    };

    let ta = stg_core::tables::build_tables_v0();
    let mut tb = stg_core::tables::build_tables_v0();
    for (i, cfg) in tb.item_cfg.iter_mut().enumerate() {
        cfg.sprite = 100 + i as u16; // 与 ta 的 0..=4 全互异
    }
    assert_ne!(ta.to_bytes(), tb.to_bytes(), "sprite 必须参与序列化身份");

    let mut wa = stg_core::step::World::new_with_tables(7, &ta);
    let mut wb = stg_core::step::World::new_with_tables(7, &tb);
    wa.start_main(&image).expect("main a");
    wb.start_main(&image).expect("main b");

    let mut picked = 0u32;
    for frame in 0..300u32 {
        let mut input = InputFrame::empty(frame);
        // 全程上冲+左移:上冲越过 POC 线触发全场磁吸(见上方文档字符串),
        // 左移保留原"自机移动,扰动拾取几何"意图。
        input.actions[0].buttons = BTN_LEFT | BTN_UP;
        step_with_director(&mut wa, &ta, &image, &input, |_| {});
        step_with_director(&mut wb, &tb, &image, &input, |_| {});
        assert_eq!(
            wa.checksum(),
            wb.checksum(),
            "frame {frame} 演化分歧:sprite 值泄漏进模拟"
        );
        picked += wa
            .body
            .frame_events()
            .iter()
            .filter(|e| e.kind == EVT_ITEM_PICKED)
            .count() as u32;
    }
    assert!(
        picked > 0,
        "场景必须真实走到拾取结算(settle.rs 的 item_cfg 读点),否则判别式空转"
    );
}

/// 端到端判别:四条表现锚点 builtin(add_score/bgm/bg/bg_phase)经编译→VM 执行,
/// 真落到 world 锚点字段(见 `world.rs` 的 `set_bgm`/`set_bg`/`set_bg_phase`/
/// `SYS_ADD_SCORE`)——不是"编译通过就算数",是读回字段值判别式。
#[test]
fn anchor_builtins_write_fields_and_reqs_via_script() {
    const SRC: &str = r#"
sub main() {
bgm(5);
bg(2);
bg_phase(1);
add_score(1000);
add_score(-2000);
loop { wait(60); }
}
"#;
    let image = stg_ecl_compiler::lang::compile(SRC, "anchors.ecl").expect("编译");
    let mut w = stg_core::step::World::new_game(7, 2, &image).expect("boot");
    let t = &stg_core::tables::TABLES_V0;
    // `InputFrame` 无 `Default`(见 input.rs);`frame` 字段不参与译码(decode_input
    // 只读 `actions`),`empty(0)` 与 spec 草稿的 `default()` 等价(机械调整许可)。
    let input = stg_core::input::InputFrame::empty(0);
    stg_core::step::step(&mut w, t, &image, &input); // 出生帧不跑(born_frame 门禁)
    stg_core::step::step(&mut w, t, &image, &input); // 次帧首跑:main 实际执行
    let v = w.body.view();
    assert_eq!(v.bgm_id(), 5);
    assert_eq!(v.bg_id(), 2);
    assert_eq!(v.bg_phase(), 1);
    assert_eq!(v.bg_phase_frame(), 1, "phase 帧戳 = 声明发生帧");
    assert_eq!(v.players()[0].score, 0, "1000-2000 饱和钳 0");
}

/// check 三路退码:OK=0 / 编译错=1(带行列) / 文件缺失=2。
#[test]
fn check_exit_codes_three_ways() {
    use std::process::ExitCode;
    let dir = std::env::temp_dir().join("ecl-check-test");
    std::fs::create_dir_all(&dir).unwrap();
    let ok = dir.join("ok.ecl");
    std::fs::write(&ok, "sub main() { loop { wait(60); } }").unwrap();
    let bad = dir.join("bad.ecl");
    std::fs::write(&bad, "sub main() { 这不是脚本 }").unwrap();
    assert_eq!(
        super::cmd_check(&[ok.to_string_lossy().into_owned()]),
        ExitCode::SUCCESS
    );
    assert_eq!(
        super::cmd_check(&[bad.to_string_lossy().into_owned()]),
        ExitCode::from(1)
    );
    assert_eq!(
        super::cmd_check(&[dir.join("nope.ecl").to_string_lossy().into_owned()]),
        ExitCode::from(2)
    );
}

/// 整局流程刀 spec §1:`collect_units` 目录收集 + `compile_units` 贯通。
#[test]
fn check_accepts_directory_of_units() {
    let dir = std::env::temp_dir().join(format!("gameflow_units_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a_main.ecl"), "sub main() { helper(); }\n").unwrap();
    std::fs::write(dir.join("b_lib.ecl"), "sub helper() { wait(1); }\n").unwrap();
    let units = super::collect_units(dir.to_str().unwrap()).unwrap();
    assert_eq!(units.len(), 2);
    assert_eq!(units[0].0, "a_main.ecl", "按名排序");
    assert!(stg_ecl_compiler::lang::compile_units(&units).is_ok());
    std::fs::remove_dir_all(&dir).ok();
}

// ── Task 6：`new_game_at` 端到端(整局流程刀 spec §2.2/§3)────────────────

/// 中段启动确定性跳过(Task 6 简报 Step 3):从头 vs `start=2` 中段启动同一份
/// 脚本——正常流 stage1 正常声明 bgm(1)、mark 垫片被跨过;中段启动 stage1 整段
/// 被跳过,垫片自动补 bgm(1) 后 stage2 同帧覆写为 2。外加两条同参中段启动世界
/// 120 帧逐帧校验和一致(确定性自证)。
#[test]
fn midstart_skips_earlier_stages_deterministically() {
    const SRC: &str = r#"
const M2: int = 2;
sub stage1() { bgm(1); wait(120); }
sub stage2() { bgm(2); wait(120); }
sub main() {
stage1();
mark(M2);
stage2();
loop { wait(60); }
}
"#;
    let image = stg_ecl_compiler::lang::compile(SRC, "mid.ecl").expect("编译");
    let t = &stg_core::tables::TABLES_V0;
    // `InputFrame` 无 `Default`(见 input.rs);`empty(0)` 与简报草稿的 `default()`
    // 等价(机械调整许可,同 `anchor_builtins_write_fields_and_reqs_via_script` 先例)。
    let input = stg_core::input::InputFrame::empty(0);
    // 从头:第 2 步后 bgm==1(stage1 正常声明,垫片被跨过)。
    let mut w0 = stg_core::step::World::new_game(7, 2, &image).expect("boot0");
    stg_core::step::step(&mut w0, t, &image, &input);
    stg_core::step::step(&mut w0, t, &image, &input);
    assert_eq!(w0.body.view().bgm_id(), 1);
    // 中段:start=2 → stage1 整跳,垫片补偿 bgm(1) 后 stage2 立即执行覆写为 2。
    let mut w2 =
        stg_core::step::World::new_game_at(7, 2, 2, Default::default(), &image).expect("boot2");
    stg_core::step::step(&mut w2, t, &image, &input);
    stg_core::step::step(&mut w2, t, &image, &input);
    assert_eq!(
        w2.body.view().bgm_id(),
        2,
        "跳入后 stage2 的声明生效(同帧覆写补偿值)"
    );
    // 确定性:同参中段启动两次,120 帧逐帧校验和一致。
    let mut wa = stg_core::step::World::new_game_at(7, 2, 2, Default::default(), &image).unwrap();
    let mut wb = stg_core::step::World::new_game_at(7, 2, 2, Default::default(), &image).unwrap();
    for f in 0..120 {
        stg_core::step::step(&mut wa, t, &image, &input);
        stg_core::step::step(&mut wb, t, &image, &input);
        assert_eq!(wa.checksum(), wb.checksum(), "frame {f}");
    }
}

/// Task 5 端到端①(简报 Step 1 原型,Task 6 解锁):正常流 start=0 跳过垫片、
/// 锚点由 stage1 正常写;中段启动 start=7 时 stage1 整个被跳过,垫片自动补
/// bgm/bg/bg_phase 三类——用 `new_game_at` 真跑 `World`/`step`,读回锚点字段
/// 判别(不是编译器侧字节码形状断言;那三条在 `stg-ecl-compiler::lang::mod::tests`
/// 保留不动,本测试是端到端补齐)。
#[test]
fn mark_injects_nearest_anchor_declarations() {
    const SRC: &str = r#"
sub stage1() { bgm(11); bg(21); bg_phase(1); wait(1); }
sub main() {
stage1();
mark(7);
loop { wait(60); }
}
"#;
    let image = stg_ecl_compiler::lang::compile(SRC, "comp.ecl").expect("编译");
    let t = &stg_core::tables::TABLES_V0;
    // `InputFrame` 无 `Default`(见 input.rs);`empty(0)` 与简报草稿的 `default()`
    // 等价(机械调整许可,同 `anchor_builtins_write_fields_and_reqs_via_script` 先例)。
    let input = stg_core::input::InputFrame::empty(0);
    let mut w =
        stg_core::step::World::new_game_at(7, 2, 7, Default::default(), &image).expect("boot");
    stg_core::step::step(&mut w, t, &image, &input);
    stg_core::step::step(&mut w, t, &image, &input);
    let v = w.body.view();
    assert_eq!((v.bgm_id(), v.bg_id(), v.bg_phase()), (11, 21, 1));
}

/// Task 5 端到端②:作者块顶层手写 `bgm(99)` 逐类抑制该类注入,`bg` 类仍照常
/// 补(stage1 里声明的 21 未被手写覆盖)。
#[test]
fn mark_manual_override_suppresses_injection_per_kind() {
    const SRC: &str = r#"
sub stage1() { bgm(11); bg(21); wait(1); }
sub main() {
stage1();
mark(3) { bgm(99); }
loop { wait(60); }
}
"#;
    let image = stg_ecl_compiler::lang::compile(SRC, "override.ecl").expect("编译");
    let t = &stg_core::tables::TABLES_V0;
    // `InputFrame` 无 `Default`(见 input.rs);`empty(0)` 与简报草稿的 `default()`
    // 等价(机械调整许可,同 `anchor_builtins_write_fields_and_reqs_via_script` 先例)。
    let input = stg_core::input::InputFrame::empty(0);
    let mut w =
        stg_core::step::World::new_game_at(7, 2, 3, Default::default(), &image).expect("boot");
    stg_core::step::step(&mut w, t, &image, &input);
    stg_core::step::step(&mut w, t, &image, &input);
    let v = w.body.view();
    assert_eq!(v.bgm_id(), 99, "作者块手写 bgm(99) 不被自动注入的 11 覆盖");
    assert_eq!(v.bg_id(), 21, "bg 类未手写,仍照常注入");
}

/// Task 5 端到端③:`bg_phase` 声明位置早于最近一条 `bg` 声明 → 视为旧背景
/// 段号,不注入;`bg` 本身仍正常补。
#[test]
fn stale_bg_phase_older_than_bg_not_injected() {
    const SRC: &str = r#"
sub main() {
bg_phase(5);
bg(30);
mark(1);
loop { wait(60); }
}
"#;
    let image = stg_ecl_compiler::lang::compile(SRC, "stale.ecl").expect("编译");
    let t = &stg_core::tables::TABLES_V0;
    // `InputFrame` 无 `Default`(见 input.rs);`empty(0)` 与简报草稿的 `default()`
    // 等价(机械调整许可,同 `anchor_builtins_write_fields_and_reqs_via_script` 先例)。
    let input = stg_core::input::InputFrame::empty(0);
    let mut w =
        stg_core::step::World::new_game_at(7, 2, 1, Default::default(), &image).expect("boot");
    stg_core::step::step(&mut w, t, &image, &input);
    stg_core::step::step(&mut w, t, &image, &input);
    let v = w.body.view();
    assert_eq!(v.bg_id(), 30, "bg 正常注入");
    assert_eq!(
        v.bg_phase(),
        0,
        "旧背景段号(早于 bg 的 bg_phase 声明)不注入"
    );
}
