//! `rl-bench`：stg-rl `VecEnv` 吞吐与单线程开销分解（spec §7）。
//!
//! ```text
//! rl-bench [--envs N] [--steps S] [--threads T] [--warmup W] [--cap C]
//!          [--workload default|dense] [--mark M] [--rank R] [--density K]
//!          [--profile | --profile-batch]
//!          [--warn-bullet-overflow]
//! ```
//!
//! **两种 workload**：
//! - `default`：内置 `game` 包、`--mark`（默认 15 boss 段）、随机移动+射击动作。随机动作下开局即死、
//!   弹量低（每 env ~12.5 弹），测的是「reset 频繁的低弹量」路径。
//! - `dense`：`scenes/rl_dense.ecl` 模板，每帧发 `--density` K 颗弹在上半区横穿（稳态 ≈ 112·K 弹），
//!   动作只按 SHOT 不移动，自机永远碰不到弹 ⇒ 不死、不 reset，计时区跑在 K 决定的弹量上。
//!   用来测密弹下观测编码与压实的真实开销（follow-ups D23#11）。
//!
//! **吞吐模式**（默认）：不给 `--threads` ⇒ 依次 1,2,4,…（≤ `available_parallelism`、≤ envs）；
//! 显式 `--threads` 夹到 `min(T, envs)`。先输出一行 `# ...` 配置，之后每行：
//! `threads envs steps env_steps_per_s bullets_total_per_env bullets_dropped_sum resets`。
//! 后三列是**计时区**内 `bullets_total` 的每 env 每步平均、`bullets_dropped` 总和、自动 reset 次数。
//! 动作缓冲在计时区外预分配、区内原地填写；计时区含每步三列观测统计（N×3 次求和，量小）。
//!
//! **`--profile`**：单个 `Env`、单线程，逐段计时一步的开销（µs/env-step）：
//! 世界 action 帧 / episode 管理与 reset / 全部观测（含激光），弹观测分为收集距离、
//! 最近集合选择、索引重排、行编码。不含 `VecEnv` 的调度与 CSR 压实。
//! **`--profile-batch`**：先跑普通吞吐，再另跑同输入诊断轮。输出整批墙钟相位与 worker
//! 累计区间（分开，不可相加）；计时会扰动负载。普通轮每 batch 仅增加两次外层时钟读取；
//! 最终吞吐对比请用不带 profile 标志的命令。不含 Python/H2D/GPU。

use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use stg_core::input::BTN_SHOT;
use stg_core::tables::TABLES_V0;
use stg_rl::encode;
use stg_rl::env::{BootCache, EndOn, Env, EnvConfig, Start, compile};
use stg_rl::layout::{BULLETS_CAP_DEFAULT, BULLETS_CAP_MAX};
use stg_rl::profile::{BatchProfile, ObservationProfile, WorkProfile};
use stg_rl::vec_env::{OwnedBuffers, VecEnv};

/// 密弹 workload 的 ECL 模板；`__DENSITY__` 替换为每帧发弹数。
const DENSE_TEMPLATE: &str = include_str!("../scenes/rl_dense.ecl");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Workload {
    Default,
    Dense,
}

#[derive(Clone, Debug)]
struct Opts {
    envs: usize,
    steps: usize,
    threads: Option<usize>,
    warmup: u32,
    cap: usize,
    workload: Workload,
    mark: i32,
    rank: i32,
    density: u32,
    profile: bool,
    profile_batch: bool,
    warn_bullet_overflow: bool,
}

impl Default for Opts {
    fn default() -> Self {
        Opts {
            envs: 512,
            steps: 2000,
            threads: None,
            warmup: 100,
            cap: BULLETS_CAP_DEFAULT,
            workload: Workload::Default,
            mark: 15,
            rank: 2,
            density: 6,
            profile: false,
            profile_batch: false,
            warn_bullet_overflow: false,
        }
    }
}

/// 密弹模板实例化（每帧发 `density` 颗）。
fn dense_source(density: u32) -> String {
    DENSE_TEMPLATE.replace("__DENSITY__", &density.to_string())
}

/// 第 `step` 步第 `i` 个 env 的动作。default：移动位（UP/DOWN）+ SHOT + SLOW，不含 BOMB（`0x53`）；
/// dense：只按 SHOT、不移动（自机停在场底，弹碰不到）。
fn action(workload: Workload, step: u32, i: usize) -> u32 {
    match workload {
        Workload::Default => {
            (step.wrapping_mul(2654435761) ^ (i as u32).wrapping_mul(40503)) & 0x53
        }
        Workload::Dense => BTN_SHOT,
    }
}

