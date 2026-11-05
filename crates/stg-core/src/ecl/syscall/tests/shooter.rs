//! 发射器配置与开火。

use super::*;

/// 指定 `self_index` 的 `call`——shooter 族专用（复审 ③）：shooter 存储按**任务索引**
/// 键，而 `call` 把 `self_index` 写死 0，于是"派发臂用的是 `ctx.self_index` 还是字面量
/// 0"在测试里不可辨（等价变异）。`vm.rs` 已有 `self_index: 7` 的先例。
fn call_at(
    w: &mut World,
    ecl: &EclImage,
    task: &mut Task,
    no: u16,
    args: &[i32],
    self_index: u16,
) -> Result<(), u8> {
    call_full(w, ecl, task, no, args, &TABLES_V0, self_index)
}

// ── Shooter 配置面（syscall 600-652；shooter 刀 T2 2026-07-31）──────────────
//
// 四条测试全是**表驱动**的：14 个 setter 各写一段复制粘贴既臃肿、又保证将来加第 15 个
// 时漏掉——而漏掉的那个恰恰是最可能忘写边界检查/写串字段的那个。表在一处，加一行即入网。

/// 一个 setter 的完整描述：syscall 号 + `id` 之后的实参（正序）+ "它应当造成的改变"。
/// `mutate` 是**期望的**改变（作用在 `ShooterSlot::default()` 上），与实际派发结果整槽比对
/// ⇒ 既验"该改的改了"，又验"不该改的一个字节都没动"。
struct ShCase {
    no: u16,
    /// `id` 之后的实参，脚本声明顺序（正序压栈）。
    rest: &'static [i32],
    mutate: fn(&mut ShooterSlot),
}

/// 14 个 setter 的单一权威表（新增 setter 就加一行——四条测试全部自动覆盖它）。
fn shooter_cases() -> Vec<ShCase> {
    vec![
        // `sh_reset` 作用在默认槽上是恒等——它自己的判别腿在
        // `sh_reset_restores_every_field_to_default`（先弄脏再重置）。
        ShCase {
            no: SYS_SH_RESET,
            rest: &[],
            mutate: |_s| {},
        },
        ShCase {
            no: SYS_SH_SPRITE,
            rest: &[19], // 编译期已折叠成单个 appearance（同 fire/batch）
            mutate: |s| s.appearance = 19,
        },
        ShCase {
            no: SYS_SH_OFFSET,
            rest: &[0x0001_0000, 0x0002_0000],
            mutate: |s| {
                s.off_x = Fx::from_raw(0x0001_0000);
                s.off_y = Fx::from_raw(0x0002_0000);
            },
        },
        ShCase {
            no: SYS_SH_OFFSET_ABS,
            rest: &[0x0003_0000, 0x0004_0000],
            mutate: |s| {
                s.off_x = Fx::from_raw(0x0003_0000);
                s.off_y = Fx::from_raw(0x0004_0000);
                s.flags |= SH_ABS_OFFSET;
            },
        },
        ShCase {
            no: SYS_SH_OFFSET_RAD,
            rest: &[0x2000, 0x0005_0000],
            mutate: |s| {
                s.polar_ang = Angle(0x2000);
                s.polar_r = Fx::from_raw(0x0005_0000);
            },
        },
        ShCase {
            no: SYS_SH_DIST,
            rest: &[0x0006_0000],
            mutate: |s| s.dist = Fx::from_raw(0x0006_0000),
        },
        ShCase {
            no: SYS_SH_ANGLE,
            rest: &[0x1234, 0x0100],
            mutate: |s| {
                s.angle0 = Angle(0x1234);
                s.angle_step = Angle(0x0100);
            },
        },
        ShCase {
            no: SYS_SH_SPEED,
            rest: &[0x0002_8000, -0x0000_4000],
            mutate: |s| {
                s.speed0 = Fx::from_raw(0x0002_8000);
                s.speed_step = Fx::from_raw(-0x0000_4000);
            },
        },
        ShCase {
            no: SYS_SH_COUNT,
            rest: &[7, 3],
            mutate: |s| {
                s.n_angle = 7;
                s.n_speed = 3;
            },
        },
        ShCase {
            no: SYS_SH_AIM,
            rest: &[1],
            mutate: |s| s.flags |= SH_AIMED,
        },
        ShCase {
            no: SYS_SH_RING,
            rest: &[1],
            mutate: |s| s.flags |= SH_RING,
        },
        ShCase {
            no: SYS_SH_XFORM,
            rest: &[9, 2], // codegen 把 XformRef 降低成 (off, cnt) 两个栈值
            mutate: |s| {
                s.xform_off = 9;
                s.xform_cnt = 2;
            },
        },
        ShCase {
            no: SYS_SH_TASK,
            rest: &[3],
            mutate: |s| s.task_script = 3,
        },
        ShCase {
            no: SYS_SH_REQ,
            rest: &[42],
            mutate: |s| s.on_fire_req = 42,
        },
    ]
}

/// `id` 打头的完整实参（正序）。
fn sh_args(id: i32, rest: &[i32]) -> Vec<i32> {
    let mut v = vec![id];
    v.extend_from_slice(rest);
    v
}

/// 每个 setter 只改自己那一维，其余字段纹丝不动。
/// 这条守的是"14 个派发臂没有互相串写"——串写在只测单个 setter 的测试里看不出来。
#[test]
fn each_shooter_setter_touches_only_its_own_field() {
    for c in shooter_cases() {
        let (mut w, ecl) = fresh_with_shooters();
        let mut task = Task::default();
        assert!(
            call(&mut w, &ecl, &mut task, c.no, &sh_args(1, c.rest)).is_ok(),
            "syscall {} 应正常返回",
            c.no
        );
        let mut want = ShooterSlot::default();
        (c.mutate)(&mut want);
        assert_eq!(
            w.tasks.shooters[0][1], want,
            "syscall {} 改动的字段集不符（多改/少改/串写）",
            c.no
        );
        for k in [0usize, 2, 3] {
            assert_eq!(
                w.tasks.shooters[0][k],
                ShooterSlot::default(),
                "syscall {} 串写到了槽 {k}",
                c.no
            );
        }
        assert_eq!(task.sp, 0, "syscall {} 未把实参全部弹栈", c.no);
    }
}

/// `sh_offset` 与 `sh_offset_abs` 写的是同一对字段，只是解释方式不同——**后写的赢**。
/// 判别腿：先 abs 后 rel，`SH_ABS_OFFSET` 必须被**清掉**（不是只置不清）。
#[test]
fn offset_abs_flag_is_set_and_cleared_by_the_two_setters() {
    let (mut w, ecl) = fresh_with_shooters();
    let mut task = Task::default();
    let ten = Fx::from_int(10).raw();
    let twenty = Fx::from_int(20).raw();
    let thirty = Fx::from_int(30).raw();
    let forty = Fx::from_int(40).raw();

    assert!(
        call(
            &mut w,
            &ecl,
            &mut task,
            SYS_SH_OFFSET_ABS,
            &[0, ten, twenty]
        )
        .is_ok()
    );
    let s = w.tasks.shooters[0][0];
    assert_ne!(s.flags & SH_ABS_OFFSET, 0, "abs 设完标志应置位");
    assert_eq!((s.off_x.raw(), s.off_y.raw()), (ten, twenty));

    assert!(call(&mut w, &ecl, &mut task, SYS_SH_OFFSET, &[0, thirty, forty]).is_ok());
    let s = w.tasks.shooters[0][0];
    assert_eq!(
        s.flags & SH_ABS_OFFSET,
        0,
        "相对模式必须把 SH_ABS_OFFSET **清掉**（只置不清 = 一旦 abs 过就再也回不去）"
    );
    assert_eq!((s.off_x.raw(), s.off_y.raw()), (thirty, forty), "后写的赢");
}

