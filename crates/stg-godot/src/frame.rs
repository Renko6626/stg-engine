//! 纯 Rust:三渲染层 → f32 实例缓冲编码器。**定点→浮点唯一转换点**(I1 边界)。
//! 布局(spec §11.2,GLES3 源码+headless 实测双源):
//! 12 float/实例 = [xx, yx, 0, ox, xy, yy, 0, oy] + custom[age, 0, 0, 0]。
//!
//! **敌层已退役**(表现契约 v2,2026-09-07,拍板 ④甲案):敌人走节点木偶,喂料见
//! `crate::puppets`。三层保留:bullets / shots / items。custom.y 只在弹层有语义(弹龄),
//! 其余层恒 0;custom.z/w 预留,stride 12 冻结。

use stg_core::bullets::BulletPool;
use stg_core::items::ItemPool;
use stg_core::math::{Angle, Fx, sincos};
use stg_core::shots::ShotPool;
use stg_core::tables::WorldTables;
use stg_core::world::WorldView;

pub const LAYER_BULLETS: usize = 0;
pub const LAYER_SHOTS: usize = 1;
pub const LAYER_ITEMS: usize = 2;
pub const LAYER_COUNT: usize = 3;
pub const FLOATS_PER_INSTANCE: usize = 12;

pub fn layer_cap(layer: usize) -> usize {
    match layer {
        LAYER_BULLETS => BulletPool::CAP,
        LAYER_SHOTS => ShotPool::CAP,
        LAYER_ITEMS => ItemPool::CAP,
        _ => 0,
    }
}

#[inline]
fn fx_f32(v: Fx) -> f32 {
    v.raw() as f32 / 65536.0
}

#[inline]
#[allow(clippy::too_many_arguments)] // 编码器内部展开,调用点只有三处、参数序即缓冲布局序
fn write_instance(
    out: &mut [f32],
    slot: usize,
    x: f32,
    y: f32,
    cos: f32,
    sin: f32,
    sprite: u16,
    custom_y: f32,
) {
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
        custom_y,
        0.0,
        0.0,
    ]);
}

/// 弹的实例基（返回 `(cos, sin)`）：**渲染朝向 = 速度方向 + 四分之一圈**。
///
/// 两个基准差 90°，必须补：世界侧 `polar_to_vec = (speed·cos, speed·sin)`，所以
/// **BAM 0 指 +x（右）**；而图集里的弹**画的是头朝上**（原作弹片惯例——arrowhead /
/// kunai / laser / rice 都是竖着画的）。不补的话，一颗朝上飞的弹（BAM 49152）会被
/// 转 270°、渲染成头朝左。占位圆看不出来，真美术一上就露馅。
///
/// 补 +16384（90°）之后：`49152 + 16384 ≡ 0` → **朝上飞的弹不旋转**，正好头朝上；
/// BAM 0（朝右飞）转 90°（屏幕 y 向下，正角即顺时针）→ 头朝右。
///
/// **查核内烘焙表，不调 libm**（表现契约 v2 §4.3）：满弹 8192 颗 = 每帧 1.6 万次
/// `sin`/`cos` 浮点调用，而 `stg_core::math::sincos` 是同一颗弹在积分相位已经查过的
/// 那张表；Q16.16 → f32 无损（`raw / 65536`，2 的幂除法精确）。
///
/// **只有弹层需要这个补偿**：自机弹层不旋转（sprite 本就朝上、也只朝上飞），道具层同理
/// 走单位基。见 `docs/render-contract.md` §2。
fn bullet_basis(a: Angle) -> (f32, f32) {
    const SPRITE_UP_QUARTER_TURN: u16 = 16384;
    let (s, c) = sincos(Angle(a.raw().wrapping_add(SPRITE_UP_QUARTER_TURN)));
    (fx_f32(c), fx_f32(s))
}

/// 弹龄（表现契约 v2 §2.4）：`frame` 是 **step 结束后**的帧号，故首次可见 = 1。
/// 饱和到 u16 域后转 f32（shader 侧只用小 t）。
#[inline]
fn bullet_age(frame: u32, born: u32) -> f32 {
    frame.wrapping_sub(born).min(u16::MAX as u32) as f32
}

