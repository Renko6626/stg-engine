//! 激光创建、几何与挂靠。

use super::*;

// ── 8xx 激光族（Task 4；spec 2026-09-25-laser-pool-design §5）────────────────

/// 一条字段全零的 `LaserInit`（池满测试用；`LaserInit` 是 exhaustive、无 `Default`）。
fn zero_laser_init() -> crate::lasers::LaserInit {
    crate::lasers::LaserInit {
        ox: Fx::ZERO,
        oy: Fx::ZERO,
        angle: Angle::ZERO,
        omega: 0,
        start: Fx::ZERO,
        end: Fx::ZERO,
        start_len: Fx::ZERO,
        speed: Fx::ZERO,
        width: Fx::ZERO,
        sprite: 0,
        warn: 0,
        active: 1,
        fade: 0,
        timer: 0,
        state: 0,
        anchor_idx: crate::lasers::ANCHOR_NONE,
        anchor_gen: 0,
        ax: Fx::ZERO,
        ay: Fx::ZERO,
        dx: Fx::ZERO,
        dy: Fx::ZERO,
        dang: 0,
        px: Fx::ZERO,
        py: Fx::ZERO,
        pang: Angle::ZERO,
        flags: 0,
        born_frame: 0,
    }
}

/// `laser()` 的 9 参正序：`color,x,y,angle,len,width,warn,active,fade`。
#[allow(clippy::too_many_arguments)]
fn laser_args(
    color: i32,
    x: Fx,
    y: Fx,
    angle: Angle,
    len: Fx,
    width: Fx,
    warn: i32,
    active: i32,
    fade: i32,
) -> [i32; 9] {
    [
        color,
        x.raw(),
        y.raw(),
        angle.raw() as i32,
        len.raw(),
        width.raw(),
        warn,
        active,
        fade,
    ]
}

/// 打包激光句柄 → 池槽下标。
fn laser_slot(packed: i32) -> usize {
    (packed & 0xFFFF) as usize
}

/// `laser()` 返回打包句柄；解码后字段逐项正确（形态一：start=0、end=start_len=len、
/// speed=0、sprite=color、omega=0）。
#[test]
fn laser_create_packs_handle_and_fills_fields() {
    let (mut w, ecl) = fresh();
    let mut task = Task::default();
    let args = laser_args(
        3,
        Fx::from_int(10),
        Fx::from_int(20),
        Angle(16384),
        Fx::from_int(500),
        Fx::from_int(32),
        30,
        120,
        16,
    );
    assert!(call(&mut w, &ecl, &mut task, SYS_LASER_CREATE, &args).is_ok());
    let packed = task.stack[0];
    assert!(packed >= 0, "创建成功压回打包句柄");
    let i = laser_slot(packed);
    assert!(w.body.lasers.is_alive(i), "打包句柄应指向活槽");
    assert_eq!(
        (packed >> 16) & 0x7FFF,
        (w.body.lasers.generation[i] & 0x7FFF) as i32,
        "打包句柄只带池代际的低 15 位"
    );
    let l = &w.body.lasers;
    assert_eq!(l.ox[i], Fx::from_int(10));
    assert_eq!(l.oy[i], Fx::from_int(20));
    assert_eq!(l.angle[i], Angle(16384));
    assert_eq!(l.start[i], Fx::ZERO, "start = 0");
    assert_eq!(l.end[i], Fx::from_int(500), "end = len");
    assert_eq!(l.start_len[i], Fx::from_int(500), "start_len = len");
    assert_eq!(l.speed[i], Fx::ZERO, "speed = 0");
    assert_eq!(l.omega[i], 0, "omega = 0");
    assert_eq!(l.width[i], Fx::from_int(32));
    assert_eq!(l.sprite[i], 3, "sprite = color");
    assert_eq!(l.warn[i], 30);
    assert_eq!(l.active[i], 120);
    assert_eq!(l.fade[i], 16);
    assert_eq!(l.flags[i], 0);
}

