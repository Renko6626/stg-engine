//! 批量 env（spec §3.1 / §7）：专属 rayon 池、不相交 `&mut` 切片并行、CSR 压实。
//!
//! 观测写入按 env 分片并行（每 env 写自己的 `Slot` 暂存区与调用方缓冲中自己那一段）。
//!
//! **偏离 spec §7 的说明（2026-09-15，控制者裁定）**：spec §7 原写「CSR 单线程压实」，
//! 故把 `compact` 拆成两阶段：这是**结构性移除最坏 `O(N*cap)` 的串行段**；在默认 rl-bench
//! （每 env ~12.5 弹）下测不出收益，弹满 `cap` 的密弹 workload 才兑现。吞吐平台的实际主因
//! 是**已修复的 `BootCache` 锁**（锁内 alloc+copy 串行化 reset），见 `docs/bench-baseline.md`。
//!
//! 1. **串行前缀和**：按 env 索引序累计 `nb/ni` 并写入 `*_offsets`——这是唯一决定
//!    「哪个 env 的行落在哪个 CSR 区间」的地方，只依赖各 `Slot` 的数据，与线程数/调度无关；
//! 2. **并行 scatter**：用 `split_at_mut` 链把目标缓冲按 offsets 切成 N 个**互不相交**的片，
//!    与各 `Slot` 的暂存区一一配对后，在专属池里 `copy_from_slice`。
//!
//! 确定性依据：目标区间由阶段 1 的串行前缀和冻结，各分片按 env 索引序一一对应、互不重叠，
//! 故最终字节只由 env 索引序决定 ⇒ 与线程数、调度无关。
//! `tests/vec_env.rs::deterministic_across_thread_counts`（1 vs 8 线程逐步 digest）与
//! `compacted_rows_match_direct_encode`（压实字节 vs 直接编码）共同押运这一点。
//!
//! 断层线以上（`stg-rl`）：允许 rayon / 浮点。`Env` 的随机性全部来自 `episode_seed`，
//! 多线程不引入任何共享可变状态。

use crate::encode::{self, BulletStats};
use crate::env::{BootCache, EVENTS, Env, EnvConfig, validate};
use crate::layout::{ENEMIES_CAP, ITEMS_CAP};
use rayon::prelude::*;
use std::sync::Arc;
use stg_core::tables::TABLES_V0;

/// `buffer_sizes` 的各字段长度（调用方按此分配，`check` 逐字段核对）。
pub struct BufferSizes {
    pub frame: usize,
    pub phase: usize,
    pub player: usize,
    pub enemies: usize,
    pub enemies_count: usize,
    pub bullets: usize,
    pub bullets_offsets: usize,
    pub items: usize,
    pub items_offsets: usize,
    pub lasers_count: usize,
    pub bullets_total: usize,
    pub bullets_dropped: usize,
    pub events: usize,
    pub done: usize,
    pub ep_frames: usize,
    pub warmup_retries: usize,
    pub start_index: usize,
}

/// 各缓冲应分配的长度（spec §3.1）。`bullets` 按 `cap`，`items` 恒 `ITEMS_CAP`。
pub fn buffer_sizes(n: usize, cap: usize) -> BufferSizes {
    BufferSizes {
        frame: n,
        phase: n,
        player: n * 36,
        enemies: n * ENEMIES_CAP * 38,
        enemies_count: n,
        bullets: n * cap * 30,
        bullets_offsets: n + 1,
        items: n * ITEMS_CAP * 18,
        items_offsets: n + 1,
        lasers_count: n,
        bullets_total: n,
        bullets_dropped: n,
        events: n * EVENTS,
        done: n,
        ep_frames: n,
        warmup_retries: n,
        start_index: n,
    }
}

/// 调用方持有的扁平缓冲视图（一轮 `reset`/`step` 写满；字段名即 `BufferSizes`）。
pub struct BufferSet<'a> {
    pub frame: &'a mut [u32],
    pub phase: &'a mut [u32],
    pub player: &'a mut [u8],
    pub enemies: &'a mut [u8],
    pub enemies_count: &'a mut [i32],
    pub bullets: &'a mut [u8],
    pub bullets_offsets: &'a mut [i32],
    pub items: &'a mut [u8],
    pub items_offsets: &'a mut [i32],
    pub lasers_count: &'a mut [i32],
    pub bullets_total: &'a mut [i32],
    pub bullets_dropped: &'a mut [i32],
    pub events: &'a mut [i32],
    pub done: &'a mut [u8],
    pub ep_frames: &'a mut [i32],
    pub warmup_retries: &'a mut [i32],
    pub start_index: &'a mut [i32],
}

