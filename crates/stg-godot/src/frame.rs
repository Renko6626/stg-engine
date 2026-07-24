//! 纯 Rust:四渲染层 → f32 实例缓冲编码器。**定点→浮点唯一转换点**(I1 边界)。
//! 布局(spec §11.2,GLES3 源码+headless 实测双源):
//! 12 float/实例 = [xx, yx, 0, ox, xy, yy, 0, oy] + custom[sprite, 0, 0, 0]。

use stg_core::bullets::BulletPool;
use stg_core::enemy::EnemyPool;
use stg_core::items::ItemPool;
use stg_core::math::Fx;
use stg_core::shots::ShotPool;
use stg_core::tables::WorldTables;
use stg_core::world::WorldView;

pub const LAYER_BULLETS: usize = 0;
pub const LAYER_SHOTS: usize = 1;
pub const LAYER_ENEMIES: usize = 2;
pub const LAYER_ITEMS: usize = 3;
pub const LAYER_COUNT: usize = 4;
pub const FLOATS_PER_INSTANCE: usize = 12;

pub fn layer_cap(layer: usize) -> usize {
    match layer {
        LAYER_BULLETS => BulletPool::CAP,
        LAYER_SHOTS => ShotPool::CAP,
        LAYER_ENEMIES => EnemyPool::CAP,
        LAYER_ITEMS => ItemPool::CAP,
        _ => 0,
    }
}

#[inline]
fn fx_f32(v: Fx) -> f32 {
    v.raw() as f32 / 65536.0
}

#[inline]
fn write_instance(out: &mut [f32], slot: usize, x: f32, y: f32, cos: f32, sin: f32, sprite: u16) {
    let o = slot * FLOATS_PER_INSTANCE;
    out[o..o + FLOATS_PER_INSTANCE].copy_from_slice(&[
        cos,
        -sin,
        0.0,
        x,
        sin,
        cos,
        0.0,
        y,
        sprite as f32,
        0.0,
        0.0,
        0.0,
    ]);
}