/// `color ∉ 0..=15` → `FAULT_BAD_OP`，先验后建（不建半成品）。
#[test]
fn laser_bad_color_faults_without_creating() {
    for color in [-1, 16] {
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        let args = laser_args(
            color,
            Fx::ZERO,
            Fx::ZERO,
            Angle::ZERO,
            Fx::ZERO,
            Fx::ZERO,
            0,
            0,
            0,
        );
        assert_eq!(
            call(&mut w, &ecl, &mut task, SYS_LASER_CREATE, &args),
            Err(FAULT_BAD_OP),
            "color={color} 应 Fault"
        );
        assert_eq!(w.body.lasers.iter_alive().count(), 0, "先验后建：零副作用");
    }
}

/// 池满 → 押 -1，不 Fault，`pool_full[POOL_LASER]` 加 1。
#[test]
fn laser_pool_full_pushes_neg1_and_counts() {
    let (mut w, ecl) = fresh();
    for k in 0..crate::lasers::LaserPool::CAP {
        assert_ne!(
            w.body.create_laser(zero_laser_init()),
            crate::lasers::LaserHandle::NULL,
            "第 {k} 条应成功"
        );
    }
    let mut task = Task::default();
    let pf0 = w.body.diag.pool_full[crate::world::POOL_LASER];
    let args = laser_args(
        0,
        Fx::ZERO,
        Fx::ZERO,
        Angle::ZERO,
        Fx::ZERO,
        Fx::ZERO,
        0,
        0,
        0,
    );
    assert!(call(&mut w, &ecl, &mut task, SYS_LASER_CREATE, &args).is_ok());
    assert_eq!(task.stack[0], -1, "池满押 -1，不 Fault");
    assert_eq!(
        w.body.diag.pool_full[crate::world::POOL_LASER],
        pf0 + 1,
        "池满计一次"
    );
}

/// **只覆盖时间字段**：`warn/active/fade` 出 `[0,65535]` → 钳位；这三个字段一起越界时
/// `laser()` 只计一次违约。几何字段（坐标/长度/宽度）另由 `world::create_laser` 再计一次，
/// 故"时间 + 几何同时坏"的一次调用会合计 2 次——控制方裁定保持现状，本测试不覆盖那半边。
#[test]
fn laser_time_fields_clamp_out_of_u16_and_count_once() {
    let (mut w, ecl) = fresh();
    let mut task = Task::default();
    let cv0 = w.body.diag.contract_viol;
    let args = laser_args(
        0,
        Fx::ZERO,
        Fx::ZERO,
        Angle::ZERO,
        Fx::ZERO,
        Fx::ZERO,
        100000,
        -1,
        70000,
    );
    assert!(call(&mut w, &ecl, &mut task, SYS_LASER_CREATE, &args).is_ok());
    let i = laser_slot(task.stack[0]);
    assert_eq!(w.body.lasers.warn[i], u16::MAX, "warn 上钳");
    assert_eq!(w.body.lasers.active[i], 0, "active 负值下钳 0");
    assert_eq!(w.body.lasers.fade[i], u16::MAX, "fade 上钳");
    assert_eq!(w.body.diag.contract_viol, cv0 + 1, "一次 create 只计一次");
}

/// `lz_omega` 的栈值取**低 16 位按位回绕**为 `i16`（BAM 语义：反向角速度编码后原始值大于
/// 32767，例如 65000 == i16 −536），**不钳位、不计数**。判别腿：旧实现把它钳到
/// `i16::MAX`（最大正转速），此断言必红；合法负角（47332 == −18204）原样写入且不计数。
#[test]
fn lz_omega_wraps_bits_not_clamps() {
    let (mut w, ecl) = fresh();
    let mut task = Task::default();
    let args = laser_args(
        0,
        Fx::ZERO,
        Fx::ZERO,
        Angle::ZERO,
        Fx::from_int(100),
        Fx::from_int(8),
        0,
        9999,
        0,
    );
    assert!(call(&mut w, &ecl, &mut task, SYS_LASER_CREATE, &args).is_ok());
    let lz = task.stack[0];
    let i = laser_slot(lz);

    let cv0 = w.body.diag.contract_viol;
    // 65000 > 32767：按位回绕为 i16 −536（反向扫射），不计数。
    task.sp = 0;
    assert!(call(&mut w, &ecl, &mut task, SYS_LASER_OMEGA, &[lz, 65000]).is_ok());
    assert_eq!(
        w.body.lasers.omega[i], 65000u16 as i16,
        ">32767 按位回绕为负"
    );
    assert_eq!(w.body.diag.contract_viol, cv0, "按位回绕不计数");

    // 47332 == u16 0xB8C4 == i16 −18204：合法负角速度原样写入，不计数。
    task.sp = 0;
    assert!(call(&mut w, &ecl, &mut task, SYS_LASER_OMEGA, &[lz, 47332]).is_ok());
    assert_eq!(w.body.lasers.omega[i], -18204, "负角速度原样");
    assert_eq!(w.body.diag.contract_viol, cv0, "仍不计数");
}

