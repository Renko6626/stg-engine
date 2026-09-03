//! run —— 跑任意 `.ecl`，把结果打成**可断言的事实**（harness run 刀，2026-08-01）。
//!
//! 缺口（三个方向撞到同一处）：`check` 只管语法，`serve` 写死跑 `rainbow.ecl`，
//! `godot/ecl/demo/` 是整取编译（加第四个文件会撞它自己的 `sub main()`）——写完一段
//! `.ecl` 之后**没有任何办法看它到底干了什么**。本模块补上这一段：
//! 建世界 → 逐帧 `step` → 输出计数/峰值/末帧 + 可选的单帧弹表。
//!
//! **最要紧的一格是 fault 不静默**：脚本被 `FAULT_BUDGET` 或坏参数杀掉时，引擎侧只有
//! `diag.task_faults` 一个计数在动，终端上一个字都没有。本命令把 `EVT_TASK_FAULT`
//! 逐条打出来并**退非零码**——静默失败在这里必须响。
//!
//! 帧号口径（全命令统一，看输出前先读这一句）：**帧 N = 第 N 次 `step` 之后**。故
//! 帧 0 = 未 step 的开局，`--frames 600` 的末行是帧 600；fault 标的帧号 N 意为
//! "第 N 次 step 里发生"。这与 viewer 线格式的 `w.frame()` 同口径。
//!
//! 断层线【以上】：本模块用浮点只为**打印**（Fx/BAM → 十进制），不喂回世界。

use std::process::ExitCode;

use stg_core::World;
use stg_core::ecl::image::EclImage;
use stg_core::input::InputFrame;
use stg_core::world::DiagCounters;

/// `--at` 单帧弹表的打印上限。满弹池 8192 行会把终端冲掉，超出部分只报条数。
const AT_DUMP_LIMIT: usize = 1024;

/// 采样行的目标条数（外加帧 0 与末帧）。
const SAMPLE_ROWS: u32 = 10;

// ── fault 码渲染 ───────────────────────────────────────────────────────────
//
// **短名来自 core，解释归这里**（F9 已还，2026-09-03）：`stg_core::ecl::FAULT_NAMES` 的
// 下标即码号，是唯一真相源——此前 harness 抄了第二份码→名字表，core 加了新码而这边没跟
// 就会静默漂。现在 core 加码必须在 `FAULT_NAMES` 里加一行（数组长度固定，编译期强制），
// 而这边**跟漏的最坏结果是"打出短名、没有中文解释"**，不再是"未知 fault 码"。
fn fault_name(code: u8) -> String {
    let short = stg_core::ecl::FAULT_NAMES
        .get(code as usize)
        .copied()
        .unwrap_or("?");
    let hint = match code {
        stg_core::ecl::FAULT_BAD_OP => "非法指令/坏 syscall 号",
        stg_core::ecl::FAULT_PC_OOB => "pc 或跳转目标越界",
        stg_core::ecl::FAULT_STACK => "求值栈上溢/下溢",
        stg_core::ecl::FAULT_BUDGET => "指令预算耗尽（多半是没 wait 的死循环）",
        stg_core::ecl::FAULT_DIV_ZERO => "除零",
        stg_core::ecl::FAULT_CALL_DEPTH => "调用深度超限",
        stg_core::ecl::FAULT_UNIMPLEMENTED => "保留码（T3 起不再产出）",
        _ => "",
    };
    if hint.is_empty() {
        short.to_string()
    } else {
        format!("{short} {hint}")
    }
}

// ── 观测数据 ───────────────────────────────────────────────────────────────

/// 一次观测的活体计数。
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(crate) struct Counts {
    pub frame: u32,
    pub bullets: u32,
    pub shots: u32,
    pub enemies: u32,
    pub items: u32,
    pub tasks: u32,
    /// 累计（`diag` 是单调计数器，不是当帧值）。
    pub faults: u32,
    pub viol: u32,
}

/// 一个峰值 + 它出现的帧号。
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(crate) struct Peak {
    pub v: u32,
    pub at: u32,
}