/// 单 env 的私有状态：世界 + 本 env 的定长观测暂存区（CSR 压实前）。
struct Slot {
    env: Env,
    bullets: Vec<u8>,
    items: Vec<u8>,
    sel: Vec<(i64, u16)>,
    nb: usize,
    ni: usize,
}

/// 并行工作项：一个 env 与它在调用方缓冲中独占的各段。
struct Work<'a> {
    slot: &'a mut Slot,
    act: u32,
    player: &'a mut [u8],
    enemies: &'a mut [u8],
    enemies_count: &'a mut i32,
    frame: &'a mut u32,
    phase: &'a mut u32,
    bullets_total: &'a mut i32,
    bullets_dropped: &'a mut i32,
    events: &'a mut [i32],
    done: &'a mut u8,
    ep_frames: &'a mut i32,
    warmup_retries: &'a mut i32,
    start_index: &'a mut i32,
}

/// 批量 env：专属 rayon 池 + N 个独立 `Env`（spec §7）。
pub struct VecEnv {
    slots: Vec<Slot>,
    pool: rayon::ThreadPool,
    cap: usize,
    n_starts: usize,
    min_len: usize,
}

impl VecEnv {
    /// 构造：`validate` 配置 + 建 N 个 env（各自 `reset`）+ 专属线程池。
    ///
    /// 所有 env 共享一份 `Arc<EnvConfig>` 与一份 `Arc<BootCache>`（开机模板缓存）。
    pub fn new(cfg: EnvConfig, num_envs: usize, threads: usize) -> Result<VecEnv, String> {
        validate(&cfg)?;
        if num_envs == 0 {
            return Err("num_envs 必须 > 0".to_string());
        }
        if threads == 0 {
            return Err("threads 必须 > 0".to_string());
        }
        let cap = cfg.bullets_cap;
        let n_starts = cfg.starts.len();
        let cfg = Arc::new(cfg);
        let cache = Arc::new(BootCache::new());
        let slots: Vec<Slot> = (0..num_envs)
            .map(|i| Slot {
                env: Env::new(cfg.clone(), i as u32, cache.clone()),
                bullets: vec![0u8; cap * 30],
                items: vec![0u8; ITEMS_CAP * 18],
                sel: Vec::with_capacity(cap),
                nb: 0,
                ni: 0,
            })
            .collect();
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .thread_name(|i| format!("stg-rl-{i}"))
            .build()
            .map_err(|e| format!("rayon 线程池创建失败: {e}"))?;
        let min_len = num_envs.div_ceil(threads);
        Ok(VecEnv {
            slots,
            pool,
            cap,
            n_starts,
            min_len,
        })
    }

    pub fn num_envs(&self) -> usize {
        self.slots.len()
    }

    pub fn bullets_cap(&self) -> usize {
        self.cap
    }

    /// 逐字段核对缓冲长度（spec §3.1；形状 / dtype 由 Python 壳层的 numpy 视图保证）。
    fn check(&self, buf: &BufferSet<'_>) -> Result<(), String> {
        let want = buffer_sizes(self.slots.len(), self.cap);
        macro_rules! chk {
            ($name:ident) => {
                if buf.$name.len() != want.$name {
                    return Err(format!(
                        "buffer {}: len {} != {}",
                        stringify!($name),
                        buf.$name.len(),
                        want.$name
                    ));
                }
            };
        }
        chk!(frame);
        chk!(phase);
        chk!(player);
        chk!(enemies);
        chk!(enemies_count);
        chk!(bullets);
        chk!(bullets_offsets);
        chk!(items);
        chk!(items_offsets);
        chk!(lasers_count);
        chk!(bullets_total);
        chk!(bullets_dropped);
        chk!(events);
        chk!(done);
        chk!(ep_frames);
        chk!(warmup_retries);
        chk!(start_index);
        Ok(())
    }

    /// 全员新一局：并行 `reset` + 写观测，随后 CSR 压实；统计列清零。
    ///
    /// 构造时 `Env::new` 已 reset 过（构造即 reset）；此处会**再开一局**（每 env 的 counter +1、
    /// 多一次随机预热）。这是既定行为，确定性测试与 rl-bench 基线都依赖现状，不要「优化」掉。
    pub fn reset(&mut self, buf: &mut BufferSet<'_>) -> Result<(), String> {
        self.check(buf)?;
        let pool = &self.pool;
        let min_len = self.min_len;
        let works = build_work(&mut self.slots, buf, |_| 0);
        pool.install(|| {
            works.into_par_iter().with_min_len(min_len).for_each(|w| {
                w.slot.env.reset();
                write_obs(
                    w.slot,
                    w.player,
                    w.enemies,
                    w.enemies_count,
                    w.frame,
                    w.phase,
                    w.bullets_total,
                    w.bullets_dropped,
                );
                w.events.fill(0);
                *w.done = 0;
                *w.ep_frames = 0;
                *w.warmup_retries = 0;
                *w.start_index = 0;
            });
        });
        self.compact(buf);
        Ok(())
    }

