use stg_core::{
    ecl::image::EclImage,
    input::InputFrame,
    player::Loadout,
    step::{World, step},
    tables::TABLES_V0,
};

fn tick(w: &mut World, image: &EclImage, frames: usize) {
    for _ in 0..frames {
        let input = InputFrame::empty(w.frame());
        step(w, &TABLES_V0, image, &input);
    }
}

#[test]
fn enemy_tasks_share_variables_and_snapshots_continue_them() {
    let src = r#"
    async sub newborn(out: int) {
        set_global(out, self_int(0) == 0 && self_fx(ENEMY_FX_SLOTS - 1) == 0fx);
        wait(100);
    }
    async sub child(value: int, out: int) {
        set_global(out + 1, self_int(0) == value);
        set_self_int(0, value + 1);
        wait(4);
        set_global(out + 3, self_fx(ENEMY_FX_SLOTS - 1) == 9.25fx);
        set_self_fx(ENEMY_FX_SLOTS - 1, 12.5fx);
    }
    sub sync_read(value: int, out: int) {
        set_global(out, self_int(0) == value);
    }
    async sub enemy(value: int, out: int) {
        set_global(out + 5, self_int(0) == 0 && self_fx(ENEMY_FX_SLOTS - 1) == 0fx);
        set_self_int(0, value);
        set_self_int(ENEMY_INT_SLOTS - 1, 37deg as int);
        set_self_fx(ENEMY_FX_SLOTS - 1, 3.5fx);
        sync_read(value, out);
        spawn child(value, out);
        _ = spawn_enemy($self_x, 30fx, 100, 0, 0, 0, newborn(out + 6));
        wait(3);
        set_global(out + 2, self_int(0) == value + 1);
        set_self_fx(ENEMY_FX_SLOTS - 1, 9.25fx);
        wait(5);
        set_global(out + 4, self_fx(ENEMY_FX_SLOTS - 1) == 12.5fx
            && (self_int(ENEMY_INT_SLOTS - 1) as angle) == 37deg);
        wait(100);
    }
    sub main() {
        _ = spawn_enemy(-60fx, 60fx, 100, 0, 0, 0, enemy(120, 16));
        _ = spawn_enemy(60fx, 60fx, 100, 0, 0, 0, enemy(240, 23));
        wait(100);
    }"#;
    let image = stg_ecl_compiler::lang::compile(src, "enemy-vars.ecl").unwrap();
    let mut w = World::new_game_at(1, 0, 0, Loadout::default(), &image).unwrap();
    tick(&mut w, &image, 4);
    let bytes = w.save_bytes(&image);
    let mut restored = World::load_bytes(&bytes, &TABLES_V0, &image).unwrap();
    let mut copied = World::new(1);
    w.copy_into(&mut copied);
    assert_eq!(w.checksum(), restored.checksum());
    assert_eq!(w.checksum(), copied.checksum());
    tick(&mut w, &image, 12);
    tick(&mut restored, &image, 12);
    tick(&mut copied, &image, 12);
    assert_eq!(&w.view().globals()[16..30], &[1; 14]);
    assert_eq!(w.view().diag().task_faults, 0);
    assert_eq!(w.checksum(), restored.checksum());
    assert_eq!(w.checksum(), copied.checksum());
}

#[test]
fn invalid_slots_do_not_alias_valid_variables() {
    let src = r#"
    async sub enemy() {
        set_self_int(0, 73);
        set_self_fx(0, -2.25fx);
        set_self_int(-1, 99);
        set_self_int(65536, 99);
        set_self_fx(-1, 99fx);
        set_self_fx(65536, 99fx);
        set_global(16, self_int(0) == 73 && self_fx(0) == -2.25fx);
        set_global(17, self_int(ENEMY_INT_SLOTS) == 0);
        set_global(18, self_fx(ENEMY_FX_SLOTS) == 0fx);
        wait(100);
    }
    sub main() {
        _ = spawn_enemy(0fx, 60fx, 100, 0, 0, 0, enemy);
        wait(100);
    }"#;
    let image = stg_ecl_compiler::lang::compile(src, "enemy-vars-slots.ecl").unwrap();
    let mut w = World::new_game_at(1, 0, 0, Loadout::default(), &image).unwrap();
    tick(&mut w, &image, 4);
    assert_eq!(&w.view().globals()[16..19], &[1, 1, 1]);
    assert_eq!(w.view().diag().contract_viol, 6);
    assert_eq!(w.view().diag().task_faults, 0);
    assert_eq!(w.view().last_status(), stg_core::world::STATUS_BAD_ARGS);
}

#[test]
fn stage_tasks_cannot_access_enemy_variables() {
    for call in [
        "_ = self_int(0);",
        "_ = self_fx(0);",
        "set_self_int(0, 7);",
        "set_self_fx(0, 7fx);",
    ] {
        let src = format!("sub main() {{ {call} set_global(16, 1); wait(100); }}");
        let image = stg_ecl_compiler::lang::compile(&src, "stage-owner.ecl").unwrap();
        let mut w = World::new_game_at(1, 0, 0, Loadout::default(), &image).unwrap();
        tick(&mut w, &image, 3);
        assert_eq!(w.view().diag().task_faults, 1, "{call}");
        assert_eq!(w.view().globals()[16], 0, "{call}");
    }
}

#[test]
fn bullet_tasks_cannot_access_the_firing_enemys_variables() {
    let src = r#"
    async sub bullet() {
        _ = self_int(0);
        set_global(16, 1);
    }
    async sub enemy() {
        set_self_int(0, 7);
        _ = fire(48, 1, $self_x, $self_y, 0fx, 0deg, none, bullet);
        wait(100);
    }
    sub main() {
        _ = spawn_enemy(0fx, 60fx, 100, 0, 0, 0, enemy);
        wait(100);
    }"#;
    let image = stg_ecl_compiler::lang::compile(src, "bullet-owner.ecl").unwrap();
    let mut w = World::new_game_at(1, 0, 0, Loadout::default(), &image).unwrap();
    tick(&mut w, &image, 5);
    assert_eq!(w.view().diag().task_faults, 1);
    assert_eq!(w.view().globals()[16], 0);
}
