//! `rl-bench [--envs N] [--steps S] [--threads T] [--mark M] [--rank R]`：stg-rl VecEnv 吞吐（spec §7）。
//!
//! 不给 `--threads` ⇒ 依次 1,2,4,…（≤ `available_parallelism`、≤ envs）。输出每行：
//! `threads envs steps env_steps_per_s`。动作只含移动/射击/低速（不 bomb），让整段基准
//! 稳定在「boss 段满屏弹」的稳态；`max_frames` 设大以免计时区内发生 reset。

use std::process::ExitCode;
use std::time::Instant;

use stg_rl::env::{EndOn, EnvConfig, Start, compile};
use stg_rl::vec_env::{BufferSet, VecEnv, buffer_sizes};

/// 调用方持有的扁平缓冲（结构同 `tests/vec_env.rs::Owned`，就地复制一份）。
struct Buf {
    frame: Vec<u32>,
    phase: Vec<u32>,
    player: Vec<u8>,
    enemies: Vec<u8>,
    enemies_count: Vec<i32>,
    bullets: Vec<u8>,
    bullets_offsets: Vec<i32>,
    items: Vec<u8>,
    items_offsets: Vec<i32>,
    lasers_count: Vec<i32>,
    bullets_total: Vec<i32>,
    bullets_dropped: Vec<i32>,
    events: Vec<i32>,
    done: Vec<u8>,
    ep_frames: Vec<i32>,
    warmup_retries: Vec<i32>,
    start_index: Vec<i32>,
}

impl Buf {
    fn new(n: usize, cap: usize) -> Buf {
        let s = buffer_sizes(n, cap);
        Buf {
            frame: vec![0; s.frame],
            phase: vec![0; s.phase],
            player: vec![0; s.player],
            enemies: vec![0; s.enemies],
            enemies_count: vec![0; s.enemies_count],
            bullets: vec![0; s.bullets],
            bullets_offsets: vec![0; s.bullets_offsets],
            items: vec![0; s.items],
            items_offsets: vec![0; s.items_offsets],
            lasers_count: vec![0; s.lasers_count],
            bullets_total: vec![0; s.bullets_total],
            bullets_dropped: vec![0; s.bullets_dropped],
            events: vec![0; s.events],
            done: vec![0; s.done],
            ep_frames: vec![0; s.ep_frames],
            warmup_retries: vec![0; s.warmup_retries],
            start_index: vec![0; s.start_index],
        }
    }

    fn view(&mut self) -> BufferSet<'_> {
        BufferSet {
            frame: &mut self.frame,
            phase: &mut self.phase,
            player: &mut self.player,
            enemies: &mut self.enemies,
            enemies_count: &mut self.enemies_count,
            bullets: &mut self.bullets,
            bullets_offsets: &mut self.bullets_offsets,
            items: &mut self.items,
            items_offsets: &mut self.items_offsets,
            lasers_count: &mut self.lasers_count,
            bullets_total: &mut self.bullets_total,
            bullets_dropped: &mut self.bullets_dropped,
            events: &mut self.events,
            done: &mut self.done,
            ep_frames: &mut self.ep_frames,
            warmup_retries: &mut self.warmup_retries,
            start_index: &mut self.start_index,
        }
    }
}

/// 动作：移动位（UP/DOWN）+ SHOT + SLOW，不含 BOMB（`0x53`）。
fn action(step: u32, i: usize) -> u32 {
    (step.wrapping_mul(2654435761) ^ (i as u32).wrapping_mul(40503)) & 0x53
}

pub fn cmd_rl_bench(args: &[String]) -> ExitCode {
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("rl-bench: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let mut envs = 512usize;
    let mut steps = 2000usize;
    let mut threads: Option<usize> = None;
    let mut mark = 15i32;
    let mut rank = 2i32;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--envs" => {
                envs = usize_arg(args, i, "--envs")?;
                i += 2;
            }
            "--steps" => {
                steps = usize_arg(args, i, "--steps")?;
                i += 2;
            }
            "--threads" => {
                threads = Some(usize_arg(args, i, "--threads")?);
                i += 2;
            }
            "--mark" => {
                mark = i32_arg(args, i, "--mark")?;
                i += 2;
            }
            "--rank" => {
                rank = i32_arg(args, i, "--rank")?;
                i += 2;
            }
            other => return Err(format!("未知参数 {other}")),
        }
    }
    if envs == 0 {
        return Err("--envs 必须 > 0".to_string());
    }
    if steps == 0 {
        return Err("--steps 必须 > 0".to_string());
    }

    let units: Vec<(String, String)> = stg_rl::bundled::bundled_sources("game")
        .ok_or_else(|| "内置 game 包缺失".to_string())?
        .iter()
        .map(|(n, s)| (n.to_string(), s.to_string()))
        .collect();
    let image = compile(&units).map_err(|e| format!("game 包编译失败:\n{e}"))?;
    let cfg = EnvConfig {
        images: vec![image],
        starts: vec![Start {
            image: 0,
            mark,
            rank,
            weight: 1.0,
        }],
        frame_skip: 1,
        max_frames: 3600,
        warmup_max: 0,
        end_on: vec![
            EndOn::PhaseEnded,
            EndOn::SpellCaptured,
            EndOn::SpellFailed,
            EndOn::StageCleared,
        ],
        bullets_cap: 1024,
        seed: 1,
    };
    let cap = cfg.bullets_cap;

    let avail = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let list: Vec<usize> = match threads {
        Some(0) => return Err("--threads 必须 > 0".to_string()),
        Some(t) => vec![t],
        None => {
            let mut v = Vec::new();
            let mut t = 1usize;
            while t <= avail && t <= envs {
                v.push(t);
                t *= 2;
            }
            v
        }
    };

    for t in list {
        let mut ve = VecEnv::new(cfg.clone(), envs, t).map_err(|e| format!("VecEnv::new: {e}"))?;
        let mut buf = Buf::new(envs, cap);
        ve.reset(&mut buf.view())
            .map_err(|e| format!("reset: {e}"))?;
        // 预热 100 步（不记账），把弹幕打到稳态。
        for s in 0..100u32 {
            let act: Vec<u32> = (0..envs).map(|i| action(s, i)).collect();
            ve.step(&act, &mut buf.view())
                .map_err(|e| format!("warmup step: {e}"))?;
        }
        // 计时区。
        let start = Instant::now();
        for s in 0..steps as u32 {
            let act: Vec<u32> = (0..envs).map(|i| action(100 + s, i)).collect();
            ve.step(&act, &mut buf.view())
                .map_err(|e| format!("step: {e}"))?;
        }
        let secs = start.elapsed().as_secs_f64();
        let eps = envs as f64 * steps as f64 / secs;
        println!("{t} {envs} {steps} {eps:.0}");
    }
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
