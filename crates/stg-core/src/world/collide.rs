//! 相位 6 · 碰撞收集（D8 矩阵）。
//!
//! **只收集不改状态硬规则**：本相位纯读，只经 `push_hit` 追加 `hits`；改状态一律在 settle（相位 7）。
//! 半径映射见 D8：行1/2 弹×自机(hit/graze)、行3 敌体×自机(体碰 radius)、行4 自机弹×敌人(受击 hurtbox)、
//! 行5 道具×自机(拾取半径+graze_radius)、行6 作用区×敌弹、行7 作用区×敌人(受击 hurtbox)。

use super::WorldBody;
use crate::events::{
    ROW_BODY_PLAYER_HIT, ROW_BULLET_PLAYER_GRAZE, ROW_BULLET_PLAYER_HIT, ROW_FIELD_BULLET,
    ROW_FIELD_ENEMY, ROW_ITEM_PLAYER, ROW_SHOT_ENEMY,
};
use crate::field::{FIELD_CLEAR_BULLETS, FIELD_DAMAGE};
use crate::math::geom::len_sq;
use crate::tables::WorldTables;

impl WorldBody {
    pub(crate) fn collide(&mut self, tables: &WorldTables) {
        self.phase_enter(super::PH_COLLIDE);
        self.collide_bullets_player(); // 行 1/2：敌弹 × 自机
        self.collide_body_player(); // 行 3：敌体 × 自机
        self.collide_shot_enemy(); // 行 4：自机弹 × 敌人
        self.collide_item_player(tables); // 行 5：道具 × 自机拾取圈
        self.collide_field_bullet(); // 行 6：作用区 × 敌弹（消弹）
        self.collide_field_enemy(); // 行 7：作用区 × 敌人（伤敌）
    }

    /// 行 1（hit）+ 行 2（graze）：敌弹 × 自机。一次 len_sq 复用两半径。
    fn collide_bullets_player(&mut self) {
        for p in 0..crate::MAX_PLAYERS {
            if self.players[p].life_state != crate::player::LIFE_ALIVE
                || self.players[p].invuln != 0
            {
                continue; // 门禁：只 Alive 且非无敌参与
            }
            let (px, py) = (self.players[p].x, self.players[p].y);
            let hit_r = self.players[p].hit_radius;
            let graze_r = self.players[p].graze_radius;
            let nw = self.bullets.alive.len();
            for w in 0..nw {
                let mut bits = self.bullets.alive[w];
                while bits != 0 {
                    let b = w * 64 + bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    if self.bullets.delay[b] > 0 {
                        continue; // delay 弹不参与
                    }
                    let dx = self.bullets.x[b] - px;
                    let dy = self.bullets.y[b] - py;
                    let d2 = len_sq(dx, dy);
                    let br = self.bullets.radius[b];
                    let graze_sum = (br + graze_r).raw() as i64;
                    if d2 <= graze_sum * graze_sum {
                        self.push_hit(ROW_BULLET_PLAYER_GRAZE, b as u16, p as u16);
                        let hit_sum = (br + hit_r).raw() as i64;
                        if d2 <= hit_sum * hit_sum {
                            self.push_hit(ROW_BULLET_PLAYER_HIT, b as u16, p as u16);
                        }
                    }
                }
            }
        }
    }
    /// 行 3：敌体（enemy.radius）× 自机 hit_radius。
    ///
    /// **不查 `ENEMY_DYING`**——人类裁定 D-9（spec `2026-07-30-enemy-death-effect-design.md`
    /// §3/§6）：dying 敌当帧仍参与体碰、仍撞得死自机。别顺手加 dying 门禁；
    /// `tests::collide_dying_enemy_still_body_hits_player` 就是为此钉的。
    fn collide_body_player(&mut self) {
        for p in 0..crate::MAX_PLAYERS {
            if self.players[p].life_state != crate::player::LIFE_ALIVE
                || self.players[p].invuln != 0
            {
                continue;
            }
            let (px, py) = (self.players[p].x, self.players[p].y);
            let hit_r = self.players[p].hit_radius;
            let nw = self.enemies.alive.len();
            for w in 0..nw {
                let mut bits = self.enemies.alive[w];
                while bits != 0 {
                    let e = w * 64 + bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    let dx = self.enemies.x[e] - px;
                    let dy = self.enemies.y[e] - py;
                    let d2 = len_sq(dx, dy);
                    let sum = (self.enemies.radius[e] + hit_r).raw() as i64;
                    if d2 <= sum * sum {
                        self.push_hit(ROW_BODY_PLAYER_HIT, e as u16, p as u16);
                    }
                }
            }
        }
    }