impl Peak {
    /// 严格大于才更新 ⇒ `at` 记的是**首次**达到峰值的帧（重演可复现）。
    fn feed(&mut self, v: u32, frame: u32) {
        if v > self.v {
            self.v = v;
            self.at = frame;
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(crate) struct Peaks {
    pub bullets: Peak,
    pub shots: Peak,
    pub enemies: Peak,
    pub items: Peak,
    pub tasks: Peak,
}

/// 一条 `EVT_TASK_FAULT`（`a_index` = 任务池索引，`data = [fault_code, script]`）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct FaultRec {
    pub frame: u32,
    pub task: u16,
    pub code: u8,
    pub script: i32,
}

/// `--at` 那一帧的一颗活弹（池索引升序，I4）。角度同时给 BAM 原值与度数——
/// 前者是引擎里真正的数（用来验"环闭没闭合"要看它），后者给人读。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct BulletRow {
    pub idx: usize,
    pub x_raw: i32,
    pub y_raw: i32,
    pub angle_bam: u16,
    pub speed_raw: i32,
    pub sprite: u16,
}

/// `--at` 那一帧的一只活敌。**弹表之外还给敌表**：教程第 4 步那种"敌在 y=96 上左右
/// 踱步"的断言，光看弹是验不出来的。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct EnemyRow {
    pub idx: usize,
    pub x_raw: i32,
    pub y_raw: i32,
    pub hp: i32,
    pub sprite: u16,
}

/// 一次 `run` 的全部事实。CLI 只负责把它印出来——测试直接吃这个结构。
pub(crate) struct RunReport {
    pub units: Vec<String>,
    pub seed: u64,
    pub rank: i32,
    pub frames: u32,
    pub rows: Vec<Counts>,
    pub last: Counts,
    pub peaks: Peaks,
    pub faults: Vec<FaultRec>,
    pub diag: DiagCounters,
    pub at: Option<(u32, Vec<BulletRow>, Vec<EnemyRow>)>,
}

impl RunReport {
    /// **本刀的核心约定**：有 fault 就退非零码。
    pub fn exit_code(&self) -> u8 {
        if self.faults.is_empty() && self.diag.task_faults == 0 {
            0
        } else {
            1
        }
    }
}

fn popcount(words: &[u64]) -> u32 {
    words.iter().map(|w| w.count_ones()).sum()
}

/// 当下世界的活体计数（不 step、无副作用）。
fn observe(w: &World) -> Counts {
    let v = w.view();
    let d = v.diag();
    Counts {
        frame: w.frame(),
        bullets: popcount(v.bullets().alive_words()),
        shots: popcount(v.shots().alive_words()),
        enemies: popcount(v.enemies().alive_words()),
        items: popcount(v.items().alive_words()),
        tasks: w.tasks().iter_alive().count() as u32,
        faults: d.task_faults,
        viol: d.contract_viol,
    }
}

/// 摘当帧全部活弹 + 活敌（各按池索引升序，I4）。
fn dump_at(w: &World) -> (Vec<BulletRow>, Vec<EnemyRow>) {
    let v = w.view();
    let b = v.bullets();
    let bullets = b
        .iter_alive()
        .map(|i| BulletRow {
            idx: i,
            x_raw: b.x()[i].raw(),
            y_raw: b.y()[i].raw(),
            angle_bam: b.angle()[i].raw(),
            speed_raw: b.speed()[i].raw(),
            sprite: b.sprite()[i],
        })
        .collect();
    let e = v.enemies();
    let enemies = e
        .iter_alive()
        .map(|i| EnemyRow {
            idx: i,
            x_raw: e.x()[i].raw(),
            y_raw: e.y()[i].raw(),
            hp: e.hp()[i],
            sprite: e.sprite()[i],
        })
        .collect();
    (bullets, enemies)
}

// ── 建世界（run / serve --ecl / dump --ecl 共用）─────────────────────────────