/// 激光几何读口（810–814）：读调用时刻池里的 `ox/oy/angle/start/end`；五个值取互不相同的数，
/// 读错字段即红。失效句柄押 0、不计数（同 `enemy_x` 与 `lz_alive` 的读族口径）。
#[test]
fn lz_readers_return_current_geometry_and_zero_when_stale() {
    let (mut w, ecl) = fresh();
    let mut task = Task::default();
    let a = laser_args(
        1,
        Fx::from_int(-30),
        Fx::from_int(70),
        Angle(12345),
        Fx::from_int(420),
        Fx::from_int(8),
        0,
        9999,
        0,
    );
    assert!(call(&mut w, &ecl, &mut task, SYS_LASER_CREATE, &a).is_ok());
    let lz = task.stack[0];
    let i = laser_slot(lz);
    task.sp = 0;
    assert!(
        call(
            &mut w,
            &ecl,
            &mut task,
            SYS_LASER_START,
            &[lz, Fx::from_int(32).raw()]
        )
        .is_ok()
    );

    let read = |w: &mut crate::step::World, task: &mut Task, sys: u16, packed: i32| -> i32 {
        task.sp = 0;
        assert!(call(w, &ecl, task, sys, &[packed]).is_ok());
        task.stack[0]
    };
    let cv0 = w.body.diag.contract_viol;
    assert_eq!(
        read(&mut w, &mut task, SYS_LASER_X, lz),
        Fx::from_int(-30).raw()
    );
    assert_eq!(
        read(&mut w, &mut task, SYS_LASER_Y, lz),
        Fx::from_int(70).raw()
    );
    assert_eq!(read(&mut w, &mut task, SYS_LASER_ANGLE, lz), 12345);
    assert_eq!(
        read(&mut w, &mut task, SYS_LASER_NEAR, lz),
        Fx::from_int(32).raw()
    );
    assert_eq!(
        read(&mut w, &mut task, SYS_LASER_FAR, lz),
        Fx::from_int(420).raw()
    );

    // 读的是当前值：改过之后再读，拿到新值。
    task.sp = 0;
    assert!(call(&mut w, &ecl, &mut task, SYS_LASER_ROTATE, &[lz, 1000]).is_ok());
    assert_eq!(read(&mut w, &mut task, SYS_LASER_ANGLE, lz), 13345);

    // 失效句柄：五个读口都押 0，不计数。
    w.body.lasers.free_index(i);
    for sys in [
        SYS_LASER_X,
        SYS_LASER_Y,
        SYS_LASER_ANGLE,
        SYS_LASER_NEAR,
        SYS_LASER_FAR,
    ] {
        assert_eq!(read(&mut w, &mut task, sys, lz), 0, "syscall {sys}");
    }
    assert_eq!(read(&mut w, &mut task, SYS_LASER_X, -1), 0);
    assert_eq!(
        w.body.diag.contract_viol, cv0,
        "读口一律不计数（含失效句柄）"
    );
}