    /// 全员一步：并行 `step` + 写观测，随后 CSR 压实。
    ///
    /// `done != 0` 的 env 在其 `Slot` 内已自动 reset（`Env::step`），观测是新局第一帧。
    pub fn step(&mut self, actions: &[u32], buf: &mut BufferSet<'_>) -> Result<(), String> {
        self.check(buf)?;
        if actions.len() != self.slots.len() {
            return Err(format!(
                "actions: len {} != num_envs {}",
                actions.len(),
                self.slots.len()
            ));
        }
        let pool = &self.pool;
        let min_len = self.min_len;
        let works = build_work(&mut self.slots, buf, |i| actions[i]);
        pool.install(|| {
            works.into_par_iter().with_min_len(min_len).for_each(|w| {
                let out = w.slot.env.step(w.act);
                w.events.copy_from_slice(&out.events);
                *w.done = out.done;
                *w.ep_frames = out.ep_frames;
                *w.warmup_retries = out.warmup_retries;
                *w.start_index = out.start_index;
                write_obs(
                    w.slot,
                    w.player,
                    w.enemies,
                    w.enemies_count,
                    w.frame,
                    w.phase,
                    w.bullets_total,
                    w.bullets_dropped,
                );
            });
        });
        self.compact(buf);
        Ok(())
    }

    /// 课程学习：改起点采样权重（长度须等于起点数、全部有限 ≥ 0、总和 > 0；下一次 reset 生效）。
    ///
    /// **运行期口径**：允许单个起点 `weight == 0`（课程学习把某起点置零）；唯一硬约束是总和 > 0。
    /// 与构造期 `validate` 要求每个起点 `weight > 0` 不同。
    pub fn set_start_weights(&mut self, w: Vec<f64>) -> Result<(), String> {
        if w.len() != self.n_starts {
            return Err(format!(
                "weights: len {} != n_starts {}",
                w.len(),
                self.n_starts
            ));
        }
        if w.iter().any(|x| !x.is_finite() || *x < 0.0) {
            return Err("weights 必须为有限且非负".to_string());
        }
        if w.iter().sum::<f64>() <= 0.0 {
            return Err("weights 总和必须 > 0".to_string());
        }
        let w = Arc::new(w);
        for slot in &mut self.slots {
            slot.env.set_weights(w.clone());
        }
        Ok(())
    }

    /// 两阶段 CSR 压实（见模块文档对 spec §7 的偏离说明）：
    ///
    /// 1. 串行按 env 索引序算 bullets/items 前缀和（写入 `*_offsets`）；
    /// 2. 并行 scatter：`split_at_mut` 链把目标缓冲切成 N 个互不相交的片，与各 `Slot` 的
    ///    暂存区配对后在专属池里 `copy_from_slice`。
    ///
    /// 结果字节只由 env 索引序与各 `Slot.nb/ni` 决定，与线程数/调度无关。
    fn compact(&self, buf: &mut BufferSet<'_>) {
        buf.lasers_count.fill(0);

        // 阶段 1（串行）：前缀和。offsets 冻结后，每个 env 的目标区间即确定。
        let n = self.slots.len();
        let mut total_bullets = 0usize;
        buf.bullets_offsets[0] = 0;
        for (i, s) in self.slots.iter().enumerate() {
            total_bullets += s.nb;
            buf.bullets_offsets[i + 1] = total_bullets as i32;
        }
        let mut total_items = 0usize;
        buf.items_offsets[0] = 0;
        for (i, s) in self.slots.iter().enumerate() {
            total_items += s.ni;
            buf.items_offsets[i + 1] = total_items as i32;
        }

        // 阶段 2（并行）：目标分片互不相交。源切片用 `&[u8]`（`Sync`），目标用 `&mut [u8]`。
        let mut rest = &mut buf.bullets[..total_bullets * 30];
        let mut bullet_dst: Vec<&mut [u8]> = Vec::with_capacity(n);
        for s in &self.slots {
            let len = s.nb * 30;
            let (head, tail) = rest.split_at_mut(len);
            bullet_dst.push(head);
            rest = tail;
        }
        let bullet_src: Vec<&[u8]> = self.slots.iter().map(|s| &s.bullets[..s.nb * 30]).collect();

        let mut rest = &mut buf.items[..total_items * 18];
        let mut item_dst: Vec<&mut [u8]> = Vec::with_capacity(n);
        for s in &self.slots {
            let len = s.ni * 18;
            let (head, tail) = rest.split_at_mut(len);
            item_dst.push(head);
            rest = tail;
        }
        let item_src: Vec<&[u8]> = self.slots.iter().map(|s| &s.items[..s.ni * 18]).collect();

        let pool = &self.pool;
        let min_len = self.min_len;
        pool.install(|| {
            bullet_dst
                .into_par_iter()
                .zip(bullet_src)
                .with_min_len(min_len)
                .for_each(|(dst, src)| dst.copy_from_slice(src));
            item_dst
                .into_par_iter()
                .zip(item_src)
                .with_min_len(min_len)
                .for_each(|(dst, src)| dst.copy_from_slice(src));
        });
    }
}