/// 路径（文件或目录）→ 编好的镜像 + 正典开机的世界。
///
/// 路径处理照抄 `cmd_check`（`collect_units` + `compile_units`），单文件与目录都收。
/// 开机走 `World::new_game(seed, rank, &image)`——它就是「`World::new`、写 `GVAR_RANK`、
/// `start_main`」三步的正典封装，也正是 `stg-godot` 桥用的那条路（`boot.rs`），故这里跑到
/// 的行为与真 Godot 工程开机同源。
///
/// **不建 Rust 侧 boss**（与 `build_rainbow_world` 分道）：教程里的脚本自己
/// `spawn_enemy`，这条空场路径才是作者的真实处境。
///
/// 错误一律成"已渲染好的多行文本"，调用方直接打——诊断不吞。
pub(crate) fn build_ecl_world(
    path: &str,
    seed: u64,
    rank: i32,
) -> Result<(Box<World>, EclImage, Vec<String>), String> {
    let units = match crate::collect_units(path) {
        Ok(u) if !u.is_empty() => u,
        Ok(_) => return Err(format!("error: {path} 目录下没有 .ecl 文件")),
        Err(e) => return Err(format!("error: 读 {path} 失败: {e}")),
    };
    let names: Vec<String> = units.iter().map(|(n, _)| n.clone()).collect();
    let image = match stg_ecl_compiler::lang::compile_units(&units) {
        Ok(img) => img,
        Err(errors) => {
            let rendered: Vec<String> = errors.iter().map(|(f, e)| e.render(f)).collect();
            return Err(rendered.join("\n\n"));
        }
    };
    let world = World::new_game(seed, rank, &image)
        .map_err(|e| format!("error: 开机失败（World::new_game）：{e:?}"))?;
    Ok((world, image, names))
}

// ── 跑 ─────────────────────────────────────────────────────────────────────

/// 逐帧推进 `frames` 步，采集事实。输入恒空（不按任何键）——`run` 观测的是**脚本**
/// 干了什么，自机火力混进来只会污染弹数。
pub(crate) fn run_scene(
    w: &mut World,
    image: &EclImage,
    frames: u32,
    at: Option<u32>,
    seed: u64,
    rank: i32,
    units: Vec<String>,
) -> RunReport {
    let stride = (frames / SAMPLE_ROWS).max(1);
    let mut rows: Vec<Counts> = Vec::new();
    let mut peaks = Peaks::default();
    let mut faults: Vec<FaultRec> = Vec::new();
    let mut at_dump: Option<(u32, Vec<BulletRow>, Vec<EnemyRow>)> = None;

    // 帧号去重：`--frames` 恰好是 stride 整数倍时末帧会被推两次。
    let push = |rows: &mut Vec<Counts>, c: Counts| {
        if rows.last().map(|l| l.frame) != Some(c.frame) {
            rows.push(c);
        }
    };

    let c0 = observe(w);
    push(&mut rows, c0);
    if at == Some(0) {
        let (b, e) = dump_at(w);
        at_dump = Some((0, b, e));
    }

    for _ in 0..frames {
        let input = InputFrame::empty(w.frame());
        stg_core::step(w, &stg_core::tables::TABLES_V0, image, &input);
        let f = w.frame(); // 帧 N = 第 N 次 step 之后（模块文档口径）

        // fault 事件当帧就得摘走——`events` 是帧内私有缓冲，下一帧 begin 清空。
        for ev in w.frame_events() {
            if ev.kind == stg_core::events::EVT_TASK_FAULT {
                faults.push(FaultRec {
                    frame: f,
                    task: ev.a_index,
                    code: ev.data[0] as u8,
                    script: ev.data[1],
                });
            }
        }

        let c = observe(w);
        peaks.bullets.feed(c.bullets, f);
        peaks.shots.feed(c.shots, f);
        peaks.enemies.feed(c.enemies, f);
        peaks.items.feed(c.items, f);
        peaks.tasks.feed(c.tasks, f);
        if f.is_multiple_of(stride) || f == frames {
            push(&mut rows, c);
        }
        if at == Some(f) {
            let (b, e) = dump_at(w);
            at_dump = Some((f, b, e));
        }
    }

    let last = observe(w);
    RunReport {
        units,
        seed,
        rank,
        frames,
        rows,
        last,
        peaks,
        faults,
        diag: w.view().diag(),
        at: at_dump,
    }
}