    /// 行 4：自机弹（shot.radius）× 敌人 hurtbox（受击圈）。
    /// 嵌套固定：shot 外层、enemy 内层（升序）→ settle 扣血序确定。无敌帧过滤留给 settle。
    fn collide_shot_enemy(&mut self) {
        let nwe = self.enemies.alive.len();
        let nws = self.shots.alive.len();
        for sw in 0..nws {
            let mut sbits = self.shots.alive[sw];
            while sbits != 0 {
                let s = sw * 64 + sbits.trailing_zeros() as usize;
                sbits &= sbits - 1;
                let (sx, sy) = (self.shots.x[s], self.shots.y[s]);
                let sr = self.shots.radius[s];
                for ew in 0..nwe {
                    let mut ebits = self.enemies.alive[ew];
                    while ebits != 0 {
                        let e = ew * 64 + ebits.trailing_zeros() as usize;
                        ebits &= ebits - 1;
                        let dx = self.enemies.x[e] - sx;
                        let dy = self.enemies.y[e] - sy;
                        let d2 = len_sq(dx, dy);
                        let sum = (sr + self.enemies.hurtbox[e]).raw() as i64;
                        if d2 <= sum * sum {
                            self.push_hit(ROW_SHOT_ENEMY, s as u16, e as u16);
                        }
                    }
                }
            }
        }
    }
    /// 行 5：道具 × 自机拾取圈（graze_radius 兼拾取圈，D7；主动半径查配置表）。
    fn collide_item_player(&mut self, tables: &WorldTables) {
        for p in 0..crate::MAX_PLAYERS {
            if self.players[p].life_state != crate::player::LIFE_ALIVE {
                continue;
            }
            let nw = self.items.alive.len();
            for w in 0..nw {
                let mut bits = self.items.alive[w];
                while bits != 0 {
                    let i = w * 64 + bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    let pr = tables.item_cfg[self.items.item_type[i] as usize].pickup_radius;
                    let r = pr + self.players[p].graze_radius;
                    let d2 = len_sq(
                        self.items.x[i] - self.players[p].x,
                        self.items.y[i] - self.players[p].y,
                    );
                    if d2 <= (r.raw() as i64) * (r.raw() as i64) {
                        self.push_hit(ROW_ITEM_PLAYER, i as u16, p as u16);
                    }
                }
            }
        }
    }

    /// 行 6：作用区（field.radius）× 敌弹（bullet.radius）→ 消弹。
    /// 能力位在收集前 gate（未开 CLEAR 的 field 整行跳过，省 O(N×M)）。
    fn collide_field_bullet(&mut self) {
        let nwf = self.fields.alive.len();
        let nwb = self.bullets.alive.len();
        for fw in 0..nwf {
            let mut fbits = self.fields.alive[fw];
            while fbits != 0 {
                let f = fw * 64 + fbits.trailing_zeros() as usize;
                fbits &= fbits - 1;
                if self.fields.flags[f] & FIELD_CLEAR_BULLETS == 0 {
                    continue;
                }
                let (fx, fy) = (self.fields.x[f], self.fields.y[f]);
                let fr = self.fields.radius[f];
                for bw in 0..nwb {
                    let mut bbits = self.bullets.alive[bw];
                    while bbits != 0 {
                        let b = bw * 64 + bbits.trailing_zeros() as usize;
                        bbits &= bbits - 1;
                        if self.bullets.delay[b] > 0 {
                            continue; // delay 弹不参与
                        }
                        let dx = self.bullets.x[b] - fx;
                        let dy = self.bullets.y[b] - fy;
                        let d2 = len_sq(dx, dy);
                        let sum = (fr + self.bullets.radius[b]).raw() as i64;
                        if d2 <= sum * sum {
                            self.push_hit(ROW_FIELD_BULLET, f as u16, b as u16);
                        }
                    }
                }
            }
        }
    }