/// 按 workload 组 `EnvConfig`。
fn build_cfg(o: &Opts) -> Result<EnvConfig, String> {
    let (units, start_mark, max_frames, end_on) = match o.workload {
        Workload::Default => {
            let units: Vec<(String, String)> = stg_rl::bundled::bundled_sources("game")
                .ok_or_else(|| "内置 game 包缺失".to_string())?
                .iter()
                .map(|(n, s)| (n.to_string(), s.to_string()))
                .collect();
            let end_on = vec![
                EndOn::PhaseEnded,
                EndOn::SpellCaptured,
                EndOn::SpellFailed,
                EndOn::StageCleared,
            ];
            (units, o.mark, 3600, end_on)
        }
        Workload::Dense => (
            vec![("rl_dense.ecl".to_string(), dense_source(o.density))],
            0,
            u32::MAX,
            Vec::new(),
        ),
    };
    let image = compile(&units).map_err(|e| format!("workload 脚本编译失败:\n{e}"))?;
    Ok(EnvConfig {
        collision_rules: Default::default(),
        images: vec![image],
        starts: vec![Start {
            image: 0,
            mark: start_mark,
            rank: o.rank,
            weight: 1.0,
        }],
        frame_skip: 1,
        max_frames,
        warmup_max: 0,
        end_on,
        bullets_cap: o.cap,
        seed: 1,
    })
}

pub fn cmd_rl_bench(args: &[String]) -> ExitCode {
    match parse(args).and_then(|o| run(&o)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("rl-bench: {e}");
            ExitCode::FAILURE
        }
    }
}

fn parse(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--envs" => o.envs = usize_arg(args, i, "--envs")?,
            "--steps" => o.steps = usize_arg(args, i, "--steps")?,
            "--threads" => o.threads = Some(usize_arg(args, i, "--threads")?),
            "--warmup" => o.warmup = usize_arg(args, i, "--warmup")? as u32,
            "--cap" => o.cap = usize_arg(args, i, "--cap")?,
            "--mark" => o.mark = i32_arg(args, i, "--mark")?,
            "--rank" => o.rank = i32_arg(args, i, "--rank")?,
            "--density" => o.density = usize_arg(args, i, "--density")? as u32,
            "--workload" => {
                o.workload = match args.get(i + 1).map(String::as_str) {
                    Some("default") => Workload::Default,
                    Some("dense") => Workload::Dense,
                    other => return Err(format!("--workload 须为 default|dense，得 {other:?}")),
                }
            }
            "--profile" => {
                o.profile = true;
                i += 1;
                continue;
            }
            "--profile-batch" => {
                o.profile_batch = true;
                i += 1;
                continue;
            }
            "--warn-bullet-overflow" => {
                o.warn_bullet_overflow = true;
                i += 1;
                continue;
            }
            other => return Err(format!("未知参数 {other}")),
        }
        i += 2;
    }
    if o.envs == 0 {
        return Err("--envs 必须 > 0".to_string());
    }
    if o.steps == 0 {
        return Err("--steps 必须 > 0".to_string());
    }
    if o.cap == 0 || o.cap > BULLETS_CAP_MAX {
        return Err(format!("--cap 须在 1..={BULLETS_CAP_MAX}"));
    }
    if o.profile && o.profile_batch {
        return Err("--profile 与 --profile-batch 分别量单 env 和整批，请分开运行".to_string());
    }
    Ok(o)
}