// ── 打印 ───────────────────────────────────────────────────────────────────

/// Q16.16 → 十进制字符串（仅打印用；断层线以上，浮点不喂回世界）。
fn fx(raw: i32, digits: usize) -> String {
    format!("{:.*}", digits, raw as f64 / 65536.0)
}

/// BAM u16 → 度（一圈 65536）。
fn deg(bam: u16) -> String {
    format!("{:.2}", bam as f64 * 360.0 / 65536.0)
}

fn print_report(path: &str, r: &RunReport) {
    // 单文件时单元名就是路径本身（`collect_units` 的诊断名），重复打没意义。
    if r.units.len() == 1 {
        println!("run: {path}");
    } else {
        println!(
            "run: {path} —— {} 个单元（按名排序）[{}]",
            r.units.len(),
            r.units.join(", ")
        );
    }
    println!(
        "     seed {} · rank {} · {} 帧 · 无输入（不按键）",
        r.seed, r.rank, r.frames
    );
    println!("     帧 N = 第 N 次 step 之后；帧 0 = 未 step 的开局");
    println!();
    println!(
        "{:>7} {:>7} {:>6} {:>6} {:>6} {:>6} {:>7} {:>6}",
        "frame", "bullet", "shot", "enemy", "item", "task", "fault", "viol"
    );
    for c in &r.rows {
        println!(
            "{:>7} {:>7} {:>6} {:>6} {:>6} {:>6} {:>7} {:>6}",
            c.frame, c.bullets, c.shots, c.enemies, c.items, c.tasks, c.faults, c.viol
        );
    }
    println!();
    println!(
        "峰值：弹 {}（帧 {}）· 敌 {}（帧 {}）· 任务 {}（帧 {}）· 道具 {}（帧 {}）· 自机弹 {}（帧 {}）",
        r.peaks.bullets.v,
        r.peaks.bullets.at,
        r.peaks.enemies.v,
        r.peaks.enemies.at,
        r.peaks.tasks.v,
        r.peaks.tasks.at,
        r.peaks.items.v,
        r.peaks.items.at,
        r.peaks.shots.v,
        r.peaks.shots.at,
    );
    println!(
        "末帧 {}：弹 {} · 敌 {} · 任务 {} · 道具 {} · 自机弹 {}",
        r.last.frame, r.last.bullets, r.last.enemies, r.last.tasks, r.last.items, r.last.shots
    );

    // 诊断：非零的才列，零的一行带过——好让"有东西不对"一眼跳出来。
    let d = &r.diag;
    let pool_full: u32 = d.pool_full.iter().sum();
    println!(
        "诊断：task_faults {} · contract_viol {} · pool_full {} · hits_ovf {} · events_ovf {} · reqs_dropped {}",
        d.task_faults,
        d.contract_viol,
        pool_full,
        d.hits_overflow,
        d.events_overflow,
        d.reqs_dropped
    );
    if d.contract_viol > 0 {
        println!(
            "⚠ contract_viol {} —— 有调用违约被引擎确定性地吞成 no-op（坏参数/悬垂句柄）。",
            d.contract_viol
        );
        println!("  不影响退出码（P4-b 是「安全结果」不是崩），但多半是脚本 bug，值得查。");
    }
    if pool_full > 0 {
        println!(
            "⚠ pool_full {pool_full} —— 有池被打满、分配被确定性降级（P4-a）。弹/敌/任务超预算了。"
        );
    }

    if let Some((f, bullets, enemies)) = &r.at {
        println!();
        println!("帧 {f} · 活敌 {} 只（池索引升序）", enemies.len());
        println!(
            "{:>6} {:>10} {:>10} {:>8} {:>7}",
            "idx", "x", "y", "hp", "sprite"
        );
        for e in enemies.iter().take(AT_DUMP_LIMIT) {
            println!(
                "{:>6} {:>10} {:>10} {:>8} {:>7}",
                e.idx,
                fx(e.x_raw, 2),
                fx(e.y_raw, 2),
                e.hp,
                e.sprite
            );
        }
        println!();
        println!("帧 {f} · 活弹 {} 条（池索引升序）", bullets.len());
        println!(
            "{:>6} {:>10} {:>10} {:>8} {:>9} {:>9} {:>7}",
            "idx", "x", "y", "angleBAM", "deg", "speed", "sprite"
        );
        for b in bullets.iter().take(AT_DUMP_LIMIT) {
            println!(
                "{:>6} {:>10} {:>10} {:>8} {:>9} {:>9} {:>7}",
                b.idx,
                fx(b.x_raw, 2),
                fx(b.y_raw, 2),
                b.angle_bam,
                deg(b.angle_bam),
                fx(b.speed_raw, 3),
                b.sprite
            );
        }
        if bullets.len() > AT_DUMP_LIMIT {
            println!(
                "  ……另 {} 条已略（--at 打印上限 {AT_DUMP_LIMIT}）",
                bullets.len() - AT_DUMP_LIMIT
            );
        }
    }
}