    /// 行 7：作用区（field.radius）× 敌人 hurtbox（受击圈，与行 4 同）→ 扣血。
    /// **不查敌 invuln**（事件照收、结算时判，与行 4 同规）。
    fn collide_field_enemy(&mut self) {
        let nwf = self.fields.alive.len();
        let nwe = self.enemies.alive.len();
        for fw in 0..nwf {
            let mut fbits = self.fields.alive[fw];
            while fbits != 0 {
                let f = fw * 64 + fbits.trailing_zeros() as usize;
                fbits &= fbits - 1;
                if self.fields.flags[f] & FIELD_DAMAGE == 0 {
                    continue;
                }
                let (fx, fy) = (self.fields.x[f], self.fields.y[f]);
                let fr = self.fields.radius[f];
                for ew in 0..nwe {
                    let mut ebits = self.enemies.alive[ew];
                    while ebits != 0 {
                        let e = ew * 64 + ebits.trailing_zeros() as usize;
                        ebits &= ebits - 1;
                        let dx = self.enemies.x[e] - fx;
                        let dy = self.enemies.y[e] - fy;
                        let d2 = len_sq(dx, dy);
                        let sum = (fr + self.enemies.hurtbox[e]).raw() as i64;
                        if d2 <= sum * sum {
                            self.push_hit(ROW_FIELD_ENEMY, f as u16, e as u16);
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::math::Fx;
    #[cfg(debug_assertions)]
    use crate::world::PH_COLLIDE;
    use crate::world::test_support::*;

    #[test]
    fn collide_bullet_on_player_collects_hit_and_graze() {
        use crate::events::{ROW_BULLET_PLAYER_GRAZE, ROW_BULLET_PLAYER_HIT};
        let mut w = crate::step::World::new(1);
        // 自机在 (0,384)，hit_radius=2.5、graze_radius=16。弹压在自机身上 → 中弹+擦弹都收。
        w.body.players[0].x = Fx::ZERO;
        w.body.players[0].y = Fx::from_int(384);
        bullet_at(&mut w, 0, 384);
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide(&crate::tables::TABLES_V0);
        let hit = (0..w.body.hits_len as usize)
            .filter(|&k| w.body.hits[k].row == ROW_BULLET_PLAYER_HIT)
            .count();
        let graze = (0..w.body.hits_len as usize)
            .filter(|&k| w.body.hits[k].row == ROW_BULLET_PLAYER_GRAZE)
            .count();
        assert_eq!(hit, 1);
        assert_eq!(graze, 1);
    }

    #[test]
    fn collide_near_bullet_grazes_only() {
        use crate::events::{ROW_BULLET_PLAYER_GRAZE, ROW_BULLET_PLAYER_HIT};
        let mut w = crate::step::World::new(1);
        w.body.players[0].x = Fx::ZERO;
        w.body.players[0].y = Fx::from_int(384);
        bullet_at(&mut w, 10, 384); // 距 10px：在 graze 圈(≈18)内、hit 圈(≈4.5)外
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide(&crate::tables::TABLES_V0);
        let hit = (0..w.body.hits_len as usize)
            .filter(|&k| w.body.hits[k].row == ROW_BULLET_PLAYER_HIT)
            .count();
        let graze = (0..w.body.hits_len as usize)
            .filter(|&k| w.body.hits[k].row == ROW_BULLET_PLAYER_GRAZE)
            .count();
        assert_eq!(hit, 0);
        assert_eq!(graze, 1);
    }

    #[test]
    fn collide_skips_invuln_player() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].x = Fx::ZERO;
        w.body.players[0].y = Fx::from_int(384);
        w.body.players[0].invuln = 60; // 无敌 → 不参与
        bullet_at(&mut w, 0, 384);
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide(&crate::tables::TABLES_V0);
        assert_eq!(w.body.hits_len, 0);
    }

    #[test]
    fn collide_skips_delay_bullet() {
        let mut w = crate::step::World::new(1);
        w.body.players[0].x = Fx::ZERO;
        w.body.players[0].y = Fx::from_int(384);
        bullet_at(&mut w, 0, 384);
        // 把刚造的弹设 delay>0（索引 0）
        w.body.bullets.delay[0] = 5;
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide(&crate::tables::TABLES_V0);
        assert_eq!(w.body.hits_len, 0);
    }

    #[test]
    fn collide_enemy_body_on_player() {
        use crate::events::ROW_BODY_PLAYER_HIT;
        let mut w = crate::step::World::new(1);
        w.body.players[0].x = Fx::ZERO;
        w.body.players[0].y = Fx::from_int(100);
        spawn_enemy(&mut w, 0, 100, 5); // 敌体 radius 12 + 自机 hit 2.5 → 圆心重合必撞
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide(&crate::tables::TABLES_V0);
        let n = (0..w.body.hits_len as usize)
            .filter(|&k| w.body.hits[k].row == ROW_BODY_PLAYER_HIT)
            .count();
        assert_eq!(n, 1);
    }

    // D8 双半径不对称的判别式测试：行 3（敌体×自机）必须用 enemy.radius（体碰，小），
    // 不能用 enemy.hurtbox（受击，大）。圆心重合（d2=0）没法判别——任何正半径和都会命中；
    // 必须选一个"卡在两个半径和之间"的距离才能让写反的代码露馅，所以这是新增负向测试而非
    // 修改 collide_enemy_body_on_player（那个测试仍保留，用来证明行 3 本身会触发）。
    //
    // 几何：player.hit_radius=2.5，spawn_enemy 固定 radius=12 / hurtbox=16。
    // 轴对齐偏移 15px → d2 = 15² = 225。
    //   正确（radius）：sum = 12+2.5 = 14.5 → 14.5² = 210.25 < 225 → 不命中。
    //   写反（hurtbox）：sum = 16+2.5 = 18.5 → 18.5² = 342.25 > 225 → 命中——测试就会失败。
    #[test]
    fn collide_body_uses_body_radius_not_hurtbox() {
        use crate::events::ROW_BODY_PLAYER_HIT;
        let mut w = crate::step::World::new(1);
        w.body.players[0].x = Fx::ZERO;
        w.body.players[0].y = Fx::from_int(100);
        spawn_enemy(&mut w, 15, 100, 5);
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide(&crate::tables::TABLES_V0);
        let n = (0..w.body.hits_len as usize)
            .filter(|&k| w.body.hits[k].row == ROW_BODY_PLAYER_HIT)
            .count();
        assert_eq!(n, 0);
    }

    /// **人类裁定 D-9（spec `2026-07-30-enemy-death-effect-design.md` §3/§6）：
    /// dying 敌当帧仍参与体碰、仍撞得死自机。这不是 bug，别"修好"它。**
    ///
    /// 相位 6 **故意不看** `ENEMY_DYING`——只有相位 7 settle 的伤害臂看。于是一只被
    /// `die()`（相位 2）掉的敌，当帧的体碰照样成立；伤害致死那条路同性质，只是它的
    /// dying 在相位 7 才置、相位 6 早已碰完，`die()` 把这个窗口拉长到了整帧。裁定理由：
    /// 加一条"dying 不参与碰撞"的例外会让 D8 碰撞矩阵多一条隐式规则，而"神风敌撞死你"
    /// 本就说得通。
    ///
    /// 这条测试是这个裁定在全仓的**唯一**回归保护（全支线复审 Critical #2 补：`PROGRESS.md`
    /// 曾声称它有测试钉死，实际零覆盖）。它守的是"未来最可能被当 bug 顺手修掉"的那条语义：
    /// 有人在 `collide_body_player` 里加一句 dying 门禁，三平台金向量闸门**会一起放行**
    /// （闸门只比跨平台一致性，抓不到行为回归，见 CLAUDE.md「金向量闸门的能力边界」）。
    ///
    /// **变异实测**（本轮）：在 `collide_body_player` 的敌人循环里加
    /// `if self.enemies.flags[e] & ENEMY_DYING != 0 { continue; }` → 本测试红在
    /// "dying 敌的体碰必须照样收（人类裁定 D-9）"（left: 0, right: 1）。
    #[test]
    fn collide_dying_enemy_still_body_hits_player() {
        use crate::events::ROW_BODY_PLAYER_HIT;
        use crate::player::{DEATHBOMB_WINDOW, LIFE_DEATHWINDOW};
        let mut w = crate::step::World::new(1);
        w.body.players[0].x = Fx::ZERO;
        w.body.players[0].y = Fx::from_int(100);
        // 敌体 radius 12 + 自机 hit 2.5，圆心重合——本条测的是 dying 门禁的有无，
        // 不是半径映射（那条判别力归 `collide_body_uses_body_radius_not_hurtbox`）。
        let e = spawn_enemy(&mut w, 0, 100, 5);
        let ei = w.body.enemies.get(e).unwrap();
        // 直接置旗 = `die()` 与"被打死"两条路径在相位 6 眼里的共同状态
        // （`kill_enemy` 除了置旗还撒掉落/加分，与本条无关，不引进来当噪声）。
        w.body.enemies.flags[ei] |= crate::enemy::ENEMY_DYING;
        assert!(
            w.body.enemies.is_alive(ei),
            "前提：相位 9 才收尸，此刻槽还活着"
        );

        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide(&crate::tables::TABLES_V0);
        assert_eq!(
            (0..w.body.hits_len as usize)
                .filter(|&k| w.body.hits[k].row == ROW_BODY_PLAYER_HIT)
                .count(),
            1,
            "dying 敌的体碰必须照样收（人类裁定 D-9）"
        );

        // 下游腿：收了还得真生效——settle 把自机推进决死窗口（"仍撞死自机"这半句）。
        w.body.settle(&crate::tables::TABLES_V0);
        assert_eq!(
            w.body.players[0].life_state, LIFE_DEATHWINDOW,
            "dying 敌撞死自机（D-9 的后半句）"
        );
        assert_eq!(w.body.players[0].state_timer, DEATHBOMB_WINDOW);
    }

    // 几何同上一条判别式思路，但行 4（自机弹×敌人）用 enemy.hurtbox（受击，大），
    // 用轴对齐 18px 偏移即可正向判别（不需要额外负向测试）：
    //   正确（hurtbox）：sum = shot.radius(4)+16 = 20 → 20² = 400 > d2(18²=324) → 命中。
    //   写反（radius）  ：sum = 4+12 = 16 → 16² = 256 < 324 → 不命中——测试就会失败。
    #[test]
    fn collide_shot_on_enemy() {
        use crate::events::ROW_SHOT_ENEMY;
        let mut w = crate::step::World::new(1);
        let e = spawn_enemy(&mut w, 0, 80, 5);
        let ei = w.body.enemies.get(e).unwrap();
        // 自机弹与敌人轴对齐偏移 18px（不再圆心重合，见上方注释的判别式几何）。
        w.body.create_player_shot(crate::shots::ShotInit {
            x: w.body.enemies.x[ei] + Fx::from_int(18),
            y: w.body.enemies.y[ei],
            vx: Fx::ZERO,
            vy: Fx::ZERO,
            damage: 1,
            radius: Fx::from_int(4),
            sprite: 0,
            owner: 0,
            flags: 0,
        });
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide(&crate::tables::TABLES_V0);
        let hits: Vec<_> = (0..w.body.hits_len as usize)
            .map(|k| w.body.hits[k])
            .filter(|h| h.row == ROW_SHOT_ENEMY)
            .collect();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].active, 0); // shot 索引
        assert_eq!(hits[0].passive as usize, ei); // enemy 索引
    }

    #[test]
    fn collide_field_bullet_discriminates_radius_sum() {
        use crate::events::ROW_FIELD_BULLET;
        use crate::field::FIELD_CLEAR_BULLETS;
        // field 半径 20 + 弹半径 2 = 和 22 → 21px 撞、23px 不撞（判别式，非圆心重合）
        let mut w = crate::step::World::new(1);
        spawn_field(&mut w, 0, 100, 20, FIELD_CLEAR_BULLETS, 1);
        bullet_at(&mut w, 21, 100); // 索引 0：内
        bullet_at(&mut w, 23, 100); // 索引 1：外
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide(&crate::tables::TABLES_V0);
        let hits: Vec<_> = (0..w.body.hits_len as usize)
            .map(|k| w.body.hits[k])
            .filter(|h| h.row == ROW_FIELD_BULLET)
            .collect();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].active, 0); // field 索引
        assert_eq!(hits[0].passive, 0); // 只有 21px 那颗
    }