/// `sh_reset` 恢复**全部**默认（不只是清几个字段）。
#[test]
fn sh_reset_restores_every_field_to_default() {
    let (mut w, ecl) = fresh_with_shooters();
    let mut task = Task::default();
    // 14 个 setter 全调一遍把槽 0 弄脏（`sh_reset` 自己那行是恒等，无所谓）。
    for c in shooter_cases() {
        assert!(call(&mut w, &ecl, &mut task, c.no, &sh_args(0, c.rest)).is_ok());
    }
    assert_ne!(
        w.tasks.shooters[0][0],
        ShooterSlot::default(),
        "前置条件：整套 setter 跑完槽必须确实脏了（否则本测试是假绿）"
    );
    assert!(call(&mut w, &ecl, &mut task, SYS_SH_RESET, &[0]).is_ok());
    assert_eq!(
        w.tasks.shooters[0][0],
        ShooterSlot::default(),
        "sh_reset 必须恢复整槽默认，不是清几个字段"
    );
}

/// P4-b：`id ≥ SHOOTERS_PER_TASK` → no-op + contract_viol + STATUS_BAD_ARGS。
/// **14 个 setter 各一腿**（表驱动）——只测其中一个的话，漏写边界检查的那几个不会红。
#[test]
fn every_shooter_setter_rejects_out_of_range_id() {
    let (mut w, ecl) = fresh_with_shooters();
    let mut task = Task::default();
    let mut expect_viol = w.body.diag.contract_viol;
    for c in shooter_cases() {
        for bad_id in [SHOOTERS_PER_TASK as i32, -1] {
            w.body.last_status = crate::world::STATUS_OK;
            assert!(
                call(&mut w, &ecl, &mut task, c.no, &sh_args(bad_id, c.rest)).is_ok(),
                "syscall {} 越界 id 是 no-op 而非 Fault",
                c.no
            );
            expect_viol += 1;
            assert_eq!(
                w.body.diag.contract_viol, expect_viol,
                "syscall {} 的 id={bad_id} 腿应记一次 contract_viol",
                c.no
            );
            assert_eq!(
                w.body.last_status,
                crate::world::STATUS_BAD_ARGS,
                "syscall {} 的 id={bad_id} 腿应置 STATUS_BAD_ARGS",
                c.no
            );
            // no-op 的判别腿：只查 contract_viol 证不了"没写脏"。
            for k in 0..SHOOTERS_PER_TASK {
                assert_eq!(
                    w.tasks.shooters[0][k],
                    ShooterSlot::default(),
                    "syscall {} 的 id={bad_id} 腿写脏了槽 {k}",
                    c.no
                );
            }
            assert_eq!(task.sp, 0, "syscall {} 越界腿也应把实参全部弹栈", c.no);
        }
    }
}

/// 复审 ②：`sh_aim`/`sh_ring` 的**清位**方向。
///
/// `shooter_cases()` 那张表每个 setter 只喂一组入参，两条旗标 setter 喂的都是 `on = 1`
/// ⇒ 把两个派发臂里的 `else { s.flags &= !BIT }` **整个删掉，表驱动那三条测试全绿**。
/// 这跟 `sh_offset` 的"只置不清"是同一个 bug 类，只是表结构塞不下"同一 setter 两种
/// 入参"，故另开一条定向腿。
#[test]
fn aim_and_ring_flags_are_cleared_by_passing_zero() {
    for (no, bit, who) in [
        (SYS_SH_AIM, SH_AIMED, "sh_aim"),
        (SYS_SH_RING, SH_RING, "sh_ring"),
    ] {
        let (mut w, ecl) = fresh_with_shooters();
        let mut task = Task::default();
        assert!(call(&mut w, &ecl, &mut task, no, &[0, 1]).is_ok());
        assert_ne!(w.tasks.shooters[0][0].flags & bit, 0, "{who}(id, 1) 应置位");
        assert!(call(&mut w, &ecl, &mut task, no, &[0, 0]).is_ok());
        assert_eq!(
            w.tasks.shooters[0][0].flags & bit,
            0,
            "{who}(id, 0) 必须把位**清掉**（只置不清 = 开了就再也关不上）"
        );
        // 判别腿：清的是自己那一位，没顺手把另一位也抹了。
        assert_eq!(
            w.tasks.shooters[0][0],
            ShooterSlot::default(),
            "{who} 一置一清之后整槽应回到默认"
        );
    }
}

/// 复审 ③：shooter 存储按**任务索引**键——派发臂必须用 `ctx.self_index`，不是字面量 0。
/// 助手把 `self_index` 写死 0 时这条是等价变异（改成 `0` 全绿）。
#[test]
fn setters_write_the_slots_of_the_calling_task_not_task_zero() {
    let (mut w, ecl) = fresh();
    w.tasks.shooters[0] = [ShooterSlot::default(); SHOOTERS_PER_TASK];
    w.tasks.shooters[7] = [ShooterSlot::default(); SHOOTERS_PER_TASK];
    let mut task = Task::default();
    assert!(call_at(&mut w, &ecl, &mut task, SYS_SH_DIST, &[2, 0x0006_0000], 7).is_ok());
    assert_eq!(
        w.tasks.shooters[7][2].dist,
        Fx::from_raw(0x0006_0000),
        "写的应是 self_index=7 的槽 2"
    );
    for k in 0..SHOOTERS_PER_TASK {
        assert_eq!(
            w.tasks.shooters[0][k],
            ShooterSlot::default(),
            "0 号任务的槽 {k} 不该被碰（派发臂若写死 0 就会红在这里）"
        );
    }
}

/// 复审 ①：`sh_sprite` 的收窄**必须保号越界性**。
///
/// 负折叠值（`sh_sprite(0, 16, c)` 里 `c` 是变量、typeck 判据跳过 ⇒ `c = -20` 折成 -4）
/// 若被钳成 **0**，就落在 `appearances[0]` 这个**在册且 valid** 的格子上 ⇒ T3 开火时
/// 再也拒不掉。`fire` 侧同样的值是 `FAULT_BAD_OP`，两条路不该有这种差别。
#[test]
fn sh_sprite_narrowing_keeps_out_of_range_values_out_of_range() {
    let (mut w, ecl) = fresh_with_shooters();
    let mut task = Task::default();
    let n_rows = TABLES_V0.appearances.len();
    assert!(
        TABLES_V0.appearances[0].valid,
        "前置条件：0 号格在册且 valid——正因为如此，把负值钳成 0 才是事故"
    );

    for bad in [-4, -1, i32::MIN] {
        assert!(call(&mut w, &ecl, &mut task, SYS_SH_SPRITE, &[0, bad]).is_ok());
        let got = w.tasks.shooters[0][0].appearance;
        assert_ne!(got, 0, "负 appearance {bad} 不得被洗成合法的 0 号格");
        assert!(
            got as usize >= n_rows,
            "负 appearance {bad} 收窄后仍须落在表外（表 {n_rows} 行，实际存了 {got}）"
        );
    }
    // 上沿：超 u16 同样保持越界。
    assert!(call(&mut w, &ecl, &mut task, SYS_SH_SPRITE, &[0, 100_000]).is_ok());
    assert!(w.tasks.shooters[0][0].appearance as usize >= n_rows);
    // 合法值照常原样存（防"一律存 u16::MAX"这种把测试骗绿的实现）。
    assert!(call(&mut w, &ecl, &mut task, SYS_SH_SPRITE, &[0, 19]).is_ok());
    assert_eq!(w.tasks.shooters[0][0].appearance, 19);
}

