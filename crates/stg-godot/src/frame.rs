//! 纯 Rust:四渲染层 → f32 实例缓冲编码器。**定点→浮点唯一转换点**(I1 边界)。
//! 布局(spec §11.2,GLES3 源码+headless 实测双源):
//! 12 float/实例 = [xx, yx, 0, ox, xy, yy, 0, oy] + custom[.., .., 0, 0]。
//!
//! **敌层已退役**(表现契约 v2,2026-09-07,拍板 ④甲案):敌人走节点木偶,喂料见
//! `crate::puppets`。四层保留:bullets / shots / items / lasers(激光池刀 2026-09-25)。
//! custom.y 在弹层 = 弹龄、激光层 = alpha,其余层恒 0;custom.z/w 预留,stride 12 冻结。
//! **激光层不走图集**(截面渐变由 `laser.gdshader` 程序化生成):基自带缩放,见 `LAYER_LASERS`。

use stg_core::bullets::BulletPool;
use stg_core::items::ItemPool;
use stg_core::lasers::{LASER_FLAG_FADE_ALPHA, LaserPool};
use stg_core::math::{Angle, Fx, sincos};
use stg_core::shots::ShotPool;
use stg_core::tables::WorldTables;
use stg_core::world::WorldView;

pub const LAYER_BULLETS: usize = 0;
pub const LAYER_SHOTS: usize = 1;
pub const LAYER_ITEMS: usize = 2;
/// 激光层(激光池刀 2026-09-25 Task 6;spec §7):无图集,截面渐变在 shader 里程序化生成。
/// 实例布局同 stride 12,但**基已含缩放**(弹/自机弹/道具层把缩放交给 QuadMesh 尺寸):
/// 沿激光方向的轴长 = `end − start`、横向轴长 = 显示宽度,原点 = 线段中点;
/// `custom = [color, alpha, 0, 0]`(`color` = 池 `sprite` 存的颜色号 0..15)。
pub const LAYER_LASERS: usize = 3;
pub const LAYER_COUNT: usize = 4;
pub const FLOATS_PER_INSTANCE: usize = 12;