    #[test]
    fn collide_field_skips_bullets_without_clear_bit() {
        use crate::events::ROW_FIELD_BULLET;
        use crate::field::FIELD_DAMAGE;
        // 只开 DAMAGE 位的 field 压着弹 → 不消弹
        let mut w = crate::step::World::new(1);
        spawn_field(&mut w, 0, 100, 20, FIELD_DAMAGE, 1);
        bullet_at(&mut w, 0, 100);
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide(&crate::tables::TABLES_V0);
        assert_eq!(
            (0..w.body.hits_len as usize)
                .filter(|&k| w.body.hits[k].row == ROW_FIELD_BULLET)
                .count(),
            0
        );
    }

    #[test]
    fn collide_field_enemy_uses_hurtbox() {
        use crate::events::ROW_FIELD_ENEMY;
        use crate::field::FIELD_DAMAGE;
        // field 半径 20 + 敌 hurtbox 16 = 和 36；若误用敌 radius 12 → 和 32
        // 敌人放 34px：正确(≤36)撞；误用 radius(≤32) 则不撞 → 判别式
        let mut w = crate::step::World::new(1);
        spawn_field(&mut w, 0, 100, 20, FIELD_DAMAGE, 1);
        spawn_enemy(&mut w, 34, 100, 5);
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide(&crate::tables::TABLES_V0);
        assert_eq!(
            (0..w.body.hits_len as usize)
                .filter(|&k| w.body.hits[k].row == ROW_FIELD_ENEMY)
                .count(),
            1
        );
    }

    #[test]
    fn collide_field_skips_enemy_without_damage_bit() {
        use crate::events::ROW_FIELD_ENEMY;
        use crate::field::FIELD_CLEAR_BULLETS;
        // 只开 CLEAR 位的 field 压着敌人 → 不伤敌
        let mut w = crate::step::World::new(1);
        spawn_field(&mut w, 0, 100, 20, FIELD_CLEAR_BULLETS, 1);
        spawn_enemy(&mut w, 0, 100, 5);
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = PH_COLLIDE;
        }
        w.body.collide(&crate::tables::TABLES_V0);
        assert_eq!(
            (0..w.body.hits_len as usize)
                .filter(|&k| w.body.hits[k].row == ROW_FIELD_ENEMY)
                .count(),
            0
        );
    }
}