/// `sh_req` 的收窄（复审 ① 顺带点名的覆盖空缺）。这里钳到 **0 是对的**——`0` 在
/// `ShooterSlot::on_fire_req` 上有明确语义（"不发请求"），不像 appearance 的 0 是个
/// 在册格子；负 req id 降级成"不发"是 P4-b 的确定性安全结果。
#[test]
fn sh_req_narrowing_degrades_bad_ids_to_no_request() {
    let (mut w, ecl) = fresh_with_shooters();
    let mut task = Task::default();
    assert!(call(&mut w, &ecl, &mut task, SYS_SH_REQ, &[0, -7]).is_ok());
    assert_eq!(w.tasks.shooters[0][0].on_fire_req, 0, "负 id → 不发请求");
    assert!(call(&mut w, &ecl, &mut task, SYS_SH_REQ, &[0, 100_000]).is_ok());
    assert_eq!(
        w.tasks.shooters[0][0].on_fire_req,
        u16::MAX,
        "超 u16 上钳，不回绕（裸 `as u16` 会得 34464）"
    );
}

/// P4-b：`sh_count` 的参数是脚本给的任意 i32，先钳后存，不回绕不 panic。
#[test]
fn sh_count_clamps_both_ends() {
    let (mut w, ecl) = fresh_with_shooters();
    let mut task = Task::default();
    assert!(call(&mut w, &ecl, &mut task, SYS_SH_COUNT, &[0, -5, i32::MAX]).is_ok());
    let s = w.tasks.shooters[0][0];
    assert_eq!(s.n_angle, 0, "负数下钳到 0（裸 `as u8` 会回绕成 251）");
    assert_eq!(s.n_speed, 255, "i32::MAX 上钳到 255");
    assert!(call(&mut w, &ecl, &mut task, SYS_SH_COUNT, &[0, 256, -256]).is_ok());
    let s = w.tasks.shooters[0][0];
    assert_eq!(s.n_angle, 255, "256 上钳到 255（裸 `as u8` 会回绕成 0）");
    assert_eq!(s.n_speed, 0, "-256 下钳到 0");
}

// ── Shooter 开火面（syscall 660；shooter 刀 T3 2026-07-31）────────────────────
//
// 开火循环住 **ECL 层**（spec §10 末：P1 使然——world 不知道任务存在、没法逐颗挂
// `task_script`），于是网格逻辑有了**第二份实现**。故下面第一条等价测试是**必需**
// 而非顺带的：它一次押住网格序、坐标算法、速度递增、以及居中公式本身。

/// 配置/开火用的 `call` 包装：顺带把"实参必须全部弹栈"这条查到每一次调用上
/// （越界腿也算——shooter 族的栈效应与成功路径一致，见 `shooter_mut` 文档）。
fn sh(w: &mut World, ecl: &EclImage, task: &mut Task, no: u16, args: &[i32]) {
    assert!(
        call(w, ecl, task, no, args).is_ok(),
        "shooter syscall {no} 不应 Fault"
    );
    assert_eq!(task.sp, 0, "shooter syscall {no} 未把实参全部弹栈");
}

/// 灌池用的哑弹模板。
fn filler_bullet() -> BulletInit {
    BulletInit {
        x: Fx::ZERO,
        y: Fx::ZERO,
        vx: Fx::ZERO,
        vy: Fx::ZERO,
        speed: Fx::ZERO,
        angle: Angle::ZERO,
        ang_vel: 0,
        accel: Fx::ZERO,
        ax: Fx::ZERO,
        ay: Fx::ZERO,
        sprite: 0,
        radius: Fx::from_int(2),
        delay: 0,
        life: 0xFFFF,
        flags: 0,
        grazed_by: 0,
        transform_head: crate::xform::XFORM_NONE,
        xform_wait: 0,
        xform_next: 0,
        born_frame: 0,
    }
}

/// shooter 的 fan 与手写 `batch` 等价——**带居中补偿**。
/// 一次押住网格序、坐标算法、速度递增、**以及居中公式本身**。
///
/// 为什么必需：开火循环住 ECL 层（P1 使然，见 spec §10 末），网格逻辑有了第二份实现。
/// 补偿写错、或实现忘了居中，这条都会红。
#[test]
fn shooter_fan_matches_batch_with_centering_compensation() {
    const N_ANGLE: i32 = 5;
    const N_SPEED: i32 = 3;
    const BASE: i32 = 0x1234;
    const STEP: i32 = 0x0400;
    let x = Fx::from_int(30).raw();
    let y = Fx::from_int(-40).raw();
    let speed0 = Fx::from_int(2).raw();
    let speed_step = 0x0000_8000; // 0.5

    // 世界 A：shooter。owner 是 STAGE ⇒ `self_pos` 恒 (0,0)，故 `sh_offset` 就是原点。
    // 其余全默认：无 aim / ring / dist / polar / xform / 挂弹任务。
    let (mut wa, ecl) = fresh_with_shooters();
    let mut ta = Task::default();
    sh(&mut wa, &ecl, &mut ta, SYS_SH_SPRITE, &[0, ROW_C]);
    sh(&mut wa, &ecl, &mut ta, SYS_SH_OFFSET, &[0, x, y]);
    sh(&mut wa, &ecl, &mut ta, SYS_SH_COUNT, &[0, N_ANGLE, N_SPEED]);
    sh(&mut wa, &ecl, &mut ta, SYS_SH_ANGLE, &[0, BASE, STEP]);
    sh(
        &mut wa,
        &ecl,
        &mut ta,
        SYS_SH_SPEED,
        &[0, speed0, speed_step],
    );
    sh(&mut wa, &ecl, &mut ta, SYS_SH_FIRE, &[0]);

    // 世界 B：手写 `batch`，基准角**自带居中补偿**。
    let compensated = BASE - (N_ANGLE - 1) * STEP / 2;
    assert_ne!(
        compensated, BASE,
        "前置条件：这组参数下补偿必须非恒等，否则'两边都忘了居中'也能绿"
    );
    let (mut wb, _) = fresh();
    let mut tb = Task::default();
    let args = [
        ROW_C,
        x,
        y,
        N_ANGLE,
        compensated,
        STEP,
        N_SPEED,
        speed0,
        speed_step,
    ];
    assert!(call(&mut wb, &ecl, &mut tb, SYS_CREATE_BULLETS_BATCH, &args).is_ok());
    assert_eq!(tb.stack[0], N_ANGLE * N_SPEED, "batch 侧应发满 5×3");

    let ia: Vec<usize> = wa.body.bullets.iter_alive().collect();
    let ib: Vec<usize> = wb.body.bullets.iter_alive().collect();
    assert_eq!(
        ia.len(),
        (N_ANGLE * N_SPEED) as usize,
        "shooter 侧应发满 5×3"
    );
    assert_eq!(ia.len(), ib.len());
    for (k, (&a, &b)) in ia.iter().zip(ib.iter()).enumerate() {
        assert_eq!(wa.body.bullets.x[a], wb.body.bullets.x[b], "第 {k} 颗 x");
        assert_eq!(wa.body.bullets.y[a], wb.body.bullets.y[b], "第 {k} 颗 y");
        assert_eq!(wa.body.bullets.vx[a], wb.body.bullets.vx[b], "第 {k} 颗 vx");
        assert_eq!(wa.body.bullets.vy[a], wb.body.bullets.vy[b], "第 {k} 颗 vy");
        assert_eq!(
            wa.body.bullets.speed[a], wb.body.bullets.speed[b],
            "第 {k} 颗 speed"
        );
        assert_eq!(
            wa.body.bullets.angle[a], wb.body.bullets.angle[b],
            "第 {k} 颗 angle"
        );
        assert_eq!(
            wa.body.bullets.sprite[a], wb.body.bullets.sprite[b],
            "第 {k} 颗 sprite"
        );
        assert_eq!(
            wa.body.bullets.radius[a], wb.body.bullets.radius[b],
            "第 {k} 颗 radius"
        );
    }
    // 判别腿的判别腿：网格里必须真的出现过多个不同角、多个不同速，否则上面逐一比
    // 是在比一堆相同的弹（`n_angle`/`n_speed` 被实现忽略掉也能绿）。
    let mut angles: Vec<u16> = ia.iter().map(|&i| wa.body.bullets.angle[i].raw()).collect();
    let mut speeds: Vec<i32> = ia.iter().map(|&i| wa.body.bullets.speed[i].raw()).collect();
    angles.sort_unstable();
    angles.dedup();
    speeds.sort_unstable();
    speeds.dedup();
    assert_eq!(angles.len(), N_ANGLE as usize, "网格里应出现 5 个不同角");
    assert_eq!(speeds.len(), N_SPEED as usize, "网格里应出现 3 个不同速");
}

