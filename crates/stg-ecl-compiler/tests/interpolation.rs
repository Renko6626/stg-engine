use stg_core::{
    input::InputFrame,
    player::Loadout,
    step::{World, step},
    tables::TABLES_V0,
};

#[test]
fn explicit_slots_advance_during_wait_and_survive_save() {
    let src = r#"sub main() {
        accum_start(0, 1fx, 2fx, 3);
        accel_start(0, 0fx, 2fx, 1fx, 3);
        hermite_start(0, 0fx, 0fx, 8fx, -8fx, 4);
        set_global(16, accum_value(0) == 1fx);
        wait(2);
        set_global(17, accum_value(0) == 5fx);
        set_global(18, accel_value(0) == 5fx);
        set_global(19, hermite_value(0) == 2fx);
        wait(3);
        set_global(20, accum_value(0) == 7fx);
        set_global(21, accel_value(0) == 9fx);
        set_global(22, hermite_value(0) == 0fx);
        wait(100);
    }"#;
    let image = stg_ecl_compiler::lang::compile(src, "slots.ecl").unwrap();
    let mut w = World::new_game_at(1, 0, 0, Loadout::default(), &image).unwrap();
    let tick = |w: &mut World| {
        let input = InputFrame::empty(w.frame());
        step(w, &TABLES_V0, &image, &input);
    };
    // Birth frame is skipped; the next frame runs through the first wait.
    tick(&mut w);
    tick(&mut w);
    assert_eq!(w.view().globals()[16], 1);
    tick(&mut w);
    let bytes = w.save_bytes(&image);
    let mut restored = World::load_bytes(&bytes, &TABLES_V0, &image).unwrap();
    let mut copied = World::new(1);
    w.copy_into(&mut copied);
    for _ in 0..6 {
        tick(&mut w);
        tick(&mut restored);
        tick(&mut copied);
    }
    assert_eq!(w.checksum(), restored.checksum());
    assert_eq!(w.checksum(), copied.checksum());
    for (slot, expected) in [(17, 1), (18, 1), (19, 1), (20, 1), (21, 1), (22, 1)] {
        assert_eq!(w.view().globals()[slot], expected, "slot {slot}");
    }
}

#[test]
fn slots_are_task_private_and_sync_calls_share_them() {
    let src = r#"
    sub read_shared() { set_global(16, accum_value(0) == 10fx); }
    async sub child() {
        set_global(17, accum_value(0) == 0fx);
        accum_start(0, 100fx, 1fx, 3);
        wait(1);
        set_global(18, accum_value(0) == 101fx);
    }
    sub main() {
        accum_start(0, 10fx, 2fx, 3);
        read_shared();
        spawn child();
        wait(3);
        set_global(19, accum_value(0) == 16fx);
        wait(10);
    }"#;
    let image = stg_ecl_compiler::lang::compile(src, "tasks.ecl").unwrap();
    let mut w = World::new_game_at(1, 0, 0, Loadout::default(), &image).unwrap();
    for _ in 0..6 {
        let input = InputFrame::empty(w.frame());
        step(&mut w, &TABLES_V0, &image, &input);
    }
    assert_eq!(&w.view().globals()[16..20], &[1, 1, 1, 1]);
}

#[test]
fn zero_duration_restart_and_invalid_arguments() {
    let src = r#"sub main() {
        accum_start(0, 3fx, 9fx, 0);
        accel_start(0, 4fx, 9fx, 9fx, 0);
        hermite_start(0, 1fx, 5fx, 9fx, 9fx, 0);
        set_global(16, accum_value(0) == 3fx);
        set_global(17, accel_value(0) == 4fx);
        set_global(18, hermite_value(0) == 5fx);
        accum_start(0, 7fx, 1fx, 2);
        accum_start(0, 99fx, 9fx, -1);
        accel_start(0, 99fx, 9fx, 9fx, 65536);
        hermite_start(INTERP_SLOTS_PER_KIND, 0fx, 0fx, 0fx, 0fx, 2);
        set_global(19, accum_value(-1) == 0fx);
        set_global(20, accum_value(0) == 7fx);
        set_global(21, accum_value(0) == 7fx);
        wait(3);
        set_global(22, accum_value(0) == 9fx);
        set_global(23, accel_value(0) == 4fx);
        set_global(24, hermite_value(0) == 5fx);
        hermite_start(0, 2fx, 10fx, 0fx, 0fx, 1);
        wait(1);
        set_global(25, hermite_value(0) == 10fx);
        wait(10);
    }"#;
    let image = stg_ecl_compiler::lang::compile(src, "edges.ecl").unwrap();
    let mut w = World::new_game_at(1, 0, 0, Loadout::default(), &image).unwrap();
    for _ in 0..7 {
        let input = InputFrame::empty(w.frame());
        step(&mut w, &TABLES_V0, &image, &input);
    }
    assert_eq!(&w.view().globals()[16..26], &[1; 10]);
    assert_eq!(w.view().diag().contract_viol, 4);
}
