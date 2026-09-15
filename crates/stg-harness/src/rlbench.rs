//! `rl-bench`：stg-rl `VecEnv` 吞吐与单线程开销分解（spec §7）。
//!
//! ```text
//! rl-bench [--envs N] [--steps S] [--threads T] [--warmup W] [--cap C]
//!          [--workload default|dense] [--mark M] [--rank R] [--density K] [--profile]
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
//! `Env::step`（世界演化 + events + 可能的 reset）/ player+enemies+phase 编码 / bullets 编码 /
//! items 编码，并给出单线程理论 env-steps/s。不含 `VecEnv` 的调度与 CSR 压实。

use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use stg_core::input::BTN_SHOT;
use stg_core::tables::TABLES_V0;
use stg_rl::encode;
use stg_rl::env::{BootCache, EndOn, Env, EnvConfig, Start, compile};
use stg_rl::layout::{BULLETS_CAP_DEFAULT, BULLETS_CAP_MAX, ENEMIES_CAP, ITEMS_CAP};
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
        }
        let mut bullets_total_sum: u64 = 0;
        let mut bullets_dropped_sum: u64 = 0;
        let mut resets: u64 = 0;
        let start = Instant::now();
        for s in 0..o.steps as u32 {
            for (i, a) in act.iter_mut().enumerate() {
                *a = action(o.workload, o.warmup + s, i);
            }
            ve.step(&act, &mut buf.view())
                .map_err(|e| format!("step: {e}"))?;
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
    }
    Ok(())
}

/// 单线程逐段计时（µs/env-step）。
fn profile(o: &Opts, cfg: EnvConfig) -> Result<(), String> {
    let cap = cfg.bullets_cap;
    let mut env = Env::new(Arc::new(cfg), 0, Arc::new(BootCache::new()));
    let mut player = vec![0u8; 36];
    let mut enemies = vec![0u8; ENEMIES_CAP * 38];
    let mut bullets = vec![0u8; cap * 30];
    let mut items = vec![0u8; ITEMS_CAP * 18];
    let mut sel = encode::BulletScratch::new();
    for s in 0..o.warmup {
        env.step(action(o.workload, s, 0));
    }
    let (mut t_step, mut t_small, mut t_bullets, mut t_items) = (
        Duration::ZERO,
        Duration::ZERO,
        Duration::ZERO,
        Duration::ZERO,
    );
    let mut bullets_sum: u64 = 0;
    let mut resets: u64 = 0;
    for s in 0..o.steps as u32 {
        let a = action(o.workload, o.warmup + s, 0);
        let t0 = Instant::now();
        let out = env.step(a);
        let t1 = Instant::now();
        let w = env.world();
        encode::write_player(w, &TABLES_V0, &mut player);
        let _ = encode::write_enemies(w, &mut enemies);
        let _ = encode::phase_bits(w);
        let t2 = Instant::now();
        let st = encode::write_bullets(w, cap, &mut bullets, &mut sel);
        let t3 = Instant::now();
        let _ = encode::write_items(w, &mut items);
        let t4 = Instant::now();
        t_step += t1 - t0;
        t_small += t2 - t1;
        t_bullets += t3 - t2;
        t_items += t4 - t3;
        bullets_sum += st.total as u64;
        if out.done != 0 {
            resets += 1;
        }
    }
    let per = |d: Duration| d.as_secs_f64() * 1e6 / o.steps as f64;
    let total = per(t_step) + per(t_small) + per(t_bullets) + per(t_items);
    println!(
        "profile bullets_per_step={:.1} resets={resets}",
        bullets_sum as f64 / o.steps as f64
    );
    println!("  env.step               {:>9.2} µs", per(t_step));
    println!("  player+enemies+phase   {:>9.2} µs", per(t_small));
    println!("  bullets encode         {:>9.2} µs", per(t_bullets));
    println!("  items encode           {:>9.2} µs", per(t_items));
    println!(
        "  total                  {total:>9.2} µs  ⇒ 单线程 ≈ {:.0} env-steps/s",
        1e6 / total
    );
    Ok(())
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
    }
}