/// 把 `buf` 的每个按-env 字段切成 N 份，与 `slots` 一一配对成并行工作项。
///
/// 所有分片互不相交（每个字段自己的 `chunks_mut` / `iter_mut`），故各 env 的写入无数据竞争。
fn build_work<'a>(
    slots: &'a mut [Slot],
    buf: &'a mut BufferSet<'_>,
    act_of: impl Fn(usize) -> u32,
) -> Vec<Work<'a>> {
    let mut player = buf.player.chunks_mut(36);
    let mut enemies = buf.enemies.chunks_mut(ENEMIES_CAP * 38);
    let mut enemies_count = buf.enemies_count.iter_mut();
    let mut frame = buf.frame.iter_mut();
    let mut phase = buf.phase.iter_mut();
    let mut bullets_total = buf.bullets_total.iter_mut();
    let mut bullets_dropped = buf.bullets_dropped.iter_mut();
    let mut events = buf.events.chunks_mut(EVENTS);
    let mut done = buf.done.iter_mut();
    let mut ep_frames = buf.ep_frames.iter_mut();
    let mut warmup_retries = buf.warmup_retries.iter_mut();
    let mut start_index = buf.start_index.iter_mut();

    slots
        .iter_mut()
        .enumerate()
        .map(|(i, slot)| Work {
            slot,
            act: act_of(i),
            player: player.next().expect("player chunks == num_envs"),
            enemies: enemies.next().expect("enemies chunks == num_envs"),
            enemies_count: enemies_count.next().expect("enemies_count == num_envs"),
            frame: frame.next().expect("frame == num_envs"),
            phase: phase.next().expect("phase == num_envs"),
            bullets_total: bullets_total.next().expect("bullets_total == num_envs"),
            bullets_dropped: bullets_dropped.next().expect("bullets_dropped == num_envs"),
            events: events.next().expect("events == num_envs"),
            done: done.next().expect("done == num_envs"),
            ep_frames: ep_frames.next().expect("ep_frames == num_envs"),
            warmup_retries: warmup_retries.next().expect("warmup_retries == num_envs"),
            start_index: start_index.next().expect("start_index == num_envs"),
        })
        .collect()
}

/// 写一个 env 的全部观测：player / enemies(+count) / phase / frame / bullets(暂存) / items(暂存)。
///
/// 并行闭包内调用，`slot` 与各缓冲段均为该 env 独占。
#[allow(clippy::too_many_arguments)] // 按 BufferSet 字段逐参传入，拆包更难读
fn write_obs(
    slot: &mut Slot,
    player_row: &mut [u8],
    enemies_rows: &mut [u8],
    enemies_count: &mut i32,
    frame: &mut u32,
    phase: &mut u32,
    bullets_total: &mut i32,
    bullets_dropped: &mut i32,
) {
    // 拆字段借用：`env.world()` 只读 env，bullets/items/sel 是另外的字段，互不冲突。
    let Slot {
        env,
        bullets,
        items,
        sel,
        nb,
        ni,
    } = slot;
    let cap = bullets.len() / 30;
    let w = env.world();
    encode::write_player(w, &TABLES_V0, player_row);
    *enemies_count = encode::write_enemies(w, enemies_rows) as i32;
    *phase = encode::phase_bits(w);
    *frame = w.frame();
    let BulletStats {
        count,
        total,
        dropped,
    } = encode::write_bullets(w, cap, bullets, sel);
    *nb = count;
    *bullets_total = total as i32;
    *bullets_dropped = dropped as i32;
    *ni = encode::write_items(w, items);
}
