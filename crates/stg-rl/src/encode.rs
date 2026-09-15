//! World → proto v1 行字节（spec §5）。只读 `w.view()` 与公开读口；每行全部字节显式写满。

use crate::layout::{ENEMIES_CAP, ITEMS_CAP, off};
use stg_core::enemy::{ENEMY_DYING, ENEMY_NO_BODY, EnemyHandle, pack_handle};
use stg_core::items::{MAGNET_NONE, MAGNET_PICKED};
use stg_core::math::{atan2, isqrt, len_sq};
use stg_core::player::LIFE_ALIVE;
use stg_core::step::World;
use stg_core::tables::WorldTables;

pub const PHASE_IN_GAME: u32 = 1;
pub const PHASE_BOMB_ACTIVE: u32 = 1 << 4;
pub const PHASE_SPELL_ACTIVE: u32 = 1 << 5;
pub const PHASE_PLAYER_CONTROLLABLE: u32 = 1 << 6;
/// 引擎 `ITEM_*` 下标 → proto `AP_ITEM_*`（POWER, POINT, LIFE_PIECE, BOMB_PIECE, STAR）。
pub const ITEM_KIND: [u8; 5] = [1, 2, 8, 9, 7];

#[inline]
fn put_i32(b: &mut [u8], o: usize, v: i32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}
#[inline]
fn put_u32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}
#[inline]
fn put_u16(b: &mut [u8], o: usize, v: u16) {
    b[o..o + 2].copy_from_slice(&v.to_le_bytes());
}

pub fn phase_bits(w: &World) -> u32 {
    let v = w.view();
    let p = &v.players()[0];
    let mut bits = PHASE_IN_GAME;
    if p.bomb_timer > 0 {
        bits |= PHASE_BOMB_ACTIVE;
    }
    if v.spells().iter().any(|s| s.active != 0) {
        bits |= PHASE_SPELL_ACTIVE;
    }
    if p.life_state == LIFE_ALIVE && v.freeze_left()[1] == 0 {
        bits |= PHASE_PLAYER_CONTROLLABLE;
    }
    bits
}

pub fn write_player(w: &World, tables: &WorldTables, row: &mut [u8]) {
    let p = &w.view().players()[0];
    let cfg = &tables.characters[p.character_id as usize];
    row.fill(0);
    put_i32(row, off::player::X, p.x.raw());
    put_i32(row, off::player::Y, p.y.raw());
    put_i32(row, off::player::HIT_R, p.hit_radius.raw());
    put_i32(row, off::player::SPEED, cfg.high_speed.raw());
    put_i32(row, off::player::SPEED_F, cfg.low_speed.raw());
    row[off::player::FOCUS] = u8::from(p.input & stg_core::input::BTN_SLOW != 0);
    row[off::player::STATE] = p.life_state;
    row[off::player::LIVES] = p.lives;
    row[off::player::BOMBS] = p.bombs;
    row[off::player::LFRAG] = p.life_pieces;
    row[off::player::BFRAG] = p.bomb_pieces;
    put_u16(row, off::player::POWER, p.power);
    put_u32(row, off::player::SCORE, p.score.min(u32::MAX as u64) as u32);
    put_u32(row, off::player::GRAZE, p.graze);
}

pub struct BulletStats {
    pub count: usize,
    pub total: usize,
    pub dropped: usize,
}