/// 活槽压实(池索引升序)写 `out` 前缀,返活数。`out.len() == layer_cap(layer)*12`。
/// 未知 layer → 0(P4-b no-op)。
pub fn encode_layer(
    view: WorldView<'_>,
    tables: &WorldTables,
    layer: usize,
    out: &mut [f32],
) -> u32 {
    let mut n: usize = 0;
    match layer {
        LAYER_BULLETS => {
            let p = view.bullets();
            let (xs, ys, angles, sprites) = (p.x(), p.y(), p.angle(), p.sprite());
            for i in p.iter_alive() {
                let rad = angles[i].raw() as f32 * (core::f32::consts::TAU / 65536.0);
                write_instance(
                    out,
                    n,
                    fx_f32(xs[i]),
                    fx_f32(ys[i]),
                    rad.cos(),
                    rad.sin(),
                    sprites[i],
                );
                n += 1;
            }
        }
        LAYER_SHOTS => {
            let p = view.shots();
            let (xs, ys, sprites) = (p.x(), p.y(), p.sprite());
            for i in p.iter_alive() {
                write_instance(out, n, fx_f32(xs[i]), fx_f32(ys[i]), 1.0, 0.0, sprites[i]);
                n += 1;
            }
        }
        LAYER_ENEMIES => {
            let p = view.enemies();
            let (xs, ys, sprites) = (p.x(), p.y(), p.sprite());
            for i in p.iter_alive() {
                write_instance(out, n, fx_f32(xs[i]), fx_f32(ys[i]), 1.0, 0.0, sprites[i]);
                n += 1;
            }
        }
        LAYER_ITEMS => {
            let p = view.items();
            let (xs, ys, types) = (p.x(), p.y(), p.item_type());
            // A1 渲染 join:循环前提栈上 LUT(spec §2.1 性能拍板)
            let lut: [u16; stg_core::items::ITEM_TYPE_COUNT] =
                core::array::from_fn(|t| tables.item_cfg[t].sprite);
            for i in p.iter_alive() {
                let s = lut[types[i] as usize];
                write_instance(out, n, fx_f32(xs[i]), fx_f32(ys[i]), 1.0, 0.0, s);
                n += 1;
            }
        }
        _ => {}
    }
    n as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    // 机械调整许可②(task-3-brief.md):`fire` builtin 实际签名(builtins.rs)是 7 参
    // `(appearance:Int, x:Fx, y:Fx, speed:Fx, angle:Angle, xf:XformRef, task:SubRef)`——
    // 简报草稿假设的 8 参 `(..., xform_off, xform_cnt, task_script)` 与之不符(xf/task 在
    // 表层是"标识符或 none"，不是求值参数，编译期解析——见 docs/ecl-lang.md `fire` 条目)，
    // 故按 builtins.rs 改成 7 参、xf/task 位填 `none`；angle 参数类型是 Ty::Angle，无隐式
    // 转换(typeck `check_builtin_call_args`：`t.ty == *pty` 严格匹配)，故整数字面量 `0`/
    // `16384` 须换成角度字面量后缀(`deg`/`bam`)——`0deg`→BAM 0，`16384bam`→BAM 16384(=90°)，
    // 与简报原意的"角度 0"/"角度 16384" 数值等价，只是补上语言要求的类型后缀。
    const SRC: &str = r#"
sub main() {
    _ = spawn_enemy(-96.0fx, -64.0fx, 100, 0, 0);
    _ = drop_item(32.0fx, 48.0fx, 2);
    _ = fire(1, 10.0fx, 20.0fx, 0.0fx, 0deg, none, none);
    _ = fire(1, 11.0fx, 21.0fx, 0.0fx, 16384bam, none, none);
    loop { wait(60); }
}
"#;

    fn stepped_game() -> crate::boot::Game {
        let mut g = crate::boot::boot(SRC, 7, 2).expect("boot");
        // 两帧:root task 的 `born_frame` 记于 `start_main`(彼时 `body.frame==0`);
        // `run_tasks`(相位 2)门禁"出生当帧不跑"(`born_frame == frame`),而 `advance()`
        // (相位 10,帧号 += 1)在同一 step 调用的最后才发生——故第 1 次 step 内
        // `run_tasks` 跑时 frame 仍是 0 == born_frame,整条 main 主体(spawn_enemy/
        // drop_item/fire ×2)被跳过不执行;第 2 次 step 时 frame==1 != born_frame(0),
        // 门禁放行,main 才真正从头跑到 `wait(60)` 挂起。两帧后实体才存在。
        let input = stg_core::input::InputFrame::empty(0);
        stg_core::step::step_with_director(&mut g.world, g.tables, &g.image, &input, |_| {});
        stg_core::step::step_with_director(&mut g.world, g.tables, &g.image, &input, |_| {});
        g
    }

    #[test]
    fn bullets_layer_transform_and_custom() {
        let g = stepped_game();
        let mut out = vec![0.0f32; layer_cap(LAYER_BULLETS) * FLOATS_PER_INSTANCE];
        let n = encode_layer(g.world.view(), g.tables, LAYER_BULLETS, &mut out);
        assert_eq!(n, 2);
        // 弹0:angle=0 → cos1/sin0 精确;pos=(10,20) 速度0不动
        assert_eq!(&out[0..8], &[1.0, -0.0, 0.0, 10.0, 0.0, 1.0, 0.0, 20.0]);
        let expect_sprite = stg_core::tables::TABLES_V0.appearances[1].sprite as f32;
        assert_eq!(out[8], expect_sprite);
        assert_eq!(&out[9..12], &[0.0, 0.0, 0.0]);
        // 弹1:angle=16384(90°) → cos≈0/sin≈1;压实序=池索引升序
        let b1 = &out[12..24];
        assert!(b1[0].abs() < 1e-6 && (b1[4] - 1.0).abs() < 1e-6);
        assert_eq!((b1[3], b1[7]), (11.0, 21.0));
    }

    #[test]
    fn enemies_layer_exact() {
        let g = stepped_game();
        let mut out = vec![0.0f32; layer_cap(LAYER_ENEMIES) * FLOATS_PER_INSTANCE];
        let n = encode_layer(g.world.view(), g.tables, LAYER_ENEMIES, &mut out);
        assert_eq!(n, 1);
        assert_eq!(&out[0..8], &[1.0, -0.0, 0.0, -96.0, 0.0, 1.0, 0.0, -64.0]);
        assert_eq!(out[8], 0.0, "spawn_enemy sprite 固定 0");
    }

    #[test]
    fn items_layer_sprite_join() {
        let g = stepped_game();
        let mut out = vec![0.0f32; layer_cap(LAYER_ITEMS) * FLOATS_PER_INSTANCE];
        let n = encode_layer(g.world.view(), g.tables, LAYER_ITEMS, &mut out);
        assert_eq!(n, 1);
        // 期望值推导(场景假设纠偏——原简报断言"x 不受弹射影响"==32.0 不成立):
        // `drop_item` 底层 `spawn_drop`(world.rs)给道具一个 PCG32 派生的随机喷发速度
        // `vx`(±1.0fx 范围)；`integrate`(相位5)对**未被磁吸**的道具无条件 `x += vx`
        // (`world/integrate.rs::integrate_item` "未锁定/刚解锁" 分支——磁吸判定优先但
        // 本景不命中：自机出生点 (0,384)(`PlayerState::spawn`)既不满足 PoC 线
        // `y<128`、也不在该道具类型 `attract_radius`(40fx)内,距离≈337fx)。道具在
        // `stepped_game()` 第 2 次 step 内相位2(director)诞生、同一 step 相位5
        // (integrate)立即挨一次 vx 位移,故 x 必偏离喷出点 32.0fx。
        // seed=7/rank=2 下,drop_item 是全程唯一摸 RNG 的调用(spawn_enemy/fire 均不
        // 耗 RNG)，PCG32(vendored,I3,跨平台位级确定)算出 raw vx=-39629；
        // 32.0fx(raw 2097152)+vx = raw 2057523，即下式(65536 是 2 的幂,该除法
        // 在 f32 精确无舍入,同生产 `fx_f32`)。
        assert_eq!(
            out[3],
            2057523_f32 / 65536.0, // raw i32 2057523(见上)同 `fx_f32` 换算公式
            "x = 起点 32.0fx + 确定性 RNG 喷发 vx(raw -39629)"
        );
        let expect = stg_core::tables::TABLES_V0.item_cfg[2].sprite as f32;
        assert_eq!(out[8], expect, "item_type→表 join(A1)");
    }

    #[test]
    fn empty_layer_returns_zero() {
        let g = stepped_game();
        let mut out = vec![0.0f32; layer_cap(LAYER_SHOTS) * FLOATS_PER_INSTANCE];
        assert_eq!(
            encode_layer(g.world.view(), g.tables, LAYER_SHOTS, &mut out),
            0
        );
    }
}