/// 居中公式的**偶数路 × 负步长**格（复审 ①/③）。
///
/// 上一版这两格全无覆盖：等价测试只跑 `N_ANGLE = 5`（奇数），唯一用偶数路的
/// `dist_pushes_each_bullet_along_its_own_angle` 只比 dist 前后的**位移差**、两侧
/// `fan_center` 相同 ⇒ 对居中值完全免疫。于是下面这个 bug 整整逃逸了一轮：
///
/// `ShooterSlot.angle_step` 存的是 `Angle`（底层 **u16**），`sh_angle(0, 0deg, -6deg)`
/// 这种常规写法存进去的是 `65536 − 1092`。`i·step`/`j·step` 在 mod 65536 下不受影响，
/// **但 `/2` 不与 mod 65536 交换**：
///   `((n−1)(s + 65536))/2 = ((n−1)s)/2 + (n−1)·32768`
/// `n` 奇数 ⇒ `(n−1)` 偶 ⇒ 多出项是 65536 的整数倍、无害；
/// **`n` 偶数 ⇒ 多出 32768 BAM = 半圈**，整把扇形被搬到 `base` 的正对面
/// （扇形形状还对，相邻仍差 `step`——所以"形状对不对"式的测试也看不见它）。
///
/// 修法是读侧符号扩展（`as i16 as i32`），回到本仓家规：`create_bullets_batch` 的形参
/// 就是 `angle_step: i16`、`Angle::add_delta` 也收 i16——**有符号角度增量是既定口径**，
/// `ShooterSlot` 存 `Angle` 是唯一破口。别改结构（槽宽已被 44 B 测试冻结）。
#[test]
fn fan_centering_handles_even_ways_with_negative_step() {
    const N_ANGLE: i32 = 4;
    const STEP: i32 = -0x400; // 脚本侧的 `-Xdeg`：经 `bam()` 存成 0xFC00
    const BASE: i32 = 0;

    let (mut wa, ecl) = fresh_with_shooters();
    let mut ta = Task::default();
    sh(&mut wa, &ecl, &mut ta, SYS_SH_SPRITE, &[0, ROW_C]);
    sh(&mut wa, &ecl, &mut ta, SYS_SH_COUNT, &[0, N_ANGLE, 1]);
    sh(&mut wa, &ecl, &mut ta, SYS_SH_ANGLE, &[0, BASE, STEP]);
    sh(&mut wa, &ecl, &mut ta, SYS_SH_FIRE, &[0]);
    let got: Vec<u16> = wa
        .body
        .bullets
        .iter_alive()
        .map(|i| wa.body.bullets.angle[i].raw())
        .collect();

    // 契约意图：base ± 512、base ± 1536（4 路以 base 为中心、步长 −1024）。
    // 有 bug 的实现给出 34304/33280/32256/31232——形状对、整扇偏 180°。
    assert_eq!(
        got,
        vec![1536, 512, 65024, 64000],
        "偶数路 + 负步长的居中值错了（差 32768 = 半圈就是 u16 零扩展那个 bug）"
    );

    // 同一格也走一遍与 `batch` 的等价（`batch` 的 `angle_step` 形参本就是 i16，
    // 故它是这条口径的现成参照物）。
    let compensated = BASE - (N_ANGLE - 1) * STEP / 2;
    let (mut wb, _) = fresh();
    let mut tb = Task::default();
    let args = [ROW_C, 0, 0, N_ANGLE, compensated, STEP, 1, 0, 0];
    assert!(call(&mut wb, &ecl, &mut tb, SYS_CREATE_BULLETS_BATCH, &args).is_ok());
    let want: Vec<u16> = wb
        .body
        .bullets
        .iter_alive()
        .map(|i| wb.body.bullets.angle[i].raw())
        .collect();
    assert_eq!(
        got, want,
        "偶数路 + 负步长下 shooter 的 fan 应仍与 batch 等价"
    );
}

/// 复审 ②：aim 的基点是**原点**（含全部三种偏移），不是 owner 位置。
///
/// 为什么另开一条：`aim_resolves_at_fire_time_not_at_set_time` 的 owner 是 STAGE、
/// 无任何偏移 ⇒ `origin == self_pos == (0,0)`，两个基点**重合**——把实现里的
/// `origin_x/origin_y` 换成 `self_pos(task, ctx)`（也就是"顺手复用
/// `sys_aim_player_angle` 口径"这个最自然的错法）那条测试照样绿。
/// 同 `CLAUDE.md` 点名的"圆心重合式测试对半径映射是瞎的"。
#[test]
fn aim_is_measured_from_the_fire_origin_not_from_the_owner() {
    let (mut w, ecl) = fresh_with_shooters();
    let eh = crate::world::test_support::spawn_enemy(&mut w, 100, -60, 5);
    let mut t = Task {
        owner_kind: OWNER_ENEMY,
        owner_index: eh.index,
        owner_gen: eh.generation,
        ..Task::default()
    };
    sh(&mut w, &ecl, &mut t, SYS_SH_SPRITE, &[0, ROW_A]);
    // 绝对偏移把原点挪到 (300, 0)——**远离** owner 的 (100, -60)。
    sh(
        &mut w,
        &ecl,
        &mut t,
        SYS_SH_OFFSET_ABS,
        &[0, Fx::from_int(300).raw(), 0],
    );
    sh(&mut w, &ecl, &mut t, SYS_SH_AIM, &[0, 1]);
    sh(&mut w, &ecl, &mut t, SYS_SH_FIRE, &[0]);

    let i = w.body.bullets.iter_alive().next().expect("应发一颗");
    let got = w.body.bullets.angle[i];
    let (px, py) = (w.body.players[0].x, w.body.players[0].y);
    let from_origin = crate::math::cordic::atan2(py - Fx::ZERO, px - Fx::from_int(300));
    let from_owner = crate::math::cordic::atan2(py - Fx::from_int(-60), px - Fx::from_int(100));
    // 两条断言都要：只写前一条的话，owner 位置恰好也对得上的取值会漏。
    assert_eq!(got, from_origin, "aim 必须从**开火原点**量");
    assert_ne!(
        got, from_owner,
        "aim 从 owner 位置量了（顺手复用 sys_aim_player_angle 的口径就是这个结果）"
    );
}