/// 活槽压实(池索引升序)写 `out` 前缀,返活数。`out.len() == layer_cap(layer)*12`。
/// `frame` = step 结束后的 `world.frame()`(弹龄用)。未知 layer → 0(P4-b no-op)。
pub fn encode_layer(
    view: WorldView<'_>,
    tables: &WorldTables,
    frame: u32,
    layer: usize,
    out: &mut [f32],
) -> u32 {
    let mut n: usize = 0;
    match layer {
        LAYER_BULLETS => {
            let p = view.bullets();
            let (xs, ys, angles, sprites, born) =
                (p.x(), p.y(), p.angle(), p.sprite(), p.born_frame());
            for i in p.iter_alive() {
                let (cos, sin) = bullet_basis(angles[i]);
                write_instance(
                    out,
                    n,
                    fx_f32(xs[i]),
                    fx_f32(ys[i]),
                    cos,
                    sin,
                    sprites[i],
                    bullet_age(frame, born[i]),
                );
                n += 1;
            }
        }
        LAYER_SHOTS => {
            let p = view.shots();
            let (xs, ys, sprites) = (p.x(), p.y(), p.sprite());
            for i in p.iter_alive() {
                write_instance(
                    out,
                    n,
                    fx_f32(xs[i]),
                    fx_f32(ys[i]),
                    1.0,
                    0.0,
                    sprites[i],
                    0.0,
                );
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
                write_instance(out, n, fx_f32(xs[i]), fx_f32(ys[i]), 1.0, 0.0, s, 0.0);
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

    // 颜色轴刀(2026-07-26):`fire` 首参拆成 `shape, color` 两参(编译器折叠成一个
    // appearance 值)。这里写 `fire(0, 1, ...)` = 0 号形第 1 色,折叠后恰是旧的
    // appearance id `1`——下方 `TABLES_V0.appearances[1]` 的 sprite 断言口径不变。
    //
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
    _ = spawn_enemy(-96.0fx, -64.0fx, 100, 0, 0, 0, none);
    _ = drop_item(32.0fx, 48.0fx, 2);
    _ = fire(0, 1, 10.0fx, 20.0fx, 0.0fx, 0deg, none, none);
    _ = fire(0, 1, 11.0fx, 21.0fx, 0.0fx, 16384bam, none, none);
    loop { wait(60); }
}
"#;

    fn step_once(g: &mut crate::boot::Game) {
        let input = stg_core::input::InputFrame::empty(0);
        stg_core::step::step_with_director(&mut g.world, g.tables, &g.image, &input, |_| {});
    }

    fn stepped_game() -> crate::boot::Game {
        let mut g = crate::boot::boot(SRC, 7, 2).expect("boot");
        // 两帧:root task 的 `born_frame` 记于 `start_main`(彼时 `body.frame==0`);
        // `run_tasks`(相位 2)门禁"出生当帧不跑"(`born_frame == frame`),而 `advance()`
        // (相位 10,帧号 += 1)在同一 step 调用的最后才发生——故第 1 次 step 内
        // `run_tasks` 跑时 frame 仍是 0 == born_frame,整条 main 主体(spawn_enemy/
        // drop_item/fire ×2)被跳过不执行;第 2 次 step 时 frame==1 != born_frame(0),
        // 门禁放行,main 才真正从头跑到 `wait(60)` 挂起。两帧后实体才存在。
        step_once(&mut g);
        step_once(&mut g);
        g
    }

    fn encode(g: &crate::boot::Game, layer: usize) -> (u32, Vec<f32>) {
        let mut out = vec![0.0f32; layer_cap(layer) * FLOATS_PER_INSTANCE];
        let n = encode_layer(g.world.view(), g.tables, g.world.frame(), layer, &mut out);
        (n, out)
    }

    #[test]
    fn bullets_layer_transform_and_custom() {
        let g = stepped_game();
        let (n, out) = encode(&g, LAYER_BULLETS);
        assert_eq!(n, 2);
        // 位置与 sprite：压实序 = 池索引升序
        assert_eq!((out[3], out[7]), (10.0, 20.0)); // 弹0 pos=(10,20)，速度 0 不动
        let expect_sprite = stg_core::tables::TABLES_V0.appearances[1].sprite as f32;
        assert_eq!(out[8], expect_sprite);
        // custom.y = 弹龄：弹在 frame==1 时出生,step 后 frame==2 → 首次可见 age 1(§0 口径)
        assert_eq!(&out[9..12], &[1.0, 0.0, 0.0]);
        assert_eq!((out[15], out[19]), (11.0, 21.0)); // 弹1 pos=(11,21)

        // 招牌不变量：**贴图的"上"经实例变换后 == 速度方向**（贴图默认头朝上）。
        // 实例基是 [xx xy; yx yy] = [cos -sin; sin cos]（out 的 0/1/4/5 位）；Godot 2D 里
        // 局部"上"是 (0,-1)，变换后 = (-xy, -yy) = (sin, -cos)。世界侧速度方向则是
        // `polar_to_vec` 的 (cs, sn)。两者必须相等——这一条同时钉死了四分之一圈补偿的
        // **存在**与**方向**：去掉补偿或补反，两侧立刻对不上。
        let angles = g.world.view().bullets().angle();
        for (k, i) in g.world.view().bullets().iter_alive().enumerate() {
            let base = k * FLOATS_PER_INSTANCE;
            let (up_x, up_y) = (out[base + 4], -out[base + 5]); // (sin, -cos)
            let (sn, cs) = stg_core::math::sincos(angles[i]);
            let (dir_x, dir_y) = (fx_f32(cs), fx_f32(sn));
            assert!(
                (up_x - dir_x).abs() < 1e-3 && (up_y - dir_y).abs() < 1e-3,
                "弹{k}(BAM {}) 贴图朝向 ({up_x:.4},{up_y:.4}) 应等于速度方向 ({dir_x:.4},{dir_y:.4})",
                angles[i].raw()
            );
        }
    }

    /// 判别腿：朝**正上方**飞的弹（BAM 49152，`polar_to_vec` 得 (0,-1)）必须**不旋转**
    /// ——贴图本就头朝上。这一格是"补偿量恰好是 +90° 而不是 -90°/180°"的锚。
    /// 查表版:表在 BAM 0 处 sin=0/cos=1 **精确**(烘焙表端点),故容差可收到 1e-6。
    #[test]
    fn bullet_flying_up_renders_unrotated() {
        let (cos, sin) = bullet_basis(Angle(49152));
        assert!(
            (cos - 1.0).abs() < 1e-6 && sin.abs() < 1e-6,
            "朝上飞的弹必须以单位基渲染，实际 (cos,sin)=({cos},{sin})"
        );
        // 反向锚：朝右飞（BAM 0）必须转成头朝右 → (cos,sin)=(0,1)
        let (cos0, sin0) = bullet_basis(Angle(0));
        assert!(cos0.abs() < 1e-6 && (sin0 - 1.0).abs() < 1e-6);
    }

    /// 弹龄逐帧 +1（表现契约 v2 §4.2）：再走 5 步 → age 6。判别腿:两颗弹同龄,任一错位即红。
    #[test]
    fn bullet_age_counts_frames_since_birth() {
        let mut g = stepped_game();
        for _ in 0..5 {
            step_once(&mut g);
        }
        let (n, out) = encode(&g, LAYER_BULLETS);
        assert_eq!(n, 2);
        assert_eq!(out[9], 6.0);
        assert_eq!(out[FLOATS_PER_INSTANCE + 9], 6.0);
        // 非弹层 custom.y 恒 0
        let (_, items) = encode(&g, LAYER_ITEMS);
        assert_eq!(items[9], 0.0);
    }

    /// 敌层退役:层号 3(旧 LAYER_ENEMIES 的位置早已被 items 顶上)与任何 ≥ LAYER_COUNT 的
    /// 层号都是 P4-b no-op——cap 0、编码返 0。敌人走 `crate::puppets`。
    #[test]
    fn enemies_layer_is_retired() {
        assert_eq!(LAYER_COUNT, 3);
        assert_eq!(layer_cap(3), 0);
        let g = stepped_game();
        assert_eq!(
            encode_layer(g.world.view(), g.tables, g.world.frame(), 3, &mut []),
            0
        );
        let c = crate::puppets::encode_puppets(g.world.view(), g.world.frame());
        assert_eq!((c.x[0], c.y[0]), (-96.0, -64.0), "敌人从木偶喂料读");
    }

    #[test]
    fn items_layer_sprite_join() {
        let g = stepped_game();
        let (n, out) = encode(&g, LAYER_ITEMS);
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
        assert_eq!(encode(&g, LAYER_SHOTS).0, 0);
    }

    // 圆心重合纪律缺口补位(task-3-brief.md Step 3;CLAUDE.md M0-7 教训的同类变种——
    // `empty_layer_returns_zero` 只钉了 shots 层空池 `n==0`,判不出 ox/oy/xx 编码错位,
    // 圆心(全零)测试对坐标映射是瞎的)。持住 `BTN_SHOT` 让角色0 tier0 单发发射器
    // (`tables.rs` `BASE_SHOOTER`:`interval=4,delay=0`)在**首帧**即命中(`shot_timer`
    // 出生即 0,"先判后加"语义——见 player.rs `char0_update_shot` doc),拿池内真值与
    // 编码输出逐字段对拍(非空 + 坐标一致 + 无旋转)。
    #[test]
    fn shots_layer_nonempty_matches_pool() {
        let mut g = crate::boot::boot("sub main() { }", 7, 2).expect("boot");
        let mut input = stg_core::input::InputFrame::empty(g.world.frame());
        input.actions[0].buttons = stg_core::input::BTN_SHOT;
        for _ in 0..4 {
            input.frame = g.world.frame();
            stg_core::step::step_with_director(&mut g.world, g.tables, &g.image, &input, |_| {});
        }

        let p = g.world.view().shots();
        let first = p
            .iter_alive()
            .next()
            .expect("BTN_SHOT 持住 4 帧后 shots 池应非空(tier0 delay=0,首帧即发)");
        let (xs, ys) = (p.x(), p.y());
        let (expect_x, expect_y) = (fx_f32(xs[first]), fx_f32(ys[first]));

        let (n, out) = encode(&g, LAYER_SHOTS);
        assert!(n > 0, "shots 层应非空");
        assert_eq!(
            out[0], 1.0,
            "shots 层 xx 恒 1.0(write_instance 固定传 cos=1.0/sin=0.0,无旋转)"
        );
        assert_eq!(out[3], expect_x, "实例0 ox 应与池内首活自机弹坐标一致");
        assert_eq!(out[7], expect_y, "实例0 oy 应与池内首活自机弹坐标一致");
    }
}
