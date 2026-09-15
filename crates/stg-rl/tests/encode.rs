//! 编码判别测试（spec §5 / §11）：手摆已知世界，逐字段断言字节；互异非零值防错位。
//!
//! 自机字段（score/graze/碎片/输入位）在跨 crate 下是 `pub(crate)` 不可直写，故走**公开途径**：
//! `drop_item` + `attract_all_items` 真拾取、真 `step` 译码输入位、真擦弹——造出的值互异非零，
//! 字段交换（LFRAG↔BFRAG、SCORE↔GRAZE、RADIUS↔VX…）会红。
use stg_core::ecl::image::EclImage;
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

/// 编译一段测试用 ECL（`new_game_at` 需要非空 root）。
fn ecl(src: &str) -> EclImage {
    stg_ecl_compiler::lang::compile(src, "test.ecl").expect("测试脚本必须编译通过")
}

/// 一帧 step（空镜像：世界内无 ECL 任务）。
fn run_step(w: &mut World, buttons: u32) {
    run_step_with(w, &EclImage::empty(), buttons);
}

/// 一帧 step（显式镜像；世界若由 `new_game_at` 开机必须传同一镜像）。
fn run_step_with(w: &mut World, image: &EclImage, buttons: u32) {
    let mut input = stg_core::input::InputFrame::empty(0);
    input.actions[0].buttons = buttons;
    stg_core::step::step(w, &TABLES_V0, image, &input);
}