/// fault 打印上限——一次全池死循环能一帧产出 256 条，全打没意义。
const FAULT_PRINT_LIMIT: usize = 20;

fn print_faults(r: &RunReport) {
    if r.faults.is_empty() && r.diag.task_faults == 0 {
        return;
    }
    eprintln!();
    eprintln!("✘ task fault ×{}（脚本被引擎杀了）", r.diag.task_faults);
    for f in r.faults.iter().take(FAULT_PRINT_LIMIT) {
        eprintln!(
            "  帧 {:<5} 任务 #{:<4} script {:<4} code {} {}",
            f.frame,
            f.task,
            f.script,
            f.code,
            fault_name(f.code)
        );
    }
    if r.faults.len() > FAULT_PRINT_LIMIT {
        eprintln!("  ……另 {} 条已略", r.faults.len() - FAULT_PRINT_LIMIT);
    }
    if r.faults.len() < r.diag.task_faults as usize {
        // events 缓冲满会丢事件；计数器不会丢。两者对不上就说明这里少打了几条。
        eprintln!(
            "  （事件缓冲只捞到 {} 条，计数器说有 {} 条——events 溢出了）",
            r.faults.len(),
            r.diag.task_faults
        );
    }
}

// ── CLI ────────────────────────────────────────────────────────────────────

/// `run <file.ecl|目录> [--frames N] [--seed S] [--rank R] [--at F]`
pub(crate) fn cmd_run(rest: &[String]) -> ExitCode {
    ExitCode::from(run_cli(rest))
}