/// 判别式（F8）：`sh_aim` 瞄的也是**最近的可瞄自机**，与 `aim_player()` 同一条规矩。
/// owner 是 STAGE、无偏移 ⇒ 出弹口在原点；P0 尸体在左上、P1 活着在右下。
#[test]
fn shooter_aim_skips_unaimable_player() {
    let (mut w, ecl) = fresh_with_shooters();
    w.body.players[0].x = Fx::from_int(-200);
    w.body.players[0].y = Fx::from_int(-150);
    w.body.players[0].life_state = crate::player::LIFE_GAMEOVER;
    w.body.players[1] = crate::player::PlayerState::spawn(0, &TABLES_V0.characters[0]);
    w.body.players[1].x = Fx::from_int(200);
    w.body.players[1].y = Fx::from_int(150);
    let mut t = Task::default();
    sh(&mut w, &ecl, &mut t, SYS_SH_SPRITE, &[0, ROW_A]);
    sh(&mut w, &ecl, &mut t, SYS_SH_AIM, &[0, 1]);
    sh(&mut w, &ecl, &mut t, SYS_SH_FIRE, &[0]);

    let i = w.body.bullets.iter_alive().next().expect("应发一颗");
    let to_p1 = crate::math::cordic::atan2(Fx::from_int(150), Fx::from_int(200));
    let to_p0 = crate::math::cordic::atan2(Fx::from_int(-150), Fx::from_int(-200));
    assert_eq!(w.body.bullets.angle[i], to_p1, "发射器应瞄唯一可瞄的 P1");
    assert_ne!(
        w.body.bullets.angle[i], to_p0,
        "发射器瞄了已 GAMEOVER 的 P0 尸体坐标"
    );
}

/// 同上的 fallback 腿：发射器这条路也**必须产出角度**，无可瞄自机时回退到 P0 最后坐标
/// （与 `sys_aim_player_angle_falls_back_to_p0_when_none_aimable` 成对，两条路同一口径）。
#[test]
fn shooter_aim_falls_back_to_p0_when_none_aimable() {
    let (mut w, ecl) = fresh_with_shooters();
    w.body.players[0].x = Fx::from_int(-200);
    w.body.players[0].y = Fx::from_int(-150);
    w.body.players[0].life_state = crate::player::LIFE_GAMEOVER; // players[1] 本就 ABSENT
    let mut t = Task::default();
    sh(&mut w, &ecl, &mut t, SYS_SH_SPRITE, &[0, ROW_A]);
    sh(&mut w, &ecl, &mut t, SYS_SH_AIM, &[0, 1]);
    sh(&mut w, &ecl, &mut t, SYS_SH_FIRE, &[0]);

    let i = w.body.bullets.iter_alive().next().expect("应发一颗");
    let to_corpse = crate::math::cordic::atan2(Fx::from_int(-150), Fx::from_int(-200));
    assert_eq!(
        w.body.bullets.angle[i], to_corpse,
        "无可瞄自机时发射器应回退到 P0 的最后坐标"
    );
}

/// `dist` 是**逐颗沿各自角度**位移，不是整环朝同一方向平移。
/// 判别腿：取一个 n_angle=4、angle_step=90deg 的十字环，四颗弹的位移方向必须两两不同；
/// 错误实现（整体平移）会让四颗的 (x,y) 相对无 dist 时的偏移量**完全相同**。
#[test]
fn dist_pushes_each_bullet_along_its_own_angle() {
    let fire = |dist: i32| -> Vec<(i32, i32)> {
        let (mut w, ecl) = fresh_with_shooters();
        let mut t = Task::default();
        sh(&mut w, &ecl, &mut t, SYS_SH_SPRITE, &[0, ROW_A]);
        sh(&mut w, &ecl, &mut t, SYS_SH_COUNT, &[0, 4, 1]);
        sh(&mut w, &ecl, &mut t, SYS_SH_ANGLE, &[0, 0, 16384]); // 十字：90° 一颗
        sh(&mut w, &ecl, &mut t, SYS_SH_DIST, &[0, dist]);
        sh(&mut w, &ecl, &mut t, SYS_SH_FIRE, &[0]);
        w.body
            .bullets
            .iter_alive()
            .map(|i| (w.body.bullets.x[i].raw(), w.body.bullets.y[i].raw()))
            .collect()
    };
    let base = fire(0);
    let pushed = fire(Fx::from_int(20).raw());
    assert_eq!(base.len(), 4);
    assert_eq!(pushed.len(), 4);
    let off: Vec<(i32, i32)> = base
        .iter()
        .zip(&pushed)
        .map(|(b, p)| (p.0 - b.0, p.1 - b.1))
        .collect();
    for (k, o) in off.iter().enumerate() {
        assert_ne!(*o, (0, 0), "第 {k} 颗根本没被 dist 推开");
    }
    for i in 0..4 {
        for j in (i + 1)..4 {
            assert_ne!(
                off[i], off[j],
                "第 {i}/{j} 颗的 dist 位移量相同 ⇒ 实现把整环朝同一方向平移了"
            );
        }
    }
}

/// `sh_offset` 与 `sh_offset_rad` 同时设 → 两者**相加**（ZUN 626 明写 stacks）。
/// 判别腿：只设其一 / 只设另一 / 两个都设，第三种的原点必须等于前两种偏移量之和。
#[test]
fn rect_and_polar_offsets_stack_rather_than_override() {
    let x = Fx::from_int(30).raw();
    let y = Fx::from_int(-10).raw();
    let ang = 8192; // 45°
    let r = Fx::from_int(25).raw();
    // owner 是 STAGE ⇒ 基准原点 (0,0)，故"原点"就是偏移量本身。
    let fire = |rect: bool, polar: bool| -> (i32, i32) {
        let (mut w, ecl) = fresh_with_shooters();
        let mut t = Task::default();
        sh(&mut w, &ecl, &mut t, SYS_SH_SPRITE, &[0, ROW_A]);
        if rect {
            sh(&mut w, &ecl, &mut t, SYS_SH_OFFSET, &[0, x, y]);
        }
        if polar {
            sh(&mut w, &ecl, &mut t, SYS_SH_OFFSET_RAD, &[0, ang, r]);
        }
        sh(&mut w, &ecl, &mut t, SYS_SH_FIRE, &[0]);
        let i = w
            .body
            .bullets
            .iter_alive()
            .next()
            .expect("默认 1×1 应恰发一颗");
        (w.body.bullets.x[i].raw(), w.body.bullets.y[i].raw())
    };
    let only_rect = fire(true, false);
    let only_polar = fire(false, true);
    let both = fire(true, true);
    assert_ne!(
        only_rect, only_polar,
        "前置条件：两种偏移必须给出不同原点，否则'覆盖'与'相加'不可辨"
    );
    assert_eq!(
        both,
        (only_rect.0 + only_polar.0, only_rect.1 + only_polar.1),
        "两种偏移必须**相加**（后设覆盖先设就会红）"
    );
}