/// Review Focus 3：激光 A 回收、新激光 B 复用同槽后，拿 A 的旧句柄调 `lz_rotate`
/// 不得改到 B；计一次违约；`lz_alive(A) == 0`、`lz_alive(B) == 1`。
#[test]
fn stale_laser_handle_does_not_touch_reused_slot() {
    let (mut w, ecl) = fresh();
    let mut task = Task::default();
    let a = laser_args(
        1,
        Fx::ZERO,
        Fx::ZERO,
        Angle::ZERO,
        Fx::from_int(100),
        Fx::from_int(16),
        0,
        9999,
        0,
    );
    assert!(call(&mut w, &ecl, &mut task, SYS_LASER_CREATE, &a).is_ok());
    let packed_a = task.stack[0];
    let ia = laser_slot(packed_a);
    let gen_a = w.body.lasers.generation[ia];

    // 模拟相位 5 回收 A，再用 B 复用同槽（最低空位优先）。
    w.body.lasers.free_index(ia);
    task.sp = 0;
    let b = laser_args(
        2,
        Fx::ZERO,
        Fx::ZERO,
        Angle(1000),
        Fx::from_int(200),
        Fx::from_int(8),
        0,
        9999,
        0,
    );
    assert!(call(&mut w, &ecl, &mut task, SYS_LASER_CREATE, &b).is_ok());
    let packed_b = task.stack[0];
    let ib = laser_slot(packed_b);
    assert_eq!(ib, ia, "最低空位复用同槽");
    assert_ne!(w.body.lasers.generation[ib], gen_a, "复用槽代际必须前进");
    let angle_b = w.body.lasers.angle[ib];

    let cv0 = w.body.diag.contract_viol;
    task.sp = 0;
    assert!(call(&mut w, &ecl, &mut task, SYS_LASER_ROTATE, &[packed_a, 1234]).is_ok());
    assert_eq!(w.body.lasers.angle[ib], angle_b, "旧句柄不得改到复用槽的 B");
    assert_eq!(w.body.diag.contract_viol, cv0 + 1, "失效句柄计一次");

    task.sp = 0;
    assert!(call(&mut w, &ecl, &mut task, SYS_LASER_ALIVE, &[packed_a]).is_ok());
    assert_eq!(task.stack[0], 0, "A 已回收");
    task.sp = 0;
    assert!(call(&mut w, &ecl, &mut task, SYS_LASER_ALIVE, &[packed_b]).is_ok());
    assert_eq!(task.stack[0], 1, "B 存活");
    assert_eq!(w.body.diag.contract_viol, cv0 + 1, "lz_alive 只读不计数");
}

/// `lz_anchor` 挂上活敌后立即吸附、相位 5 逐帧跟随；`lz_anchor(lz, -1, …)` 解除挂靠。
#[test]
fn lz_anchor_follows_enemy_then_detaches_with_neg1() {
    use crate::input::InputFrame;
    let (mut w, ecl) = fresh();
    let e = crate::world::test_support::spawn_enemy(&mut w, 0, 100, 5);
    let ei = e.index as usize;
    w.body.enemies.vx[ei] = Fx::from_int(2);
    let mut task = Task {
        owner_kind: OWNER_ENEMY,
        owner_index: e.index,
        owner_gen: e.generation,
        ..Task::default()
    };
    let a = laser_args(
        0,
        Fx::ZERO,
        Fx::ZERO,
        Angle::ZERO,
        Fx::from_int(500),
        Fx::from_int(16),
        0,
        9999,
        0,
    );
    assert!(call(&mut w, &ecl, &mut task, SYS_LASER_CREATE, &a).is_ok());
    let lz = task.stack[0];
    let i = laser_slot(lz);

    // brief 的原句：`lz_anchor($self_enemy, 0, 8)`——先经 `$self_enemy` 拿打包敌号。
    task.sp = 0;
    assert!(call(&mut w, &ecl, &mut task, SYS_SELF_ENEMY, &[]).is_ok());
    let self_enemy = task.stack[0];
    task.sp = 0;
    let anchor = [lz, self_enemy, Fx::ZERO.raw(), Fx::from_int(8).raw()];
    assert!(call(&mut w, &ecl, &mut task, SYS_LASER_ANCHOR, &anchor).is_ok());
    assert_eq!(w.body.lasers.anchor_idx[i], e.index, "挂上敌下标");
    assert_eq!(w.body.lasers.ox[i], Fx::ZERO, "出生帧立即吸附 x");
    assert_eq!(w.body.lasers.oy[i], Fx::from_int(108), "出生帧立即吸附 y+8");

    // 相位 5 一帧：敌人 x += 2，激光跟到 2。
    crate::world::test_support::step_t(&mut w, &InputFrame::empty(0));
    assert_eq!(w.body.lasers.ox[i], Fx::from_int(2));
    assert_eq!(w.body.lasers.oy[i], Fx::from_int(108));

    // -1 解除：锚态清空、原点留在原地。
    task.sp = 0;
    assert!(call(&mut w, &ecl, &mut task, SYS_LASER_ANCHOR, &[lz, -1, 0, 0]).is_ok());
    assert_eq!(
        w.body.lasers.anchor_idx[i],
        crate::lasers::ANCHOR_NONE,
        "-1 解除挂靠"
    );
    assert_eq!(w.body.lasers.ox[i], Fx::from_int(2), "解除不动原点");
}