pub fn layer_cap(layer: usize) -> usize {
    match layer {
        LAYER_BULLETS => BulletPool::CAP,
        LAYER_SHOTS => ShotPool::CAP,
        LAYER_ITEMS => ItemPool::CAP,
        LAYER_LASERS => LaserPool::CAP,
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

/// 写入一条**基自带缩放**的实例（激光层用）。弹/自机弹/道具层继续走 `write_instance`——
/// 它们的 QuadMesh 尺寸承担缩放，单位基即够；激光长度/宽度逐实例变，只能塞进基。
/// 参数序 = `(x, y, xx, yx, xy, yy, custom_x, custom_y)`；写入时按 stride-12 布局展开成
/// `[xx, yx, 0, x, xy, yy, 0, y, custom_x, custom_y, 0, 0]`。
#[inline]
#[allow(clippy::too_many_arguments)]
fn write_instance_basis(
    out: &mut [f32],
    slot: usize,
    x: f32,
    y: f32,
    xx: f32,
    yx: f32,
    xy: f32,
    yy: f32,
    custom_x: f32,
    custom_y: f32,
) {
    let o = slot * FLOATS_PER_INSTANCE;
    out[o..o + FLOATS_PER_INSTANCE]
        .copy_from_slice(&[xx, yx, 0.0, x, xy, yy, 0.0, y, custom_x, custom_y, 0.0, 0.0]);
}

/// 激光的**显示宽度与 alpha**（spec §7；原作画法见 spec §2「贴图」）。核里只存判定宽度
/// `width` 与三态计时，画面宽度/淡出全在表现层算：
/// - 预警（state 0）：1.2 px 细线；最后 `min(warn, 30)` 帧线性长到全宽。
/// - 生效（state 1）：全宽。
/// - 收缩（state 2）：`flags` 位 0 为 1 时 alpha 线性到 0（宽度不动），否则宽度线性到 0。
///
/// `fade == 0` 不能除零（相位 7 被清弹 field 取消的激光会以 `state 2 / timer 0` 多留一帧，
/// 见 `docs/render-contract.md` 激光层一节）：`k = 0`。
#[inline]
fn laser_display(
    state: u8,
    timer: u16,
    warn: u16,
    fade: u16,
    width: f32,
    fade_alpha: bool,
) -> (f32, f32) {
    match state {
        0 => {
            let ramp = warn.min(30);
            let t0 = warn - ramp;
            if timer >= t0 && ramp > 0 {
                (1.2 + (width - 1.2) * (timer - t0) as f32 / ramp as f32, 1.0)
            } else {
                (1.2, 1.0)
            }
        }
        1 => (width, 1.0),
        _ => {
            let k = if fade == 0 {
                0.0
            } else {
                1.0 - timer as f32 / fade as f32
            };
            if fade_alpha {
                (width, k)
            } else {
                (width * k, 1.0)
            }
        }
    }
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
            let (xs, ys, sprites, born) = (p.x(), p.y(), p.sprite(), p.born_frame());
            for i in p.iter_alive() {
                // CART_FX 惰性化（引擎第二刀 §5）：`angle()` 裸切片可能是陈值，表现层
                // 走 `polar(i)`——陈值时按 vx/vy 纯计算，不越权改世界状态。
                let (_, angle) = p.polar(i);
                let (cos, sin) = bullet_basis(angle);
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
        LAYER_LASERS => {
            let p = view.lasers();
            let (oxs, oys, angles) = (p.ox(), p.oy(), p.angle());
            let (starts, ends, widths, colors) = (p.start(), p.end(), p.width(), p.sprite());
            let (warns, fades, timers, states, flags) =
                (p.warn(), p.fade(), p.timer(), p.state(), p.flags());
            for i in p.iter_alive() {
                // `sincos` 返 (sin, cos);BAM 0 指 +x(右),与激光角度同基准。
                let (sin, cos) = sincos(angles[i]);
                let (dir_x, dir_y) = (fx_f32(cos), fx_f32(sin));
                // 横截面方向 = 方向转 +90°(-y, x);实例基 [xx,yx; xy,yy] 的局部 x 轴承载
                // 显示宽度、局部 y 轴承载沿轴长度(与 shader 的 `UV.x` = 截面一致)。
                let (cross_x, cross_y) = (-dir_y, dir_x);
                let (disp_w, alpha) = laser_display(
                    states[i],
                    timers[i],
                    warns[i],
                    fades[i],
                    fx_f32(widths[i]),
                    flags[i] & LASER_FLAG_FADE_ALPHA != 0,
                );
                let len = fx_f32(ends[i] - starts[i]);
                // 原点 = 线段中点 = 射线原点 + dir × (start+end)/2。
                let mid = (fx_f32(starts[i]) + fx_f32(ends[i])) * 0.5;
                write_instance_basis(
                    out,
                    n,
                    fx_f32(oxs[i]) + dir_x * mid,
                    fx_f32(oys[i]) + dir_y * mid,
                    cross_x * disp_w,
                    dir_x * len,
                    cross_y * disp_w,
                    dir_y * len,
                    colors[i] as f32,
                    alpha,
                );
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
        g.timeline.advance(&input);
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
        let n = encode_layer(
            g.world().view(),
            g.tables,
            g.world().frame(),
            layer,
            &mut out,
        );
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
        let angles = g.world().view().bullets().angle();
        for (k, i) in g.world().view().bullets().iter_alive().enumerate() {
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

    /// 敌层退役:敌人走 `crate::puppets`,任何 ≥ LAYER_COUNT 的层号都是 P4-b no-op——
    /// cap 0、编码返 0。激光池刀(2026-09-25)后层号 3 已被 LAYER_LASERS 顶上,越界腿改打 4。
    #[test]
    fn enemies_layer_is_retired() {
        assert_eq!(LAYER_COUNT, 4);
        assert_eq!(layer_cap(LAYER_LASERS), stg_core::lasers::LaserPool::CAP);
        assert_eq!(layer_cap(LAYER_COUNT), 0);
        let g = stepped_game();
        assert_eq!(
            encode_layer(
                g.world().view(),
                g.tables,
                g.world().frame(),
                LAYER_COUNT,
                &mut []
            ),
            0
        );
        let c = crate::puppets::encode_puppets(g.world().view(), g.world().frame());
        assert_eq!((c.x[0], c.y[0]), (-96.0, -64.0), "敌人从木偶喂料读");
    }

    // ── 激光层(激光池刀 2026-09-25 Task 6;spec §7)────────────────────────

    /// 表现层显示宽度/alpha 的三态换算(允许 f32):预警 1.2 px;生效全宽;收缩按 `flags`
    /// 位 0 选变窄或淡出。三条判别式单测分别锚住早期、预警最后一帧、收缩一半。
    #[test]
    fn laser_display_warn_early_stays_thin() {
        // timer = 0 落在 ramp 之前:还是 1.2 px 细线,alpha 满。
        assert_eq!(laser_display(0, 0, 30, 16, 100.0, false), (1.2, 1.0));
        // 长预警(warn=120)只取最后 30 帧做 ramp:前 90 帧一律细线。
        assert_eq!(laser_display(0, 89, 120, 16, 100.0, false), (1.2, 1.0));
        assert_eq!(laser_display(0, 90, 120, 16, 100.0, false), (1.2, 1.0));
    }

    #[test]
    fn laser_display_warn_last_frame_near_full_width() {
        // warn=30,ramp=30 → timer=29(本态最后一帧)宽 1.2 + (100−1.2)·29/30。
        let (w, a) = laser_display(0, 29, 30, 16, 100.0, false);
        assert!(a == 1.0);
        assert!(
            w > 95.0 && w < 100.0,
            "预警最后一帧应接近全宽但未到全宽,实际 {w}"
        );
        // 单调:越靠后越宽(20 帧处应明显窄于 29 帧处)。
        let (w20, _) = laser_display(0, 20, 30, 16, 100.0, false);
        assert!(w20 < w, "预警期宽度应随时间递增");
    }

    #[test]
    fn laser_display_fade_half_width_halved() {
        // 收缩一半:变窄腿宽度减半、alpha 仍满;淡出腿宽度不动、alpha 减半。
        assert_eq!(laser_display(2, 5, 30, 10, 100.0, false), (50.0, 1.0));
        assert_eq!(laser_display(2, 5, 30, 10, 100.0, true), (100.0, 0.5));
        // fade == 0(相位 7 取消的 fade==0 激光当帧仍可见一帧)不能除零。
        assert_eq!(laser_display(2, 0, 30, 0, 100.0, false), (0.0, 1.0));
    }

    /// ECL 建一条激光(color 3、原点 (100,50)、angle 0、len 200、width 20、warn 0
    /// 即出生生效),编码后逐字段对拍实例布局:基 = 旋转(angle+90°)×缩放(宽, 长),
    /// 原点 = 线段中点,custom = [color, alpha, 0, 0]。
    #[test]
    fn lasers_layer_transform_and_custom() {
        const SRC: &str = r#"
sub main() {
    _ = laser(3, 100.0fx, 50.0fx, 0deg, 200.0fx, 20.0fx, 0, 9999, 0);
    loop { wait(60); }
}
"#;
        let mut g = crate::boot::boot(SRC, 7, 2).expect("boot");
        step_once(&mut g);
        step_once(&mut g);

        let p = g.world().view().lasers();
        let i = p.iter_alive().next().expect("两帧后激光应已出生");
        assert_eq!(
            (p.ox()[i], p.oy()[i]),
            (Fx::from_int(100), Fx::from_int(50))
        );
        assert_eq!((p.start()[i], p.end()[i]), (Fx::ZERO, Fx::from_int(200)));
        assert_eq!(p.sprite()[i], 3, "sprite 存的是颜色号");

        let (n, out) = encode(&g, LAYER_LASERS);
        assert_eq!(n, 1);
        // angle 0 → dir=(1,0)、cross=(0,1);沿轴长 200、横向宽 20。
        // 实例基 [xx,yx,0,ox, xy,yy,0,oy] = [cross·w, dir·长, 中点]。
        assert_eq!(&out[0..2], &[0.0, 200.0], "xx=0(cross 无 x)、yx=沿轴长");
        assert_eq!((out[3], out[7]), (200.0, 50.0), "原点 = start..end 中点");
        assert_eq!(&out[4..6], &[20.0, 0.0], "xy=横向宽、yy=0(dir 无 y)");
        assert_eq!(
            &out[8..12],
            &[3.0, 1.0, 0.0, 0.0],
            "custom=[color,alpha,0,0]"
        );
    }

    /// 判别腿:angle 90°(BAM 16384)时两条基轴必须整体转 90°——截面从 (0,1)·宽 变
    /// (−1,0)·宽、沿轴从 (1,0)·长 变 (0,1)·长,中点随射线方向落到 (ox, oy + mid)。
    /// 交叉方向写反(`cross = (dir_y, −dir_x)`)时此腿立刻红(angle 0 那条对不上符号)。
    #[test]
    fn lasers_layer_90deg_rotates_cross_section() {
        const SRC: &str = r#"
sub main() {
    _ = laser(3, 100.0fx, 50.0fx, 16384bam, 200.0fx, 20.0fx, 0, 9999, 0);
    loop { wait(60); }
}
"#;
        let mut g = crate::boot::boot(SRC, 7, 2).expect("boot");
        step_once(&mut g);
        step_once(&mut g);

        let (n, out) = encode(&g, LAYER_LASERS);
        assert_eq!(n, 1);
        // dir=(0,1)(屏幕 y 向下,90° = 朝下)、cross=(−1,0);warn=0 出生即生效 → alpha=1。
        assert_eq!(
            &out[0..2],
            &[-20.0, 0.0],
            "局部 x 轴(截面)= cross·显示宽 = (−20, 0)"
        );
        assert_eq!(
            (out[3], out[7]),
            (100.0, 150.0),
            "原点 = 射线原点 + dir·mid"
        );
        assert_eq!(
            &out[4..6],
            &[0.0, 200.0],
            "局部 y 轴(沿激光)= dir·长 = (0, 200)"
        );
        assert_eq!(
            &out[8..12],
            &[3.0, 1.0, 0.0, 0.0],
            "custom=[color,alpha,0,0]"
        );
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
        let mut input = stg_core::input::InputFrame::empty(g.world().frame());
        input.actions[0].buttons = stg_core::input::BTN_SHOT;
        for _ in 0..4 {
            input.frame = g.world().frame();
            g.timeline.advance(&input);
        }

        let p = g.world().view().shots();
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