fn run(o: &Opts) -> Result<(), String> {
    let cfg = build_cfg(o)?;
    println!(
        "# workload={:?} density={} mark={} rank={} cap={} envs={} steps={} warmup={}",
        o.workload, o.density, o.mark, o.rank, o.cap, o.envs, o.steps, o.warmup
    );
    if o.profile {
        return profile(o, cfg);
    }

    let avail = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let list: Vec<usize> = match o.threads {
        Some(0) => return Err("--threads 必须 > 0".to_string()),
        Some(t) => vec![t.min(o.envs)],
        None => {
            let mut v = Vec::new();
            let mut t = 1usize;
            while t <= avail && t <= o.envs {
                v.push(t);
                t *= 2;
            }
            v
        }
    };

    for t in list {
        let mut ve =
            VecEnv::new(cfg.clone(), o.envs, t).map_err(|e| format!("VecEnv::new: {e}"))?;
        let mut buf = OwnedBuffers::new(o.envs, o.cap);
        let mut warnings = o
            .warn_bullet_overflow
            .then(|| BulletOverflowWarnings::new(o.envs));
        ve.reset(&mut buf.view())
            .map_err(|e| format!("reset: {e}"))?;
        // 预热 `--warmup` 步（不记账）：default 让各 env 离开开局；dense 让弹幕铺到稳态（~112 帧）。
        let mut act = vec![0u32; o.envs];
        for s in 0..o.warmup {
            for (i, a) in act.iter_mut().enumerate() {
                *a = action(o.workload, s, i);
            }
            ve.step(&act, &mut buf.view())
                .map_err(|e| format!("warmup step: {e}"))?;
            if let Some(warnings) = &mut warnings {
                warnings.emit(&buf, o.cap);
            }
        }
        let mut bullets_total_sum: u64 = 0;
        let mut bullets_dropped_sum: u64 = 0;
        let mut resets: u64 = 0;
        let mut step_wall = Duration::ZERO;
        let start = Instant::now();
        for s in 0..o.steps as u32 {
            for (i, a) in act.iter_mut().enumerate() {
                *a = action(o.workload, o.warmup + s, i);
            }
            let step_start = o.profile_batch.then(Instant::now);
            ve.step(&act, &mut buf.view())
                .map_err(|e| format!("step: {e}"))?;
            if let Some(t0) = step_start {
                step_wall += t0.elapsed();
            }
            if let Some(warnings) = &mut warnings {
                warnings.emit(&buf, o.cap);
            }
            bullets_total_sum += buf.bullets_total.iter().map(|&x| x as u64).sum::<u64>();
            bullets_dropped_sum += buf.bullets_dropped.iter().map(|&x| x as u64).sum::<u64>();
            resets += buf.done.iter().filter(|&&d| d != 0).count() as u64;
        }
        let secs = start.elapsed().as_secs_f64();
        let eps = o.envs as f64 * o.steps as f64 / secs;
        let avg_total = bullets_total_sum as f64 / (o.steps as f64 * o.envs as f64);
        println!(
            "{t} {} {} {eps:.0} {avg_total:.1} {bullets_dropped_sum} {resets}",
            o.envs, o.steps
        );
        if o.profile_batch {
            println!(
                "# ordinary VecEnv.step wall: {:.2} µs/batch (two outer clock reads; excludes action fill/statistics)",
                step_wall.as_secs_f64() * 1e6 / o.steps as f64
            );
            profile_batch(o, cfg.clone(), t)?;
        }
    }
    Ok(())
}

/// 单 env：诊断区间，不含批量调度/压实或 Python/GPU。
fn profile(o: &Opts, cfg: EnvConfig) -> Result<(), String> {
    let cap = cfg.bullets_cap;
    let mut env = Env::new(Arc::new(cfg), 0, Arc::new(BootCache::new()));
    // 与生产缓冲共用布局长度，避免敌人 stride 等字段扩展后基准仍分配旧尺寸。
    let mut buf = OwnedBuffers::new(1, cap);
    let mut scratch = encode::BulletScratch::new();
    let mut warnings = o
        .warn_bullet_overflow
        .then(|| BulletOverflowWarnings::new(1));
    for s in 0..o.warmup {
        let out = env.step(action(o.workload, s, 0));
        // 连同编码暂存/派生量缓存预热，计时窗口不混入首次分配。
        let stats = encode::write_bullets(env.world(), cap, &mut buf.bullets, &mut scratch);
        if let Some(warnings) = &mut warnings {
            buf.done[0] = out.done;
            buf.bullets_total[0] = stats.total as i32;
            buf.bullets_dropped[0] = stats.dropped as i32;
            warnings.emit(&buf, cap);
        }
    }
    let mut workers = WorkProfile::default();
    let mut bullets_sum = 0u64;
    let mut rows_sum = 0u64;
    let loop_start = Instant::now();
    for s in 0..o.steps as u32 {
        let (out, env_profile) = env.step_profiled(action(o.workload, o.warmup + s, 0));
        let w = env.world();
        let mut observation = ObservationProfile::default();
        let t0 = Instant::now();
        encode::write_player(w, &TABLES_V0, &mut buf.player);
        buf.enemies_count[0] = encode::write_enemies(w, &mut buf.enemies) as i32;
        buf.phase[0] = encode::phase_bits(w);
        buf.frame[0] = w.frame();
        observation.small = t0.elapsed();
        let t0 = Instant::now();
        buf.lasers_count[0] = encode::write_lasers(w, &mut buf.lasers) as i32;
        observation.lasers = t0.elapsed();
        let (stats, bullet_profile) =
            encode::write_bullets_profiled(w, cap, &mut buf.bullets, &mut scratch);
        observation.bullets = bullet_profile;
        let t0 = Instant::now();
        encode::write_items(w, &mut buf.items);
        observation.items = t0.elapsed();
        if let Some(warnings) = &mut warnings {
            buf.done[0] = out.done;
            buf.bullets_total[0] = stats.total as i32;
            buf.bullets_dropped[0] = stats.dropped as i32;
            warnings.emit(&buf, cap);
        }
        workers += WorkProfile {
            env: env_profile,
            observation,
        };
        bullets_sum += stats.total as u64;
        rows_sum += stats.count as u64;
    }
    let loop_wall = loop_start.elapsed();
    std::hint::black_box(&buf);
    println!("profile single-env (instrumented; excludes VecEnv/CSR/Python/GPU)");
    println!(
        "  bullets/step={:.1} selected/step={:.1} resets={} core_frames={}",
        bullets_sum as f64 / o.steps as f64,
        rows_sum as f64 / o.steps as f64,
        workers.env.resets,
        workers.env.core_frames
    );
    print_workers(&workers, o.steps as f64);
    println!(
        "  diagnostic loop wall {:>9.2} µs/env-step (includes clocks/bookkeeping)",
        loop_wall.as_secs_f64() * 1e6 / o.steps as f64
    );
    Ok(())
}