/// `aim` 在**开火那一刻**解析，不是 `sh_aim` 时。
/// 判别腿：sh_aim(0,1) → 移动自机 → sh_fire，基准角必须跟着新位置变。
/// 错误实现（设的时候就把角算死）会让两次开火的角相同。
#[test]
fn aim_resolves_at_fire_time_not_at_set_time() {
    let (mut w, ecl) = fresh_with_shooters();
    let mut t = Task::default();
    sh(&mut w, &ecl, &mut t, SYS_SH_SPRITE, &[0, ROW_A]);
    sh(&mut w, &ecl, &mut t, SYS_SH_AIM, &[0, 1]);

    // 第一发：自机在出场点。原点是 (0,0)（STAGE owner、无偏移），故基准角 = atan2(自机)。
    let p0 = (w.body.players[0].x, w.body.players[0].y);
    sh(&mut w, &ecl, &mut t, SYS_SH_FIRE, &[0]);
    // 挪自机——shooter 的参数一个字节都没改。
    w.body.players[0].x = Fx::from_int(200);
    w.body.players[0].y = Fx::from_int(-150);
    let p1 = (w.body.players[0].x, w.body.players[0].y);
    sh(&mut w, &ecl, &mut t, SYS_SH_FIRE, &[0]);

    let idx: Vec<usize> = w.body.bullets.iter_alive().collect();
    assert_eq!(idx.len(), 2);
    let a0 = w.body.bullets.angle[idx[0]];
    let a1 = w.body.bullets.angle[idx[1]];
    assert_eq!(
        a0,
        crate::math::cordic::atan2(p0.1, p0.0),
        "第一发朝旧自机位"
    );
    assert_eq!(
        a1,
        crate::math::cordic::atan2(p1.1, p1.0),
        "第二发朝新自机位——aim 必须在开火那一刻解析"
    );
    assert_ne!(a0, a1, "两发同角 ⇒ 实现在 sh_aim 那一刻就把角算死了");
}

/// ring 精确闭合：n_angle=28 时 28 颗**铺满整 65536**、收尾无缝。
/// 防照抄 demo 的 `65536/n` 预乘写法（2340×28 = 65520，收尾留 16 BAM 的缝）。
#[test]
fn ring_distributes_exactly_around_the_full_circle() {
    const N: usize = 28;
    let (mut w, ecl) = fresh_with_shooters();
    let mut t = Task::default();
    sh(&mut w, &ecl, &mut t, SYS_SH_SPRITE, &[0, ROW_A]);
    sh(&mut w, &ecl, &mut t, SYS_SH_RING, &[0, 1]);
    sh(&mut w, &ecl, &mut t, SYS_SH_COUNT, &[0, N as i32, 1]);
    sh(&mut w, &ecl, &mut t, SYS_SH_FIRE, &[0]);
    let a: Vec<u16> = w
        .body
        .bullets
        .iter_alive()
        .map(|i| w.body.bullets.angle[i].raw())
        .collect();
    assert_eq!(a.len(), N);

    // 主判据一：逐颗算 (i×65536)/N，把余数均摊——**不是** i×(65536/N)。
    for (i, &got) in a.iter().enumerate() {
        let want = ((i as i32 * 65536) / N as i32) as u16;
        assert_eq!(got, want, "第 {i} 颗角不是均摊余数的 (i×65536)/N");
    }
    // 主判据二：把**闭合那一段**也算进来，全部相邻角差的极差 ≤ 1。
    // 预乘写法前 27 段都是 2340、闭合段却是 2356 ⇒ 极差 16，红在这里。
    let gaps: Vec<u32> = (0..N)
        .map(|i| a[(i + 1) % N].wrapping_sub(a[i]) as u32)
        .collect();
    let lo = *gaps.iter().min().unwrap();
    let hi = *gaps.iter().max().unwrap();
    assert!(hi - lo <= 1, "相邻角差极差 {} ⇒ 收尾留了缝", hi - lo);
    // 顺带：一整圈。**这条单独是弱判据**——任何绕行一圈的排布都满足它（预乘写法
    // 也满足：27×2340 + 2356 = 65536），故主力是上面两条。
    assert_eq!(gaps.iter().sum::<u32>(), 65536);
}

/// ring 模式下 `angle_step` 是**逐层**偏移（而非逐弹）——两层的同序号弹角差 == angle_step。
#[test]
fn ring_angle_step_offsets_layers_not_bullets() {
    const STEP: i32 = 0x0300;
    let (mut w, ecl) = fresh_with_shooters();
    let mut t = Task::default();
    sh(&mut w, &ecl, &mut t, SYS_SH_SPRITE, &[0, ROW_A]);
    sh(&mut w, &ecl, &mut t, SYS_SH_RING, &[0, 1]);
    sh(&mut w, &ecl, &mut t, SYS_SH_COUNT, &[0, 4, 2]);
    sh(&mut w, &ecl, &mut t, SYS_SH_ANGLE, &[0, 0, STEP]);
    sh(&mut w, &ecl, &mut t, SYS_SH_FIRE, &[0]);
    let a: Vec<u16> = w
        .body
        .bullets
        .iter_alive()
        .map(|i| w.body.bullets.angle[i].raw())
        .collect();
    assert_eq!(a.len(), 8, "序 = 角度外层、速度内层 ⇒ 索引 2i+j");
    for i in 0..4 {
        assert_eq!(
            a[i * 2 + 1].wrapping_sub(a[i * 2]),
            STEP as u16,
            "第 {i} 组两层的角差应恰是 angle_step（逐层，不是逐弹）"
        );
    }
    assert_eq!(
        a[2].wrapping_sub(a[0]),
        (65536 / 4) as u16,
        "同层相邻两颗的角差是均分整周，不是 angle_step"
    );
}