/// 控制方裁定 ②：失效敌号（非 -1，已死/代际不符）→ 不挂靠 + 计一次违约。
#[test]
fn lz_anchor_stale_enemy_is_noop_and_counted() {
    let (mut w, ecl) = fresh();
    let e = crate::world::test_support::spawn_enemy(&mut w, 0, 100, 5);
    let mut task = Task::default();
    let a = laser_args(
        0,
        Fx::ZERO,
        Fx::ZERO,
        Angle::ZERO,
        Fx::from_int(500),
        Fx::from_int(16),
        0,
        9999,
        0,
    );
    assert!(call(&mut w, &ecl, &mut task, SYS_LASER_CREATE, &a).is_ok());
    let lz = task.stack[0];
    let i = laser_slot(lz);

    let live = crate::enemy::pack_handle(e);
    task.sp = 0;
    assert!(call(&mut w, &ecl, &mut task, SYS_LASER_ANCHOR, &[lz, live, 0, 0]).is_ok());
    assert_eq!(w.body.lasers.anchor_idx[i], e.index);
    assert_eq!(w.body.lasers.anchor_gen[i], e.generation);

    // 杀敌回收槽 → 旧敌号失效（非 -1）。
    w.body.enemies.free(e);
    let cv0 = w.body.diag.contract_viol;
    task.sp = 0;
    assert!(call(&mut w, &ecl, &mut task, SYS_LASER_ANCHOR, &[lz, live, 0, 0]).is_ok());
    assert_eq!(w.body.diag.contract_viol, cv0 + 1, "失效敌号计一次");
    assert_eq!(
        w.body.lasers.anchor_idx[i], e.index,
        "失败调用是 no-op：不改锚态"
    );
}

/// 跨任务陷阱（阶段终审 I-3）：ECL 敌号只带代际低 15 位，而相位 5 的挂靠比较用**完整
/// u16 代际**。`lz_anchor` 必须经 `resolve_enemy_handle` 取下标、再用池里的完整代际重建
/// 句柄——用低 15 位复刻时，代际越过 0x7FFF 后挂靠会立刻静默脱钩。
#[test]
fn lz_anchor_survives_enemy_generation_above_15_bits() {
    use crate::input::InputFrame;
    let (mut w, ecl) = fresh();
    let e = crate::world::test_support::spawn_enemy(&mut w, 0, 100, 5);
    let ei = e.index as usize;
    w.body.enemies.generation[ei] = 0xF00D;
    w.body.enemies.vx[ei] = Fx::from_int(2);
    let mut task = Task::default();
    let a = laser_args(
        0,
        Fx::ZERO,
        Fx::ZERO,
        Angle::ZERO,
        Fx::from_int(500),
        Fx::from_int(16),
        0,
        9999,
        0,
    );
    assert!(call(&mut w, &ecl, &mut task, SYS_LASER_CREATE, &a).is_ok());
    let lz = task.stack[0];
    let i = laser_slot(lz);

    let packed = crate::enemy::pack_handle(EnemyHandle {
        index: e.index,
        generation: 0xF00D,
    });
    assert_eq!(packed >> 16, 0x700D, "前提：句柄只带低 15 位");
    task.sp = 0;
    assert!(
        call(
            &mut w,
            &ecl,
            &mut task,
            SYS_LASER_ANCHOR,
            &[lz, packed, 0, 0]
        )
        .is_ok()
    );
    assert_eq!(
        w.body.lasers.anchor_gen[i], 0xF00D,
        "必须存池里的完整 u16 代际"
    );

    // 若用低 15 位重建，anchor_gen 会落 0x700D，相位 5 代际不符 → 立刻脱钩。
    crate::world::test_support::step_t(&mut w, &InputFrame::empty(0));
    assert_eq!(w.body.lasers.anchor_idx[i], e.index, "代际相符：仍挂靠");
    assert_eq!(w.body.lasers.ox[i], Fx::from_int(2), "跟随敌人本帧位移");
    crate::world::test_support::step_t(&mut w, &InputFrame::empty(1));
    assert_eq!(w.body.lasers.ox[i], Fx::from_int(4));
}