fn print_workers(p: &WorkProfile, calls: f64) {
    let print = |name: &str, d: Duration| {
        println!(
            "  {name:<27} {:>9.2} µs/env-step",
            d.as_secs_f64() * 1e6 / calls
        );
    };
    println!("  worker intervals: summed elapsed / env calls; do not add to batch wall");
    print("core action frames", p.env.core_step);
    print("reset (incl warmup)", p.env.reset);
    print(
        "episode management+timers",
        p.env.total.saturating_sub(p.env.core_step + p.env.reset),
    );
    print("player+enemies+phase+frame", p.observation.small);
    print("lasers select+encode", p.observation.lasers);
    print("bullets count+gather+dist", p.observation.bullets.gather);
    print("bullets nearest selection", p.observation.bullets.nearest);
    print("bullets index reorder", p.observation.bullets.reorder);
    print("bullets rows+derived", p.observation.bullets.rows);
    print("items encode", p.observation.items);
}

fn profile_batch(o: &Opts, cfg: EnvConfig, threads: usize) -> Result<(), String> {
    let mut ve = VecEnv::new(cfg, o.envs, threads)?;
    let mut buf = OwnedBuffers::new(o.envs, o.cap);
    ve.reset(&mut buf.view())?;
    let mut actions = vec![0u32; o.envs];
    for s in 0..o.warmup {
        for (i, a) in actions.iter_mut().enumerate() {
            *a = action(o.workload, s, i);
        }
        ve.step(&actions, &mut buf.view())?;
    }
    let mut total = BatchProfile::default();
    let mut bullets_sum = 0u64;
    let mut rows_sum = 0u64;
    let mut dropped = 0u64;
    for s in 0..o.steps as u32 {
        for (i, a) in actions.iter_mut().enumerate() {
            *a = action(o.workload, o.warmup + s, i);
        }
        total += ve.step_profiled(&actions, &mut buf.view())?;
        bullets_sum += buf.bullets_total.iter().map(|&x| x as u64).sum::<u64>();
        rows_sum += *buf.bullets_offsets.last().unwrap() as u64;
        dropped += buf.bullets_dropped.iter().map(|&x| x as u64).sum::<u64>();
    }
    let calls = o.steps as f64 * o.envs as f64;
    println!("profile batch threads={threads} (instrumented; excludes Python/GPU)");
    println!(
        "  bullets/env-step={:.1} selected/env-step={:.1} dropped={dropped} resets={} core_frames={}",
        bullets_sum as f64 / calls,
        rows_sum as f64 / calls,
        total.workers.env.resets,
        total.workers.env.core_frames
    );
    println!("  batch wall: mean per VecEnv.step; includes scheduling/waiting");
    let print = |name: &str, d: Duration| {
        println!(
            "  {name:<27} {:>9.2} µs/batch",
            d.as_secs_f64() * 1e6 / o.steps as f64
        );
    };
    print("validate+build_work", total.prepare);
    print("parallel step+observations", total.parallel_step_observe);
    print("CSR prefix", total.prefix);
    print("CSR slice construction", total.split);
    print("CSR scatter+scheduling", total.scatter);
    print("profile aggregation", total.aggregate);
    print("full profiled VecEnv.step", total.total);
    print_workers(&total.workers, calls);
    Ok(())
}

/// One warning per env episode, aggregated across the current batch.
struct BulletOverflowWarnings {
    warned: Vec<bool>,
}