/// 真正的入口，返回**裸退出码**——判别式测试直接断言这个 u8（`ExitCode` 不可比）。
pub(crate) fn run_cli(rest: &[String]) -> u8 {
    let Some(path) = rest.first().filter(|a| !a.starts_with("--")) else {
        eprintln!(
            "usage: stg-harness run <file.ecl|目录> [--frames N] [--seed S] [--rank R] [--at F]"
        );
        return 2;
    };
    let path = path.clone();
    let mut frames: u32 = 600;
    let mut seed: u64 = 1;
    let mut rank: i32 = 2;
    let mut at: Option<u32> = None;
    let mut i = 1;
    while i < rest.len() {
        let parse = |v: &String, what: &str| -> Result<i64, u8> {
            v.parse::<i64>().map_err(|_| {
                eprintln!("run: {what} 要整数，收到 {v}");
                2u8
            })
        };
        match (rest[i].as_str(), rest.get(i + 1)) {
            ("--frames", Some(v)) => {
                frames = match parse(v, "--frames") {
                    Ok(n) if (0..=u32::MAX as i64).contains(&n) => n as u32,
                    Ok(_) => {
                        eprintln!("run: --frames 越界");
                        return 2;
                    }
                    Err(c) => return c,
                };
                i += 2;
            }
            ("--seed", Some(v)) => {
                seed = match v.parse::<u64>() {
                    Ok(s) => s,
                    Err(_) => {
                        eprintln!("run: --seed 要 u64，收到 {v}");
                        return 2;
                    }
                };
                i += 2;
            }
            ("--rank", Some(v)) => {
                rank = match parse(v, "--rank") {
                    Ok(n) => n as i32,
                    Err(c) => return c,
                };
                i += 2;
            }
            ("--at", Some(v)) => {
                at = match parse(v, "--at") {
                    Ok(n) if (0..=u32::MAX as i64).contains(&n) => Some(n as u32),
                    Ok(_) => {
                        eprintln!("run: --at 越界");
                        return 2;
                    }
                    Err(c) => return c,
                };
                i += 2;
            }
            (a, _) => {
                eprintln!("run: 未知参数 {a}（支持 --frames/--seed/--rank/--at）");
                return 2;
            }
        }
    }
    if let Some(f) = at
        && f > frames
    {
        eprintln!(
            "run: --at {f} 超出 --frames {frames}（帧 N = 第 N 次 step 之后，最大 {frames}）"
        );
        return 2;
    }

    let (mut w, image, units) = match build_ecl_world(&path, seed, rank) {
        Ok(t) => t,
        Err(msg) => {
            eprintln!("{msg}");
            return 2;
        }
    };
    let report = run_scene(&mut w, &image, frames, at, seed, rank, units);
    print_report(&path, &report);
    print_faults(&report);
    report.exit_code()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 把源码写进临时 .ecl 并返回路径（唯一名：pid + 计数器，测试并行也不撞）。
    fn tmp_ecl(tag: &str, src: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("stg-run-{}-{}-{}", std::process::id(), tag, n));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.ecl");
        std::fs::write(&p, src).unwrap();
        p
    }

    fn run_src(src: &str, frames: u32, at: Option<u32>) -> RunReport {
        let p = tmp_ecl("rep", src);
        let (mut w, image, units) = build_ecl_world(p.to_str().unwrap(), 1, 2).expect("应编过");
        run_scene(&mut w, &image, frames, at, 1, 2, units)
    }

    const EMPTY: &str = "sub main() { loop { wait(1); } }";

    /// 空关卡（教程第 1 步）：零弹零敌、任务恒 1（main 自己）、零 fault。
    #[test]
    fn empty_stage_is_quiet() {
        let r = run_src(EMPTY, 120, None);
        assert_eq!(r.last.bullets, 0);
        assert_eq!(r.last.enemies, 0);
        assert_eq!(r.last.tasks, 1, "只有 main 一条任务");
        assert_eq!(r.faults.len(), 0);
        assert_eq!(r.exit_code(), 0);
        assert_eq!(r.last.frame, 120, "帧 N = 第 N 次 step 之后");
    }

    /// **判别式①：计数真的来自世界，不是常数。** 一只敌 vs 空场必须不同。
    #[test]
    fn enemy_count_discriminates() {
        let one = run_src(
            "sub main() { _ = spawn_enemy(0.0fx, 96.0fx, 500, 1, 1000, 1, none); loop { wait(1); } }",
            60,
            None,
        );
        assert_eq!(one.last.enemies, 1, "教程第 2 步：一只敌，不动不消失");
        assert_eq!(run_src(EMPTY, 60, None).last.enemies, 0);
    }

    /// **判别式②（本刀最要紧的一格）：故意 fault 的脚本必须退非零码、且报出 fault。**
    ///
    /// `loop { wait(0); }`——`wait(0)` 不让出，一帧之内烧穿任务 1024 条指令预算 →
    /// `FAULT_BUDGET(3)`。变异检验：把 `run_cli` 的返回改成恒 0、或把 `EVT_TASK_FAULT`
    /// 那段摘事件的代码删掉，本测试即红（见 report 的变异实证一节）。
    #[test]
    fn budget_fault_is_loud_and_nonzero_exit() {
        let src = "sub main() { loop { wait(0); } }";
        let r = run_src(src, 30, None);
        assert!(!r.faults.is_empty(), "死循环必须报出 fault，不能静默");
        assert_eq!(r.faults[0].code, 3, "FAULT_BUDGET");
        assert_eq!(r.diag.task_faults, r.faults.len() as u32);
        assert_ne!(r.exit_code(), 0, "有 fault 必须退非零码");

        // 端到端押运 CLI 出口（不只是 report 结构）
        let p = tmp_ecl("fault", src);
        let code = run_cli(&[
            p.to_str().unwrap().to_string(),
            "--frames".into(),
            "30".into(),
        ]);
        assert_eq!(code, 1, "run 的进程退出码必须非零");
        // 对照：健康脚本走同一条 CLI 路径退 0（判别 "恒 1" 的实现）
        let ok = tmp_ecl("okay", EMPTY);
        assert_eq!(
            run_cli(&[
                ok.to_str().unwrap().to_string(),
                "--frames".into(),
                "30".into()
            ]),
            0
        );
    }

    /// **判别式③：`--at` 的弹表真反映角度/速度分布。** 教程第 6 步那个 16 路整周环：
    /// 首波 16×2=32 颗，角度必须是 16 个 4096 的整数倍、每个角度两个速度层。
    /// 圆心重合式断言（"有 32 条"）对角度映射是瞎的，故这里逐值比对。
    #[test]
    fn at_dump_shows_ring_geometry() {
        let src = r#"
const BALL: int = 48;
async sub shoot() {
    sh_reset(0);
    sh_sprite(0, BALL, 2);
    sh_ring(0, 1);
    sh_count(0, 16, 2);
    sh_speed(0, 1.2fx, 0.5fx);
    loop { sh_fire(0); wait(600); }
}
sub main() {
    _ = spawn_enemy(0.0fx, 96.0fx, 500, 1, 1000, 1, shoot);
    loop { wait(1); }
}
"#;
        // 敌出生当帧不跑，次帧首跑 → 帧 2 已开完第一波。
        let r = run_src(src, 3, Some(3));
        let (_f, rows, enemies) = r.at.as_ref().expect("--at 应有快照");
        assert_eq!(enemies.len(), 1, "敌表也在（教程第 4 步的走位靠它验）");
        assert_eq!(rows.len(), 32, "16 路 × 2 层 = 32 颗");

        let mut angles: Vec<u16> = rows.iter().map(|b| b.angle_bam).collect();
        angles.sort_unstable();
        angles.dedup();
        assert_eq!(angles.len(), 16, "16 个互异角度");
        for (k, a) in angles.iter().enumerate() {
            assert_eq!(*a as u32, k as u32 * 4096, "整周均分：第 {k} 个角 = k×4096");
        }
        let mut speeds: Vec<i32> = rows.iter().map(|b| b.speed_raw).collect();
        speeds.sort_unstable();
        speeds.dedup();
        assert_eq!(speeds.len(), 2, "两层速度");
        assert_eq!(speeds[0], (1.2f64 * 65536.0) as i32);
        assert_eq!(speeds[1], ((1.2 + 0.5) * 65536.0) as i32);
        assert!(
            rows.iter().all(|b| b.sprite != 0),
            "sprite 应是内容包给的值"
        );
        // 池索引升序（I4）
        assert!(rows.windows(2).all(|w| w[0].idx < w[1].idx));
    }

    /// 目录整取（多文件编译单元）走得通——与 `check` 同一条路径。
    #[test]
    fn directory_units_are_collected() {
        let dir = tmp_ecl("dir", "async sub helper() { loop { wait(1); } }")
            .parent()
            .unwrap()
            .to_path_buf();
        std::fs::write(
            dir.join("z_main.ecl"),
            "sub main() { spawn helper(); loop { wait(1); } }",
        )
        .unwrap();
        let (mut w, image, units) = build_ecl_world(dir.to_str().unwrap(), 1, 2).expect("应编过");
        assert_eq!(units.len(), 2, "两个单元都被收进来");
        let r = run_scene(&mut w, &image, 10, None, 1, 2, units);
        assert_eq!(r.last.tasks, 2, "main + spawn 出来的 helper");
        assert_eq!(r.exit_code(), 0);
    }

    /// 参数校验：坏路径 / 坏参数 / `--at` 越界都退 2（与 `check` 的 usage 错同码）。
    #[test]
    fn cli_argument_errors_exit_two() {
        assert_eq!(run_cli(&[]), 2, "缺路径");
        assert_eq!(run_cli(&["/nonexistent/nope.ecl".into()]), 2);
        let p = tmp_ecl("args", EMPTY);
        let s = p.to_str().unwrap().to_string();
        assert_eq!(run_cli(&[s.clone(), "--frames".into(), "x".into()]), 2);
        assert_eq!(run_cli(&[s.clone(), "--wat".into(), "1".into()]), 2);
        assert_eq!(
            run_cli(&[
                s.clone(),
                "--frames".into(),
                "5".into(),
                "--at".into(),
                "9".into()
            ]),
            2,
            "--at 超出 --frames"
        );
        // rank 越界由核里的 new_game 拒（不钳位）——开机失败也退 2
        assert_eq!(run_cli(&[s, "--rank".into(), "9".into()]), 2);
    }

    /// 编译错误走渲染诊断、退 2（不 panic、不静默）。
    #[test]
    fn compile_error_is_rendered_not_panic() {
        let p = tmp_ecl("bad", "sub main() { int x = 5; }");
        let e = build_ecl_world(p.to_str().unwrap(), 1, 2).unwrap_err();
        assert!(e.contains(":"), "应是 文件:行:列 形态的渲染诊断，实得：{e}");
        assert_eq!(run_cli(&[p.to_str().unwrap().to_string()]), 2);
    }

    /// 采样行口径：含帧 0 与末帧，帧号严格升序不重复。
    #[test]
    fn sample_rows_cover_head_and_tail() {
        let r = run_src(EMPTY, 95, None);
        assert_eq!(r.rows.first().unwrap().frame, 0);
        assert_eq!(r.rows.last().unwrap().frame, 95, "末帧必在");
        assert!(r.rows.windows(2).all(|w| w[0].frame < w[1].frame));
    }

    /// `--frames 0` 不空转假绿：只出开局一行，帧号 0。
    #[test]
    fn zero_frames_reports_opening_only() {
        let r = run_src(EMPTY, 0, None);
        assert_eq!(r.rows.len(), 1);
        assert_eq!(r.last.frame, 0);
        assert_eq!(r.exit_code(), 0);
    }

    /// 峰值是**全程**峰值，不是采样行的最大值——弹在两次采样之间起落也要抓到。
    #[test]
    fn peak_is_over_all_frames_not_just_samples() {
        // 每 7 帧一颗、寿命短：峰值必然落在非采样帧上
        let src = r#"
async sub shoot() { loop { _ = fire(48, 2, $self_x, $self_y, 3.0fx, 90deg, none, none); wait(7); } }
sub main() { _ = spawn_enemy(0.0fx, 96.0fx, 500, 1, 1000, 1, shoot); loop { wait(1); } }
"#;
        let r = run_src(src, 200, None);
        let sample_max = r.rows.iter().map(|c| c.bullets).max().unwrap();
        assert!(r.peaks.bullets.v >= sample_max);
        assert!(r.peaks.bullets.v > 0, "总得有弹");
    }

    /// F9：core 的 `FAULT_NAMES` 是唯一真相源，本文件的中文解释必须**逐码跟满**。
    ///
    /// core 加了新 fault 码（`FAULT_NAMES` 长度 +1）而这边忘了加解释时转红。
    /// **判别力**：删掉 `fault_name` 里任意一条 `hint` 分支立刻红；而"短名来自 core"
    /// 那半边则由第二条断言押住——若有人把 `short` 改回硬编码字面量，core 侧改名后转红。
    #[test]
    fn every_core_fault_code_has_a_chinese_hint() {
        for code in 0..stg_core::ecl::FAULT_NAMES.len() {
            let rendered = fault_name(code as u8);
            let short = stg_core::ecl::FAULT_NAMES[code];
            assert!(
                rendered.starts_with(short),
                "码 {code} 的渲染 {rendered:?} 没有以 core 的短名 {short:?} 打头——\
                 短名的真相源是 core，不该在 harness 侧另写一份"
            );
            assert!(
                rendered.len() > short.len(),
                "码 {code}（{short}）在 harness 侧没有中文解释——core 新增了 fault 码，\
                 这边的 hint 分支要跟一行"
            );
        }
        // 越界码不 panic、退化成短名占位（不是"未知 fault 码"那种会漂的措辞）
        assert_eq!(fault_name(200), "?");
    }
}