/// xform 延迟读安全：`sh_xform` 之后 **wait 跨帧** 再 `sh_fire`，xform 仍生效。
///
/// xform 数据住**本任务的 `locals`**（`task.locals[xform_off..]`），由编译器在 sub 入口
/// 一次性 staging，`slots` 的调用图着色保证区间不被复用——所以存 `(off, cnt)`、开火时
/// 才读是安全的。这条用真字节码跑完整 `run_tasks` 循环把它钉死。
#[test]
fn xform_survives_a_frame_boundary_between_set_and_fire() {
    // 槽 0：wait=3, op=OP_SET_SPEED(10), args=[Fx::from_int(2).raw(), 0]
    let word0 = ((3u32 << 16) | ((crate::xform::OP_SET_SPEED as u32) << 8)) as i32;
    let arg0 = Fx::from_int(2).raw();
    let code = vec![
        crate::ecl::ops::OP_END as u32,   //  0  sub0 Root（占位）
        crate::ecl::ops::OP_PUSHI as u32, //  1  sub1 Async 入口：locals[0..3] = xform 槽
        word0 as u32,                     //  2
        crate::ecl::ops::OP_POPL as u32,  //  3
        0,                                //  4
        crate::ecl::ops::OP_PUSHI as u32, //  5
        arg0 as u32,                      //  6
        crate::ecl::ops::OP_POPL as u32,  //  7
        1,                                //  8
        crate::ecl::ops::OP_PUSHI as u32, //  9
        0,                                // 10
        crate::ecl::ops::OP_POPL as u32,  // 11
        2,                                // 12
        crate::ecl::ops::OP_PUSHI as u32, // 13  sh_sprite(0, ROW_A)
        0,                                // 14
        crate::ecl::ops::OP_PUSHI as u32, // 15
        ROW_A as u32,                     // 16
        crate::ecl::ops::OP_SYS as u32,   // 17
        SYS_SH_SPRITE as u32,             // 18
        crate::ecl::ops::OP_PUSHI as u32, // 19  sh_xform(0, off=0, cnt=1)
        0,                                // 20
        crate::ecl::ops::OP_PUSHI as u32, // 21
        0,                                // 22
        crate::ecl::ops::OP_PUSHI as u32, // 23
        1,                                // 24
        crate::ecl::ops::OP_SYS as u32,   // 25
        SYS_SH_XFORM as u32,              // 26
        crate::ecl::ops::OP_PUSHI as u32, // 27  wait(2) ← **跨帧**
        2,                                // 28  （2026-08-01 语义修正：旧 `wait(1)` 就是
        //                                        "隔一帧跑"，新语义下等价写法是 wait(2)。
        //                                        这里要的是"设 xform 与开火之间夹一个
        //                                        任务不执行的空转帧"，故跟着改数不改结构。）
        crate::ecl::ops::OP_WAIT as u32,  // 29
        crate::ecl::ops::OP_PUSHI as u32, // 30  sh_fire(0)
        0,                                // 31
        crate::ecl::ops::OP_SYS as u32,   // 32
        SYS_SH_FIRE as u32,               // 33
        crate::ecl::ops::OP_PUSHI as u32, // 34  wait(999)：停在这里别自然结束
        999,                              // 35
        crate::ecl::ops::OP_WAIT as u32,  // 36
        crate::ecl::ops::OP_END as u32,   // 37
    ];
    let ecl = test_image(
        code,
        vec![
            SubInit::new(0, SubKind::Root, vec![]),
            SubInit::new(1, SubKind::Async, vec![]),
        ],
        vec![EntryInit::new("xf_task", 1)],
        Some(0),
    );
    let mut w = World::new(1);
    let idx = w
        .spawn_sub_internal(&ecl, ecl.sub_id(1).unwrap(), &[], (OWNER_STAGE, 0, 0))
        .expect("应能派一个任务");

    // 帧 1：跑到 wait(2) 让出（此时 sh_xform 已设、还没开火）。
    w.body.frame = 1;
    crate::ecl::vm::run_tasks(&mut w.tasks, &mut w.body, &ecl, &TABLES_V0);
    assert_eq!(w.body.bullets.iter_alive().count(), 0, "跨帧前还没开火");
    assert_ne!(
        w.tasks.shooters[idx as usize][0].xform_cnt, 0,
        "前置条件：sh_xform 应已写进槽"
    );
    // 帧 2：wait 递减，本帧不执行。
    w.body.frame = 2;
    crate::ecl::vm::run_tasks(&mut w.tasks, &mut w.body, &ecl, &TABLES_V0);
    assert_eq!(w.body.bullets.iter_alive().count(), 0);
    // 帧 3：开火——xform 必须仍生效。
    w.body.frame = 3;
    crate::ecl::vm::run_tasks(&mut w.tasks, &mut w.body, &ecl, &TABLES_V0);

    let b = w
        .body
        .bullets
        .iter_alive()
        .next()
        .expect("跨帧后应发出一颗弹");
    let seg = w.body.bullets.transform_head[b];
    assert_ne!(
        seg,
        crate::xform::XFORM_NONE,
        "跨帧后 xform 丢了（延迟读没读到本任务 locals）"
    );
    assert_eq!(
        w.body.xforms.seg_slots(seg)[0],
        XformSlot {
            wait: 3,
            op: crate::xform::OP_SET_SPEED,
            _pad: 0,
            args: [arg0, 0],
        },
        "跨帧后段内容应与设 xform 时的 locals 逐位相同"
    );
}

/// 挂弹任务：每颗弹各派一个任务，owner = (BULLET, 该弹 index/gen)。
#[test]
fn sh_task_spawns_one_task_per_bullet() {
    let ecl = async_pattern_image(); // sub raw=1 是 0 参 Async
    let mut w = World::new(1);
    w.tasks.shooters[0] = [ShooterSlot::default(); SHOOTERS_PER_TASK];
    let mut t = Task::default();
    sh(&mut w, &ecl, &mut t, SYS_SH_SPRITE, &[0, ROW_A]);
    sh(&mut w, &ecl, &mut t, SYS_SH_COUNT, &[0, 3, 1]);
    sh(&mut w, &ecl, &mut t, SYS_SH_TASK, &[0, 1]);
    sh(&mut w, &ecl, &mut t, SYS_SH_FIRE, &[0]);

    let bullets: Vec<usize> = w.body.bullets.iter_alive().collect();
    assert_eq!(bullets.len(), 3);
    let spawned: Vec<usize> = (0..crate::ecl::task::TASK_CAP)
        .filter(|&i| w.tasks.is_alive(i))
        .collect();
    assert_eq!(spawned.len(), 3, "每颗弹各派一个任务");
    for (k, (&ti, &bi)) in spawned.iter().zip(&bullets).enumerate() {
        assert_eq!(w.tasks.slots[ti].owner_kind, OWNER_BULLET, "第 {k} 个");
        assert_eq!(w.tasks.slots[ti].owner_index, bi as u16, "第 {k} 个");
        assert_eq!(
            w.tasks.slots[ti].owner_gen, w.body.bullets.generation[bi],
            "第 {k} 个"
        );
    }
}

/// P4-a：任务池满 → 弹**保留**、`pool_full[POOL_TASK]` 逐颗计数、不 Fault（同 `fire` 口径）。
#[test]
fn sh_task_degrades_when_task_pool_is_full() {
    let ecl = async_pattern_image();
    let mut w = World::new(1);
    // **先灌满任务池、再配 shooter**：`TaskPool::spawn` 的"复用槽写满"纪律会把
    // 每个新槽的 shooter 抹成默认值（含 0 号槽），先配后灌会被抹掉。
    while w
        .tasks
        .spawn(ecl.sub_id(1).unwrap(), 0, (OWNER_STAGE, 0, 0), 0, 0)
        .is_some()
    {}
    let mut t = Task::default();
    sh(&mut w, &ecl, &mut t, SYS_SH_SPRITE, &[0, ROW_A]);
    sh(&mut w, &ecl, &mut t, SYS_SH_COUNT, &[0, 3, 1]);
    sh(&mut w, &ecl, &mut t, SYS_SH_TASK, &[0, 1]);

    let before = w.body.diag.pool_full[crate::world::POOL_TASK];
    sh(&mut w, &ecl, &mut t, SYS_SH_FIRE, &[0]);
    assert_eq!(
        w.body.bullets.iter_alive().count(),
        3,
        "任务池满不影响弹——弹保留"
    );
    assert_eq!(
        w.body.diag.pool_full[crate::world::POOL_TASK],
        before + 3,
        "逐颗计一次 pool_full[POOL_TASK]"
    );
}

/// `on_fire_req`：发一条请求，`args[3]` 是**实际**创建数而非请求数。
/// 判别腿：把弹池灌到只剩 2 格再发 5 颗的环，args[3] 必须是 2。
#[test]
fn on_fire_req_reports_actual_count_not_requested() {
    let (mut w, ecl) = fresh_with_shooters();
    for _ in 0..(crate::bullets::BulletPool::CAP - 2) {
        w.body.create_bullet(filler_bullet());
    }
    let ox = Fx::from_int(12).raw();
    let oy = Fx::from_int(-34).raw();
    let mut t = Task::default();
    sh(&mut w, &ecl, &mut t, SYS_SH_SPRITE, &[0, ROW_C]);
    sh(&mut w, &ecl, &mut t, SYS_SH_OFFSET, &[0, ox, oy]);
    sh(&mut w, &ecl, &mut t, SYS_SH_RING, &[0, 1]);
    sh(&mut w, &ecl, &mut t, SYS_SH_COUNT, &[0, 5, 1]);
    sh(&mut w, &ecl, &mut t, SYS_SH_REQ, &[0, 77]);
    sh(&mut w, &ecl, &mut t, SYS_SH_FIRE, &[0]);

    assert_eq!(
        w.body.bullets.iter_alive().count(),
        crate::bullets::BulletPool::CAP,
        "只剩 2 格 ⇒ 请求 5 颗只发得出 2 颗"
    );
    let r = *w
        .body
        .take_requests()
        .iter()
        .find(|r| r.id == 77)
        .expect("on_fire_req ≠ 0 应发一条请求");
    assert_eq!(r.args[0], ox, "args[0] = 原点 x");
    assert_eq!(r.args[1], oy, "args[1] = 原点 y");
    assert_eq!(r.args[2], ROW_C, "args[2] = appearance");
    assert_eq!(r.args[3], 2, "args[3] 必须是**实际**创建数（不是请求的 5）");
}

