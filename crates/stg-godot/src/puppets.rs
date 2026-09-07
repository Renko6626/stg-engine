//! 纯 Rust：敌人木偶喂料（表现契约 v2 spec §4.4）。零 gdext 类型。
//!
//! 敌层不再走 MultiMesh（拍板 ④甲案：cap 256、常态几十，节点开销可忽略，换来
//! AnimationPlayer/AnimationTree 与编辑器可视化）。壳侧按池索引预分配节点，每帧读这里的
//! 压缩列：`(index, gen)` 识别新生/复用，`(sprite, anm_state, state_age)` 选帧，`hit_flash`
//! 叠白。**没有 `dying` 列**——敌人 dying 位在 settle 置、同一 step 的 cleanup 回收，step
//! 后壳侧永远读不到 dying == 1；死亡动画数据源是 `REQ_ENEMY_DEATH`。
//!
//! 自机**不进**这里（继续走 `player_pos()` + `hud_player()`）。

use stg_core::world::WorldView;

/// 一帧的敌人木偶列（按池索引升序压实，各列等长）。
#[derive(Default, Debug, Clone, PartialEq)]
pub struct PuppetCols {
    pub index: Vec<i32>,
    pub generation: Vec<i32>,
    pub x: Vec<f32>,
    pub y: Vec<f32>,
    pub sprite: Vec<i32>,
    pub anm_state: Vec<i32>,
    /// `frame − anm_state_frame`（step 结束后的帧号；首帧 = 1，render-contract §0）。
    pub state_age: Vec<i32>,
    pub hit_flash: Vec<i32>,
}

impl PuppetCols {
    pub fn len(&self) -> usize {
        self.index.len()
    }
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }
}

#[inline]
fn fx_f32(v: stg_core::math::Fx) -> f32 {
    v.raw() as f32 / 65536.0
}

/// `frame` = step 结束后的 `world.frame()`。
pub fn encode_puppets(view: WorldView<'_>, frame: u32) -> PuppetCols {
    let p = view.enemies();
    let (xs, ys) = (p.x(), p.y());
    let (sprites, states, state_frames, flashes) = (
        p.sprite(),
        p.anm_state(),
        p.anm_state_frame(),
        p.hit_flash(),
    );
    let mut c = PuppetCols::default();
    for i in p.iter_alive() {
        c.index.push(i as i32);
        c.generation.push(p.generation_of(i) as i32);
        c.x.push(fx_f32(xs[i]));
        c.y.push(fx_f32(ys[i]));
        c.sprite.push(sprites[i] as i32);
        c.anm_state.push(states[i] as i32);
        c.state_age
            .push(frame.wrapping_sub(state_frames[i]).min(i32::MAX as u32) as i32);
        c.hit_flash.push(flashes[i] as i32);
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = r#"
sub main() {
    _ = spawn_enemy(-96.0fx, -64.0fx, 100, 0, 0, 3, none);
    _ = spawn_enemy(40.0fx, 24.0fx, 100, 0, 0, 5, none);
    loop { wait(60); }
}
"#;

    fn stepped(n: usize) -> crate::boot::Game {
        let mut g = crate::boot::boot(SRC, 7, 2).expect("boot");
        let input = stg_core::input::InputFrame::empty(0);
        for _ in 0..n {
            stg_core::step::step_with_director(&mut g.world, g.tables, &g.image, &input, |_| {});
        }
        g
    }

    #[test]
    fn empty_world_yields_empty_columns() {
        let g = crate::boot::boot("sub main() { }", 7, 2).expect("boot");
        let c = encode_puppets(g.world.view(), g.world.frame());
        assert!(c.is_empty());
        assert_eq!(c, PuppetCols::default());
    }

    /// 两敌按池索引升序压实；坐标/sprite 逐列对拍；`state_age` 首帧 = 1（main 在第 2 次
    /// step 才真正跑——见 frame.rs `stepped_game` 的 born-frame 门禁说明——敌在 frame==1
    /// 时出生，step 后 frame==2，故 age = 1）。
    #[test]
    fn two_enemies_compacted_in_index_order_with_age_one() {
        let g = stepped(2);
        let c = encode_puppets(g.world.view(), g.world.frame());
        assert_eq!(c.len(), 2);
        assert_eq!(c.index, vec![0, 1]);
        assert_eq!(c.generation, vec![1, 1], "首次分配 gen 从 1 起");
        assert_eq!(c.x, vec![-96.0, 40.0]);
        assert_eq!(c.y, vec![-64.0, 24.0]);
        assert_eq!(c.sprite, vec![3, 5]);
        assert_eq!(c.anm_state, vec![0, 0]);
        assert_eq!(c.state_age, vec![1, 1], "出生后首次可见 age = 1");
        assert_eq!(c.hit_flash, vec![0, 0]);
    }

    /// `state_age` 随帧增长；`set_anm_state` 由核内写 API 直调后重新从 1 起算。
    #[test]
    fn state_age_grows_and_restarts_on_set_anm_state() {
        let mut g = stepped(6);
        let c = encode_puppets(g.world.view(), g.world.frame());
        assert_eq!(c.state_age, vec![5, 5]);
        let h = stg_core::enemy::EnemyHandle {
            index: 1,
            generation: 1,
        };
        g.world.body.set_anm_state(h, 4);
        let c = encode_puppets(g.world.view(), g.world.frame());
        assert_eq!(c.anm_state, vec![0, 4]);
        assert_eq!(c.state_age[1], 0, "同帧内写完即读：age 0");
        let input = stg_core::input::InputFrame::empty(0);
        stg_core::step::step_with_director(&mut g.world, g.tables, &g.image, &input, |_| {});
        let c = encode_puppets(g.world.view(), g.world.frame());
        assert_eq!(c.state_age, vec![6, 1]);
    }
}
