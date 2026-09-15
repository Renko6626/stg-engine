//! 编码判别测试（spec §5 / §11）：手摆已知世界，逐字段断言字节；互异非零值防错位。
use stg_core::math::Fx;
use stg_core::step::World;
use stg_core::tables::TABLES_V0;
use stg_rl::encode::*;
use stg_rl::layout::off;

fn rd_i32(b: &[u8], o: usize) -> i32 {
    i32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn rd_u32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn rd_u16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes(b[o..o + 2].try_into().unwrap())
}

#[allow(clippy::too_many_arguments)]
fn bullet(w: &mut World, x: i32, y: i32, vx: i32, vy: i32, delay: u8, grazed: u8, sprite: u16) {
    w.body.create_bullet(stg_core::bullets::BulletInit {
        x: Fx::from_int(x),
        y: Fx::from_int(y),
        vx: Fx::from_int(vx),
        vy: Fx::from_int(vy),
        speed: Fx::ZERO,
        angle: stg_core::math::Angle::ZERO,
        ang_vel: 0,
        accel: Fx::ZERO,
        ax: Fx::ZERO,
        ay: Fx::ZERO,
        sprite,
        radius: Fx::from_int(3),
        delay,
        life: 0xFFFF,
        flags: 0,
        grazed_by: grazed,
        transform_head: 0xFFFF,
        xform_wait: 0,
        xform_next: 0,
        born_frame: 0,
    });
}

fn enemy(w: &mut World, x: i32, y: i32, hp: i32) -> stg_core::enemy::EnemyHandle {
    w.body.create_enemy(stg_core::enemy::EnemyInit {
        x: Fx::from_int(x),
        y: Fx::from_int(y),
        vx: Fx::ZERO,
        vy: Fx::ZERO,
        speed: Fx::ZERO,
        angle: stg_core::math::Angle::ZERO,
        vel_from_0: 0,
        vel_from_1: 0,
        vel_to_0: 0,
        vel_to_1: 0,
        vel_t: 0,
        vel_dur: 0,
        vel_easing: 0,
        vel_active: 0,
        vel_space: 0,
        vel_touched: 0,
        mv_from_x: Fx::ZERO,
        mv_from_y: Fx::ZERO,
        mv_to_x: Fx::ZERO,
        mv_to_y: Fx::ZERO,
        mv_t: 0,
        mv_dur: 0,
        mv_easing: 0,
        mv_active: 0,
        hp,
        hp_max: hp,
        radius: Fx::from_int(12),
        hurtbox: Fx::from_int(16),
        invuln: 0,
        hit_flash: 0,
        flags: 0,
        sprite: 0,
        anm_state: 0,
        anm_state_frame: w.body.frame(),
        main_task: 0,
        death_script: 0,
        drop_count: [0; stg_core::items::ITEM_TYPE_COUNT],
        score: 100,
    })
}

#[test]
fn player_row_fields() {
    let w = World::new(1);
    let mut row = [0xAAu8; 36];
    write_player(&w, &TABLES_V0, &mut row);
    let p = w.view().players()[0];
    let cfg = &TABLES_V0.characters[p.character_id as usize];
    assert_eq!(rd_i32(&row, off::player::X), p.x.raw());
    assert_eq!(rd_i32(&row, off::player::Y), p.y.raw());
    assert_eq!(rd_i32(&row, off::player::HIT_R), p.hit_radius.raw());
    assert_eq!(rd_i32(&row, off::player::SPEED), cfg.high_speed.raw());
    assert_eq!(rd_i32(&row, off::player::SPEED_F), cfg.low_speed.raw());
    assert_eq!(row[off::player::STATE], p.life_state);
    assert_eq!(row[off::player::LIVES], p.lives);
    assert_eq!(row[off::player::BOMBS], p.bombs);
    assert_eq!(rd_u16(&row, off::player::POWER), p.power);
    assert!(row.iter().all(|&b| b != 0xAA), "每字节都被写过");
}

#[test]
fn bullets_rows_flags_state_type_and_derived() {
    let mut w = World::new(1);
    bullet(&mut w, 10, 100, 3, 4, 0, 1, 77); // 可碰撞、已擦
    bullet(&mut w, -20, 50, 0, -2, 5, 0, 88); // 延迟中、未擦
    let mut rows = vec![0u8; 8 * 30];
    let mut scratch = Vec::new();
    let st = write_bullets(&w, 8, &mut rows, &mut scratch);
    assert_eq!((st.count, st.total, st.dropped), (2, 2, 0));
    let r0 = &rows[0..30];
    assert_eq!(rd_i32(r0, off::bullet::X), Fx::from_int(10).raw());
    assert_eq!(rd_i32(r0, off::bullet::VY), Fx::from_int(4).raw());
    assert_eq!(
        rd_i32(r0, off::bullet::SPEED),
        Fx::from_int(5).raw(),
        "isqrt(3²+4²)"
    );
    assert_eq!(
        rd_u16(r0, off::bullet::ANGLE),
        stg_core::math::atan2(Fx::from_int(4), Fx::from_int(3)).raw()
    );
    assert_eq!(r0[off::bullet::FLAGS], 0b111);
    assert_eq!(r0[off::bullet::STATE], 1);
    assert_eq!(rd_u16(r0, off::bullet::TYPE), 77);
    let r1 = &rows[30..60];
    assert_eq!(r1[off::bullet::FLAGS], 0b010, "delay>0 不可碰撞、未擦");
    assert_eq!(r1[off::bullet::STATE], 0);
    assert_eq!(rd_u16(r1, off::bullet::TYPE), 88);
}