/// `on_fire_req == 0` = 不发（默认值的语义，别发一条 id=0 的垃圾请求）。
#[test]
fn on_fire_req_zero_emits_nothing() {
    let (mut w, ecl) = fresh_with_shooters();
    let mut t = Task::default();
    sh(&mut w, &ecl, &mut t, SYS_SH_SPRITE, &[0, ROW_A]);
    sh(&mut w, &ecl, &mut t, SYS_SH_FIRE, &[0]);
    assert_eq!(w.body.bullets.iter_alive().count(), 1);
    assert!(
        w.body.take_requests().is_empty(),
        "on_fire_req=0 不该发请求"
    );
}

/// P4-b：n_angle 或 n_speed 为 0、或乘积超弹池 CAP → 不发 + contract_viol
/// （对齐 `create_bullets_batch` 的既有口径）。
#[test]
fn sh_fire_rejects_degenerate_grid() {
    const {
        assert!(
            255 * 255 > crate::bullets::BulletPool::CAP,
            "前置条件：255×255 必须真的超弹池 CAP"
        )
    };
    for (n_angle, n_speed, why) in [
        (0, 1, "n_angle=0"),
        (1, 0, "n_speed=0"),
        (255, 255, "超CAP"),
    ] {
        let (mut w, ecl) = fresh_with_shooters();
        let mut t = Task::default();
        sh(&mut w, &ecl, &mut t, SYS_SH_SPRITE, &[0, ROW_A]);
        sh(&mut w, &ecl, &mut t, SYS_SH_COUNT, &[0, n_angle, n_speed]);
        let viol = w.body.diag.contract_viol;
        w.body.last_status = crate::world::STATUS_OK;
        sh(&mut w, &ecl, &mut t, SYS_SH_FIRE, &[0]);
        assert_eq!(
            w.body.bullets.iter_alive().count(),
            0,
            "{why}：一颗都不该发"
        );
        assert_eq!(
            w.body.diag.contract_viol,
            viol + 1,
            "{why}：应记一次 contract_viol"
        );
        assert_eq!(
            w.body.last_status,
            crate::world::STATUS_BAD_ARGS,
            "{why}：应置 STATUS_BAD_ARGS"
        );
    }
}

/// `sh_fire` 的 `id` 越界走**与 14 个 setter 同一口径**：no-op + contract_viol +
/// STATUS_BAD_ARGS，不 Fault（判据集中在 `shooter_mut` 一处）。
#[test]
fn sh_fire_rejects_out_of_range_id() {
    let (mut w, ecl) = fresh_with_shooters();
    let mut t = Task::default();
    for bad in [SHOOTERS_PER_TASK as i32, -1] {
        let viol = w.body.diag.contract_viol;
        w.body.last_status = crate::world::STATUS_OK;
        assert!(
            call(&mut w, &ecl, &mut t, SYS_SH_FIRE, &[bad]).is_ok(),
            "id={bad} 是 no-op 而非 Fault"
        );
        assert_eq!(t.sp, 0, "id={bad} 腿也应把实参弹栈");
        assert_eq!(w.body.bullets.iter_alive().count(), 0, "id={bad} 不该发弹");
        assert_eq!(w.body.diag.contract_viol, viol + 1, "id={bad}");
        assert_eq!(
            w.body.last_status,
            crate::world::STATUS_BAD_ARGS,
            "id={bad}"
        );
    }
}

/// appearance 的在册/空格校验**推迟到开火那一刻**（T2 的 14 个 setter 只写字段）——
/// 越界号与空格都 Fault，且零副作用。这条正是 T2 复审 ① 那条"收窄必须保号越界性"
/// 的下游消费者：`sh_sprite` 若把负值钳成 0，这里就再也拒不掉了。
#[test]
fn sh_fire_faults_on_out_of_range_or_blank_appearance() {
    // ① 越界号（`sh_sprite` 收窄成 u16::MAX，远超表长）
    let (mut w, ecl) = fresh_with_shooters();
    let mut t = Task::default();
    sh(&mut w, &ecl, &mut t, SYS_SH_SPRITE, &[0, -4]);
    assert_eq!(
        call(&mut w, &ecl, &mut t, SYS_SH_FIRE, &[0]),
        Err(FAULT_BAD_OP),
        "越界 appearance 必须 Fault"
    );
    assert_eq!(w.body.bullets.iter_alive().count(), 0, "先验后建：零副作用");

    // ② 空格（合成表）
    const BLANK: i32 = 3 * 16 + 7;
    let holed = tables_with_hole(BLANK as usize);
    let (mut w, ecl) = fresh_with_shooters();
    let mut t = Task::default();
    sh(&mut w, &ecl, &mut t, SYS_SH_SPRITE, &[0, BLANK]);
    assert_eq!(
        call_with_tables(&mut w, &ecl, &mut t, SYS_SH_FIRE, &[0], &holed),
        Err(FAULT_BAD_OP),
        "空格 appearance 必须 Fault（隐形弹）"
    );
    assert_eq!(w.body.bullets.iter_alive().count(), 0);
}

/// `SH_ABS_OFFSET`：绝对偏移**不跟随 owner**，相对偏移跟随。
/// 必须用非 STAGE 的 owner 才判别得了——STAGE 的 `self_pos` 恒 (0,0)，两条分支同结果。
#[test]
fn abs_offset_ignores_owner_position_while_relative_follows_it() {
    let dx = Fx::from_int(7).raw();
    let dy = Fx::from_int(9).raw();
    let fire = |abs: bool| -> (i32, i32) {
        let (mut w, ecl) = fresh_with_shooters();
        let eh = crate::world::test_support::spawn_enemy(&mut w, 100, -60, 5);
        let mut t = Task {
            owner_kind: OWNER_ENEMY,
            owner_index: eh.index,
            owner_gen: eh.generation,
            ..Task::default()
        };
        sh(&mut w, &ecl, &mut t, SYS_SH_SPRITE, &[0, ROW_A]);
        let no = if abs {
            SYS_SH_OFFSET_ABS
        } else {
            SYS_SH_OFFSET
        };
        sh(&mut w, &ecl, &mut t, no, &[0, dx, dy]);
        sh(&mut w, &ecl, &mut t, SYS_SH_FIRE, &[0]);
        let i = w.body.bullets.iter_alive().next().expect("应发一颗");
        (w.body.bullets.x[i].raw(), w.body.bullets.y[i].raw())
    };
    assert_eq!(fire(true), (dx, dy), "绝对偏移：原点就是偏移量本身");
    assert_eq!(
        fire(false),
        (Fx::from_int(100).raw() + dx, Fx::from_int(-60).raw() + dy),
        "相对偏移：原点 = owner 位置 + 偏移量"
    );
}