impl BulletOverflowWarnings {
    fn new(n: usize) -> Self {
        Self {
            warned: vec![false; n],
        }
    }

    fn observe(&mut self, buf: &OwnedBuffers, cap: usize) -> Option<(usize, i32, i64)> {
        let (mut count, mut peak, mut dropped) = (0, 0, 0i64);
        for (i, warned) in self.warned.iter_mut().enumerate() {
            // done is the previous episode; observations are already reset.
            if buf.done[i] != 0 {
                *warned = false;
            }
            if !*warned && buf.bullets_total[i] as usize > cap {
                *warned = true;
                count += 1;
                peak = peak.max(buf.bullets_total[i]);
                dropped += buf.bullets_dropped[i] as i64;
            }
        }
        (count > 0).then_some((count, peak, dropped))
    }

    fn emit(&mut self, buf: &OwnedBuffers, cap: usize) {
        if let Some((count, peak, dropped)) = self.observe(buf, cap) {
            eprintln!(
                "弹数超过观测上限：本批 {count} 个环境首次超限，最多 {peak} 颗，上限 {cap}，观测省略 {dropped} 颗；世界仍模拟全部弹。"
            );
        }
    }
}

fn usize_arg(args: &[String], i: usize, name: &str) -> Result<usize, String> {
    args.get(i + 1)
        .ok_or_else(|| format!("{name} 缺参数"))?
        .parse::<usize>()
        .map_err(|_| format!("{name} 参数非法"))
}

fn i32_arg(args: &[String], i: usize, name: &str) -> Result<i32, String> {
    args.get(i + 1)
        .ok_or_else(|| format!("{name} 缺参数"))?
        .parse::<i32>()
        .map_err(|_| format!("{name} 参数非法"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 密弹 workload 的判别押运：density 3 跑 300 步，**不得 done**（不死、不结束），且稳态弹数
    /// ≥ 250（理论 ≈ 336）。脚本被改坏（fault 停发 / 弹飞进自机 / 回收过快）任一都会红——
    /// 防 bench 静默退化回「低弹量」而数字照出。
    #[test]
    fn dense_workload_is_dense_and_deathless() {
        let o = Opts {
            workload: Workload::Dense,
            density: 3,
            ..Opts::default()
        };
        let mut env = Env::new(
            Arc::new(build_cfg(&o).expect("密弹模板必须能编译")),
            0,
            Arc::new(BootCache::new()),
        );
        for s in 0..300u32 {
            let out = env.step(action(o.workload, s, 0));
            assert_eq!(out.done, 0, "密弹 workload 第 {s} 步不得死亡 / 结束");
        }
        let alive = env.world().view().bullets().iter_alive().count();
        assert!(
            alive >= 250,
            "density 3 稳态应 ≥ 250 弹（理论 ≈336），实得 {alive}"
        );
        assert_eq!(env.world().view().players()[0].deaths, 0);
    }

    #[test]
    fn parse_rejects_bad_workload_and_cap() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert!(parse(&s(&["--workload", "nope"])).is_err());
        assert!(parse(&s(&["--cap", "0"])).is_err());
        let o = parse(&s(&["--workload", "dense", "--density", "9", "--profile"])).unwrap();
        assert_eq!(
            (o.workload, o.density, o.profile),
            (Workload::Dense, 9, true)
        );
        assert!(parse(&s(&["--profile", "--profile-batch"])).is_err());
        assert!(parse(&s(&["--profile-batch"])).unwrap().profile_batch);
        assert!(
            parse(&s(&["--warn-bullet-overflow"]))
                .unwrap()
                .warn_bullet_overflow
        );
        assert!(!parse(&[]).unwrap().warn_bullet_overflow);
    }

    #[test]
    fn overflow_warning_aggregates_and_rearms_only_reset_envs() {
        let mut warnings = BulletOverflowWarnings::new(2);
        let mut buf = OwnedBuffers::new(2, 4);
        buf.bullets_total.copy_from_slice(&[7, 5]);
        buf.bullets_dropped.copy_from_slice(&[3, 1]);
        assert_eq!(warnings.observe(&buf, 4), Some((2, 7, 4)));
        assert_eq!(warnings.observe(&buf, 4), None);
        // Falling below cap does not start a new episode.
        buf.bullets_total.fill(0);
        assert_eq!(warnings.observe(&buf, 4), None);
        buf.bullets_total.copy_from_slice(&[7, 5]);
        buf.done[0] = 3;
        assert_eq!(warnings.observe(&buf, 4), Some((1, 7, 3)));
        buf.done[0] = 0;
        assert_eq!(warnings.observe(&buf, 4), None);
    }
}