#[allow(clippy::too_many_arguments)]
fn bullet(
    w: &mut World,
    x: i32,
    y: i32,
    vx: i32,
    vy: i32,
    radius: i32,
    delay: u8,
    grazed: u8,
    sprite: u16,
) {
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
        radius: Fx::from_int(radius),
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

fn enemy(w: &mut World, x: i32, y: i32, hp: i32, hp_max: i32) -> stg_core::enemy::EnemyHandle {
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
        hp_max,
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

/// player 行：14 个字段偏移全覆盖，值互异非零（X=0 是自机中轴，单独标注）。
#[test]
fn player_row_fields() {
    use stg_core::input::BTN_SLOW;
    use stg_core::items::{ITEM_BOMB_PIECE, ITEM_LIFE_PIECE, ITEM_POINT};

    let mut w = World::new(1);
    // 公开途径造互异非零：2 片残机碎片 / 3 片雷碎片 / 1 颗分数道具，落在自机上并磁吸锁定。
    for _ in 0..2 {
        w.body.drop_item(
            Fx::from_int(0),
            Fx::from_int(384),
            ITEM_LIFE_PIECE,
            &TABLES_V0,
        );
    }
    for _ in 0..3 {
        w.body.drop_item(
            Fx::from_int(0),
            Fx::from_int(384),
            ITEM_BOMB_PIECE,
            &TABLES_V0,
        );
    }
    w.body
        .drop_item(Fx::from_int(0), Fx::from_int(384), ITEM_POINT, &TABLES_V0);
    w.body.attract_all_items(0);
    // 只擦不中的一颗弹：距自机 10px（擦圈 19、中弹圈 5.5），只贡献 graze。
    bullet(&mut w, 10, 384, 0, 0, 3, 0, 0, 0);
    run_step(&mut w, BTN_SLOW);

    let p = w.view().players()[0];
    // 前置：四个量必须真的被造出来（否则后面的行断言退化成"比 0"）。
    assert_eq!(p.life_pieces, 2, "前置：收集 2 片残机碎片");
    assert_eq!(p.bomb_pieces, 3, "前置：收集 3 片雷碎片");
    assert!(p.score > 0, "前置：POINT 道具入分");
    assert_eq!(p.graze, 1, "前置：擦弹一次");
    assert_eq!(p.input & BTN_SLOW, BTN_SLOW, "前置：低速输入已译码");

    let mut row = [0xAAu8; 36];
    write_player(&w, &TABLES_V0, &mut row);
    let cfg = &TABLES_V0.characters[p.character_id as usize];
    assert_eq!(rd_i32(&row, off::player::X), p.x.raw());
    assert_eq!(rd_i32(&row, off::player::Y), p.y.raw());
    assert_eq!(rd_i32(&row, off::player::HIT_R), p.hit_radius.raw());
    assert_eq!(rd_i32(&row, off::player::SPEED), cfg.high_speed.raw());
    assert_eq!(rd_i32(&row, off::player::SPEED_F), cfg.low_speed.raw());
    assert_eq!(row[off::player::FOCUS], 1, "BTN_SLOW 电平");
    assert_eq!(row[off::player::STATE], p.life_state);
    assert_eq!(row[off::player::LIVES], p.lives);
    assert_eq!(row[off::player::BOMBS], p.bombs);
    assert_eq!(row[off::player::LFRAG], p.life_pieces);
    assert_eq!(row[off::player::BFRAG], p.bomb_pieces);
    assert_eq!(row[off::player::LFRAG], 2);
    assert_eq!(row[off::player::BFRAG], 3);
    assert_eq!(rd_u16(&row, off::player::POWER), p.power);
    assert_eq!(u64::from(rd_u32(&row, off::player::SCORE)), p.score);
    assert_eq!(rd_u32(&row, off::player::GRAZE), p.graze);
}

#[test]
fn bullets_rows_flags_state_type_and_derived() {
    let mut w = World::new(1);
    bullet(&mut w, 10, 100, 3, 4, 7, 0, 1, 77); // 可碰撞、已擦；x/y/vx/vy/radius 互异非零
    bullet(&mut w, -20, 50, 0, -2, 9, 5, 0, 88); // 延迟中、未擦
    let mut rows = vec![0u8; 8 * 30];
    let mut scratch = BulletScratch::new();
    let st = write_bullets(&w, 8, &mut rows, &mut scratch);
    assert_eq!((st.count, st.total, st.dropped), (2, 2, 0));
    let r0 = &rows[0..30];
    assert_eq!(rd_i32(r0, off::bullet::X), Fx::from_int(10).raw());
    assert_eq!(rd_i32(r0, off::bullet::Y), Fx::from_int(100).raw());
    assert_eq!(rd_i32(r0, off::bullet::VX), Fx::from_int(3).raw());
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
    assert_eq!(rd_i32(r0, off::bullet::RADIUS), Fx::from_int(7).raw());
    assert_eq!(r0[off::bullet::FLAGS], 0b111);
    assert_eq!(r0[off::bullet::STATE], 1);
    assert_eq!(rd_u16(r0, off::bullet::TYPE), 77);
    let r1 = &rows[30..60];
    assert_eq!(rd_i32(r1, off::bullet::X), Fx::from_int(-20).raw());
    assert_eq!(rd_i32(r1, off::bullet::Y), Fx::from_int(50).raw());
    assert_eq!(rd_i32(r1, off::bullet::VX), 0);
    assert_eq!(rd_i32(r1, off::bullet::VY), Fx::from_int(-2).raw());
    assert_eq!(rd_i32(r1, off::bullet::RADIUS), Fx::from_int(9).raw());
    assert_eq!(r1[off::bullet::FLAGS], 0b010, "delay>0 不可碰撞、未擦");
    assert_eq!(r1[off::bullet::STATE], 0);
    assert_eq!(rd_u16(r1, off::bullet::TYPE), 88);
}

/// 派生量覆盖轴向 `(1,0)` / `(0,1)`、零向量 `(0,0)`、任意 `(-3,4)`：
/// `speed`/`angle` 必须与 core `isqrt(len_sq)` / `atan2(vy,vx)` 现算值逐位相等。
/// 判别力：helper 把池内 `speed`/`angle` 都设成 0，若实现偷读池内滞后极坐标立刻红。
#[test]
fn bullets_derived_speed_angle_over_axis_zero_and_negative_vectors() {
    let vecs: [(i32, i32); 4] = [(1, 0), (0, 1), (0, 0), (-3, 4)];
    let mut w = World::new(1);
    for (k, &(vx, vy)) in vecs.iter().enumerate() {
        bullet(&mut w, (k + 1) as i32, 10, vx, vy, 3, 0, 0, k as u16);
    }
    let mut rows = vec![0u8; 4 * 30];
    write_bullets(&w, 4, &mut rows, &mut BulletScratch::new());
    for (k, &(vx, vy)) in vecs.iter().enumerate() {
        let r = &rows[k * 30..(k + 1) * 30];
        let (vxf, vyf) = (Fx::from_int(vx), Fx::from_int(vy));
        let want_speed = stg_core::math::isqrt(stg_core::math::len_sq(vxf, vyf) as u64) as i32;
        assert_eq!(
            rd_i32(r, off::bullet::SPEED),
            want_speed,
            "v=({vx},{vy}) speed"
        );
        assert_eq!(
            rd_u16(r, off::bullet::ANGLE),
            stg_core::math::atan2(vyf, vxf).raw(),
            "v=({vx},{vy}) angle"
        );
    }
}

/// 派生量记忆绝不供陈旧值：同一份 `BulletScratch` 先喂世界 A、再喂世界 B（同池槽、不同速度），
/// B 的 speed/angle 必须等于现算。判别力：记忆若按槽身份（而非 `(vx,vy)` 值）命中，B 会读到 A 的结果立刻红；
/// 再喂回 A 验证回切也对。零向量 `(0,0)` 覆盖记忆初值格。
#[test]
fn bullet_scratch_memo_never_serves_stale_derived() {
    let sets: [[(i32, i32); 3]; 3] = [
        [(3, 4), (0, 0), (-5, 2)],
        [(-3, 4), (1, 0), (-5, -2)],
        [(3, 4), (0, 0), (-5, 2)],
    ];
    let mut scratch = BulletScratch::new();
    for vecs in sets {
        let mut w = World::new(1);
        for (k, &(vx, vy)) in vecs.iter().enumerate() {
            bullet(&mut w, k as i32, 10, vx, vy, 3, 0, 0, k as u16);
        }
        let mut rows = vec![0u8; 3 * 30];
        write_bullets(&w, 3, &mut rows, &mut scratch);
        for (k, &(vx, vy)) in vecs.iter().enumerate() {
            let r = &rows[k * 30..(k + 1) * 30];
            let (vxf, vyf) = (Fx::from_int(vx), Fx::from_int(vy));
            assert_eq!(
                rd_i32(r, off::bullet::SPEED),
                stg_core::math::isqrt(stg_core::math::len_sq(vxf, vyf) as u64) as i32,
                "v=({vx},{vy}) speed"
            );
            assert_eq!(
                rd_u16(r, off::bullet::ANGLE),
                stg_core::math::atan2(vyf, vxf).raw(),
                "v=({vx},{vy}) angle"
            );
        }
    }
}

#[test]
fn bullets_overflow_keeps_nearest_in_pool_order() {
    let mut w = World::new(1);
    // 自机在 (0,384)。池索引 0..10 距离依次：远近交错
    let ys = [0, 380, 10, 370, 20, 360, 30, 350, 40, 340];
    for (i, y) in ys.iter().enumerate() {
        bullet(&mut w, 0, *y, 0, 0, 3, 0, 0, i as u16);
    }
    let mut rows = vec![0u8; 4 * 30];
    let st = write_bullets(&w, 4, &mut rows, &mut BulletScratch::new());
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
        bullet(&mut w, 0, 300, 0, 0, 3, 0, 0, i); // 三颗等距
    }
    let mut rows = vec![0u8; 2 * 30];
    write_bullets(&w, 2, &mut rows, &mut BulletScratch::new());
    assert_eq!(
        (
            rd_u16(&rows, off::bullet::TYPE),
            rd_u16(&rows[30..], off::bullet::TYPE)
        ),
        (0, 1)
    );
}

/// `cap == 0` 是编码函数的契约外输入（plan 规定 `1..=8192`，HELLO 校验）——
/// 必须响亮 panic，而不是 `cap - 1` 下溢成巨索引后越界。
#[test]
#[should_panic(expected = "cap")]
fn write_bullets_rejects_zero_cap() {
    let mut w = World::new(1);
    bullet(&mut w, 0, 0, 1, 0, 3, 0, 0, 0);
    let mut rows = [0u8; 30];
    let _ = write_bullets(&w, 0, &mut rows, &mut BulletScratch::new());
}

#[test]
fn enemies_rows_boss_collidable_id() {
    let mut w = World::new(1);
    let a = enemy(&mut w, 30, 60, 500, 700);
    let b = enemy(&mut w, -30, 90, 800, 900);
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
    assert_eq!(rd_i32(r0, off::enemy::X), Fx::from_int(30).raw());
    assert_eq!(rd_i32(r0, off::enemy::Y), Fx::from_int(60).raw());
    assert_eq!(rd_i32(r0, off::enemy::HURT_W), Fx::from_int(16).raw());
    assert_eq!(rd_i32(r0, off::enemy::HURT_H), Fx::from_int(16).raw());
    assert_eq!(rd_i32(r0, off::enemy::HIT_W), Fx::from_int(12).raw());
    assert_eq!(rd_i32(r0, off::enemy::HIT_H), Fx::from_int(12).raw());
    assert_eq!(rd_i32(r0, off::enemy::HP), 500);
    assert_eq!(rd_i32(r0, off::enemy::HP_MAX), 700, "hp 与 hp_max 互异");
    assert_eq!(rd_u16(r0, off::enemy::FLAGS), 0x11, "boss + 可碰撞");
    assert_eq!(
        rd_u32(r0, off::enemy::ID),
        stg_core::enemy::pack_handle(a) as u32
    );
    assert_eq!(rd_i32(r1, off::enemy::X), Fx::from_int(-30).raw());
    assert_eq!(rd_i32(r1, off::enemy::Y), Fx::from_int(90).raw());
    assert_eq!(rd_i32(r1, off::enemy::HURT_H), Fx::from_int(16).raw());
    assert_eq!(rd_i32(r1, off::enemy::HIT_H), Fx::from_int(12).raw());
    assert_eq!(rd_i32(r1, off::enemy::HP), 800);
    assert_eq!(rd_i32(r1, off::enemy::HP_MAX), 900);
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
    // 落点互异 ⇒ X/Y 错位会红（逐行与池值比，见下）。
    let drops = [
        (ITEM_POWER, 11, 101),
        (ITEM_POINT, 12, 102),
        (ITEM_LIFE_PIECE, 13, 103),
        (ITEM_BOMB_PIECE, 14, 104),
    ];
    for &(t, x, y) in drops.iter() {
        w.body
            .drop_item(Fx::from_int(x), Fx::from_int(y), t, &TABLES_V0);
    }
    w.body.attract_all_items(0); // 前 4 个上锁
    w.body
        .drop_item(Fx::from_int(15), Fx::from_int(105), ITEM_STAR, &TABLES_V0); // 未锁
    let mut rows = vec![0u8; 1024 * 18];
    assert_eq!(write_items(&w, &mut rows), 5);
    let kinds: Vec<u8> = (0..5).map(|i| rows[i * 18 + off::item::KIND]).collect();
    assert_eq!(kinds, vec![1, 2, 8, 9, 7]);
    let flags: Vec<u8> = (0..5).map(|i| rows[i * 18 + off::item::FLAGS]).collect();
    assert_eq!(flags, vec![1, 1, 1, 1, 0]);
    // X/Y/VX/VY：逐行与池值相等（drop 后未 step ⇒ 位置 = 落点、速度为散布初速）。
    let it = w.view().items();
    for i in 0..5 {
        let r = &rows[i * 18..(i + 1) * 18];
        assert_eq!(rd_i32(r, off::item::X), it.x()[i].raw(), "item {i} x");
        assert_eq!(rd_i32(r, off::item::Y), it.y()[i].raw(), "item {i} y");
        assert_eq!(rd_i32(r, off::item::VX), it.vx()[i].raw(), "item {i} vx");
        assert_eq!(rd_i32(r, off::item::VY), it.vy()[i].raw(), "item {i} vy");
    }
    assert_eq!(rd_i32(&rows[0..], off::item::X), Fx::from_int(11).raw());
    assert_eq!(
        rd_i32(&rows[4 * 18..], off::item::Y),
        Fx::from_int(105).raw()
    );
}

#[test]
fn phase_bits_in_game_and_controllable() {
    let w = World::new(1);
    assert_eq!(phase_bits(&w), PHASE_IN_GAME | PHASE_PLAYER_CONTROLLABLE);
}

/// 活跃符卡 ⇒ 置 `PHASE_SPELL_ACTIVE`（全等断言，位值/条件写错都红）。
#[test]
fn phase_bits_spell_active() {
    let mut w = World::new(1);
    let boss = enemy(&mut w, 30, 60, 500, 700);
    assert!(w.body.spell_begin_internal(0, boss, 1, 300, 1000, 0, 100));
    assert_eq!(
        phase_bits(&w),
        PHASE_IN_GAME | PHASE_PLAYER_CONTROLLABLE | PHASE_SPELL_ACTIVE
    );
}

/// 机体 1（Classic）喂一帧 `BTN_BOMB` ⇒ `bomb_timer > 0` ⇒ 置 `PHASE_BOMB_ACTIVE`。
#[test]
fn phase_bits_bomb_active() {
    use stg_core::input::BTN_BOMB;
    use stg_core::player::Loadout;
    let image = ecl("sub main() { loop { wait(1); } }");
    let mut w = World::new_game_at(
        1,
        2,
        0,
        Loadout {
            character: 1,
            ..Loadout::default()
        },
        &image,
    )
    .expect("开机");
    run_step_with(&mut w, &image, BTN_BOMB);
    assert_ne!(phase_bits(&w) & PHASE_BOMB_ACTIVE, 0, "bomb 触发帧必须置位");
}

/// CONTROLLABLE 负例：ECL `time_stop_player` 写 `freeze_left[1]` ⇒ 冻结中不可控。
/// 判别力：若 `phase_bits` 误读 `freeze_left[0]`（或漏判），此世界会回 `CONTROLLABLE`，测试红。
#[test]
fn phase_bits_frozen_player_not_controllable() {
    use stg_core::player::Loadout;
    let image = ecl("sub main() { time_stop_player(90); loop { wait(1); } }");
    let mut w = World::new_game_at(
        1,
        2,
        0,
        Loadout {
            character: 1,
            ..Loadout::default()
        },
        &image,
    )
    .expect("开机");
    run_step_with(&mut w, &image, 0); // 出生帧 main 不跑
    run_step_with(&mut w, &image, 0); // main 首跑：写 freeze_left[1] = 90
    assert_ne!(phase_bits(&w) & PHASE_IN_GAME, 0);
    assert_eq!(
        phase_bits(&w) & PHASE_PLAYER_CONTROLLABLE,
        0,
        "冻结中不可控"
    );
}

/// phase 位的字面值钉死（proto SPEC §1 phase 位表 / c/world.h `AP_PHASE_*`）：
/// 其它 phase 测试的期望都复用同组常量，改常量数值它们不会红——这条防的就是这个。
#[test]
fn phase_bit_literals_match_proto() {
    assert_eq!(PHASE_IN_GAME, 0x01, "AP_PHASE_IN_GAME = 1<<0");
    assert_eq!(PHASE_BOMB_ACTIVE, 0x10, "AP_PHASE_BOMB_ACTIVE = 1<<4");
    assert_eq!(PHASE_SPELL_ACTIVE, 0x20, "AP_PHASE_SPELL_ACTIVE = 1<<5");
    assert_eq!(
        PHASE_PLAYER_CONTROLLABLE, 0x40,
        "AP_PHASE_PLAYER_CONTROLLABLE = 1<<6"
    );
}