pub fn write_bullets(
    w: &World,
    cap: usize,
    rows: &mut [u8],
    scratch: &mut Vec<(i64, u16)>,
) -> BulletStats {
    let v = w.view();
    let b = v.bullets();
    let p = &v.players()[0];
    let total = b.iter_alive().count();
    scratch.clear();
    if total <= cap {
        scratch.extend(b.iter_alive().map(|i| (0, i as u16)));
    } else {
        scratch.extend(
            b.iter_alive()
                .map(|i| (len_sq(b.x()[i] - p.x, b.y()[i] - p.y), i as u16)),
        );
        scratch.select_nth_unstable(cap - 1); // 全序键 (距离², 池索引) ⇒ 结果确定
        scratch.truncate(cap);
        scratch.sort_unstable_by_key(|&(_, i)| i);
    }
    for (k, &(_, i)) in scratch.iter().enumerate() {
        let i = i as usize;
        let r = &mut rows[k * 30..(k + 1) * 30];
        r.fill(0);
        let (vx, vy) = (b.vx()[i], b.vy()[i]);
        put_i32(r, off::bullet::X, b.x()[i].raw());
        put_i32(r, off::bullet::Y, b.y()[i].raw());
        put_i32(r, off::bullet::VX, vx.raw());
        put_i32(r, off::bullet::VY, vy.raw());
        put_i32(
            r,
            off::bullet::SPEED,
            isqrt(len_sq(vx, vy) as u64).min(i32::MAX as u32) as i32,
        );
        put_u16(r, off::bullet::ANGLE, atan2(vy, vx).raw());
        put_i32(r, off::bullet::RADIUS, b.radius()[i].raw());
        let delay = b.delay()[i];
        r[off::bullet::FLAGS] =
            u8::from(delay == 0) | 2 | (u8::from(b.grazed_by()[i] & 1 != 0) << 2);
        r[off::bullet::STATE] = u8::from(delay == 0);
        put_u16(r, off::bullet::TYPE, b.sprite()[i]);
    }
    let count = scratch.len();
    BulletStats {
        count,
        total,
        dropped: total - count,
    }
}

pub fn write_enemies(w: &World, rows: &mut [u8]) -> usize {
    let v = w.view();
    let e = v.enemies();
    let bosses: Vec<EnemyHandle> = v
        .boss_ui()
        .iter()
        .filter(|s| s.active != 0)
        .map(|s| s.enemy)
        .collect();
    let mut k = 0;
    for i in e.iter_alive().take(ENEMIES_CAP) {
        let r = &mut rows[k * 38..(k + 1) * 38];
        r.fill(0);
        let h = EnemyHandle {
            index: i as u16,
            generation: e.generation_of(i),
        };
        put_i32(r, off::enemy::X, e.x()[i].raw());
        put_i32(r, off::enemy::Y, e.y()[i].raw());
        put_i32(r, off::enemy::HURT_W, e.hurtbox()[i].raw());
        put_i32(r, off::enemy::HURT_H, e.hurtbox()[i].raw());
        put_i32(r, off::enemy::HIT_W, e.radius()[i].raw());
        put_i32(r, off::enemy::HIT_H, e.radius()[i].raw());
        put_i32(r, off::enemy::HP, e.hp()[i]);
        put_i32(r, off::enemy::HP_MAX, e.hp_max()[i]);
        let boss = bosses.contains(&h);
        let collidable = e.flags()[i] & (ENEMY_NO_BODY | ENEMY_DYING) == 0;
        put_u16(
            r,
            off::enemy::FLAGS,
            u16::from(boss) | (u16::from(collidable) << 4),
        );
        put_u32(r, off::enemy::ID, pack_handle(h) as u32);
        k += 1;
    }
    k
}

pub fn write_items(w: &World, rows: &mut [u8]) -> usize {
    let it = w.view().items();
    let mut k = 0;
    for i in it.iter_alive() {
        if k == ITEMS_CAP || it.magnet_to()[i] == MAGNET_PICKED {
            continue;
        }
        let r = &mut rows[k * 18..(k + 1) * 18];
        r.fill(0);
        put_i32(r, off::item::X, it.x()[i].raw());
        put_i32(r, off::item::Y, it.y()[i].raw());
        put_i32(r, off::item::VX, it.vx()[i].raw());
        put_i32(r, off::item::VY, it.vy()[i].raw());
        r[off::item::KIND] = ITEM_KIND
            .get(it.item_type()[i] as usize)
            .copied()
            .unwrap_or(0);
        r[off::item::FLAGS] = u8::from(it.magnet_to()[i] != MAGNET_NONE);
        k += 1;
    }
    k
}