#[test]
fn bullets_overflow_keeps_nearest_in_pool_order() {
    let mut w = World::new(1);
    // 自机在 (0,384)。池索引 0..10 距离依次：远近交错
    let ys = [0, 380, 10, 370, 20, 360, 30, 350, 40, 340];
    for (i, y) in ys.iter().enumerate() {
        bullet(&mut w, 0, *y, 0, 0, 0, 0, i as u16);
    }
    let mut rows = vec![0u8; 4 * 30];
    let st = write_bullets(&w, 4, &mut rows, &mut Vec::new());
    assert_eq!((st.count, st.total, st.dropped), (4, 10, 6));
    let types: Vec<u16> = (0..4)
        .map(|i| rd_u16(&rows[i * 30..], off::bullet::TYPE))
        .collect();
    assert_eq!(
        types,
        vec![1, 3, 5, 7],
        "最近 4 颗（y=380/370/360/350），按池索引序输出"
    );
}

#[test]
fn bullets_overflow_tie_breaks_by_pool_index() {
    let mut w = World::new(1);
    for i in 0..3 {
        bullet(&mut w, 0, 300, 0, 0, 0, 0, i); // 三颗等距
    }
    let mut rows = vec![0u8; 2 * 30];
    write_bullets(&w, 2, &mut rows, &mut Vec::new());
    assert_eq!(
        (
            rd_u16(&rows, off::bullet::TYPE),
            rd_u16(&rows[30..], off::bullet::TYPE)
        ),
        (0, 1)
    );
}

#[test]
fn enemies_rows_boss_collidable_id() {
    let mut w = World::new(1);
    let a = enemy(&mut w, 30, 60, 500);
    let b = enemy(&mut w, -30, 90, 800);
    w.body.boss_set(
        0,
        stg_core::boss::BossUiSlot {
            enemy: a,
            hp_ratio: Fx::ONE,
            spell_id: 0,
            timer_frames: 0,
            phase_left: 0,
            active: 1,
        },
    );
    // b 标 NO_BODY：`WorldBody::set_enemy_flags` 是公开脚本写口（只许 NO_BODY|KILLALL_EXEMPT）。
    w.body
        .set_enemy_flags(b, stg_core::enemy::ENEMY_NO_BODY, true);
    let mut rows = vec![0u8; 256 * 38];
    let n = write_enemies(&w, &mut rows);
    assert_eq!(n, 2);
    let (r0, r1) = (&rows[0..38], &rows[38..76]);
    assert_eq!(rd_i32(r0, off::enemy::HIT_W), Fx::from_int(12).raw());
    assert_eq!(rd_i32(r0, off::enemy::HURT_W), Fx::from_int(16).raw());
    assert_eq!(rd_i32(r0, off::enemy::HP), 500);
    assert_eq!(rd_u16(r0, off::enemy::FLAGS), 0x11, "boss + 可碰撞");
    assert_eq!(
        rd_u32(r0, off::enemy::ID),
        stg_core::enemy::pack_handle(a) as u32
    );
    assert_eq!(rd_u16(r1, off::enemy::FLAGS) & 0x01, 0, "非 boss");
    assert_eq!(
        rd_u16(r1, off::enemy::FLAGS) & 0x10,
        0,
        "NO_BODY/DYING 不可碰撞"
    );
    assert_eq!(
        rd_u32(r1, off::enemy::ID),
        stg_core::enemy::pack_handle(b) as u32
    );
}

#[test]
fn items_rows_kind_and_magnet() {
    use stg_core::items::*;
    let mut w = World::new(1);
    for t in [ITEM_POWER, ITEM_POINT, ITEM_LIFE_PIECE, ITEM_BOMB_PIECE] {
        w.body
            .drop_item(Fx::from_int(0), Fx::from_int(100), t, &TABLES_V0);
    }
    w.body.attract_all_items(0); // 前 4 个上锁
    w.body
        .drop_item(Fx::from_int(0), Fx::from_int(100), ITEM_STAR, &TABLES_V0); // 未锁
    let mut rows = vec![0u8; 1024 * 18];
    assert_eq!(write_items(&w, &mut rows), 5);
    let kinds: Vec<u8> = (0..5).map(|i| rows[i * 18 + off::item::KIND]).collect();
    assert_eq!(kinds, vec![1, 2, 8, 9, 7]);
    let flags: Vec<u8> = (0..5).map(|i| rows[i * 18 + off::item::FLAGS]).collect();
    assert_eq!(flags, vec![1, 1, 1, 1, 0]);
}

#[test]
fn phase_bits_in_game_and_controllable() {
    let w = World::new(1);
    assert_eq!(phase_bits(&w), PHASE_IN_GAME | PHASE_PLAYER_CONTROLLABLE);
}
