//! 单 env 生命周期（spec §4/§6）：起点加权采样、种子流、开机模板缓存 + `reseed`、
//! 随机预热重试、events 8 列、done 0–3 与自动 reset。
//!
//! 断层线以上（`stg-rl`）：允许 `Arc`/`HashMap`/浮点。`HashMap` 只做开机模板的键查找，
//! 不参与任何遍历序；浮点只用于起点权重累积与比较（确定性由相同位级运算保证）。
//!
//! 起点采样与预热的全部随机性从 `episode_seed`（`splitmix64` 链）派生 ⇒ 同 `base_seed` +
//! 同动作序列 ⇒ 世界逐位一致，与线程数 / 调度顺序无关（spec §4.1）。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use stg_core::ecl::image::EclImage;
use stg_core::events::{
    EVT_ENEMY_DIED, EVT_ITEM_PICKED, EVT_PHASE_ENDED, EVT_PLAYER_DIED, EVT_SHOT_HIT_ENEMY,
    EVT_SPELL_CAPTURED, EVT_SPELL_FAILED, EVT_STAGE_CLEARED, Event,
};
use stg_core::input::{BTN_DOWN, BTN_LEFT, BTN_RIGHT, BTN_UP, InputFrame};
use stg_core::player::{LIFE_ALIVE, Loadout};
use stg_core::step::{World, step as core_step};
use stg_core::tables::TABLES_V0;

use crate::layout::{ACTION_MASK, BULLETS_CAP_MAX};

/// events 列数（spec §6；列序与 `layout::EVENT_COLUMNS` 一致）。
pub const EVENTS: usize = 8;
/// done 码（spec §4.4）：继续。
pub const DONE_NONE: u8 = 0;
/// done 码：决死窗口耗尽后死亡（bomb 救回不算）。
pub const DONE_DIED: u8 = 1;
/// done 码：本步出现 `end_on` 中任一事件。
pub const DONE_SEGMENT: u8 = 2;
/// done 码：`ep_frames >= max_frames`。
pub const DONE_TIMEOUT: u8 = 3;
/// 预热重试上限（spec §4.2）：耗尽则 k = 0 开局。
pub const MAX_WARMUP_RETRIES: u32 = 8;

/// 编译好的只读镜像（多个 env 共享同一份 `Arc<EclImage>`）。
#[derive(Clone, Debug)]
pub struct Image(pub(crate) Arc<EclImage>);

/// 源码单元集（文件名 + 文本）→ 镜像。错误 = 各文件 `CompileError::render` 拼接。
pub fn compile(units: &[(String, String)]) -> Result<Image, String> {
    let image = stg_ecl_compiler::lang::compile_units(units).map_err(|errs| {
        errs.iter()
            .map(|(file, e)| e.render(file))
            .collect::<Vec<_>>()
            .join("\n")
    })?;
    Ok(Image(Arc::new(image)))
}

/// 一个采样起点：`image` = `EnvConfig.images` 下标，`mark` = 中段启动标记（0 = 从头）。
#[derive(Clone)]
pub struct Start {
    pub image: usize,
    pub mark: i32,
    pub rank: i32,
    pub weight: f64,
}

/// 段落结束判据（spec §4.4 done=2）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndOn {
    PhaseEnded,
    SpellCaptured,
    SpellFailed,
    StageCleared,
}

/// 单个 env 的静态配置（`Arc` 共享）。
#[derive(Clone)]
pub struct EnvConfig {
    pub images: Vec<Image>,
    pub starts: Vec<Start>,
    pub frame_skip: u32,
    pub max_frames: u32,
    pub warmup_max: u32,
    pub end_on: Vec<EndOn>,
    pub bullets_cap: usize,
    pub seed: u64,
}

/// 构造期配置校验（spec §3 的 Python 侧错误在构造期抛，不在 step 里抛）。
///
/// `mark == 0`（从头开局）例外——它不查镜像标记表。
pub fn validate(cfg: &EnvConfig) -> Result<(), String> {
    if cfg.starts.is_empty() {
        return Err("starts 不能为空".to_string());
    }
    if cfg.frame_skip == 0 {
        return Err("frame_skip 必须 > 0".to_string());
    }
    if cfg.max_frames == 0 {
        return Err("max_frames 必须 > 0".to_string());
    }
    if cfg.bullets_cap == 0 || cfg.bullets_cap > BULLETS_CAP_MAX {
        return Err(format!(
            "bullets_cap {} 越界（合法 1..={BULLETS_CAP_MAX}）",
            cfg.bullets_cap
        ));
    }
    for (i, s) in cfg.starts.iter().enumerate() {
        if !s.weight.is_finite() || s.weight <= 0.0 {
            return Err(format!(
                "start {i} weight 必须为正有限值（got {}）",
                s.weight
            ));
        }
        let image = cfg.images.get(s.image).ok_or_else(|| {
            format!(
                "start {i} image 下标 {} 越界（共 {}）",
                s.image,
                cfg.images.len()
            )
        })?;
        if !(stg_core::consts::RANK_EASY..=stg_core::consts::RANK_EXTRA).contains(&s.rank) {
            return Err(format!("start {i} rank {} 越界", s.rank));
        }
        if s.mark != 0 && image.0.resolve_mark(s.mark).is_none() {
            return Err(format!("start {i} mark {} 在镜像标记表中查无", s.mark));
        }
    }
    Ok(())
}

/// 一次 `step` 的产出（spec §6；`done != 0` 时统计的是刚结束那局）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StepOut {
    pub events: [i32; EVENTS],
    pub done: u8,
    pub ep_frames: i32,
    pub warmup_retries: i32,
    pub start_index: i32,
}

/// splitmix64 终混（spec §4.1 种子流；常量钉死）。
pub fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}
const K1: u64 = 0xD1B5_4A32_D192_ED03;
const K2: u64 = 0xABC9_8388_FB8F_AC03;

/// 随机游走方向表（spec §4.2）：9 方向，不按 SHOT/BOMB。
const WALK_DIRS: [u32; 9] = [
    0,
    BTN_UP,
    BTN_UP | BTN_RIGHT,
    BTN_RIGHT,
    BTN_DOWN | BTN_RIGHT,
    BTN_DOWN,
    BTN_DOWN | BTN_LEFT,
    BTN_LEFT,
    BTN_UP | BTN_LEFT,
];

/// 四个段落结束事件 → (segment_end 列值, EndOn)。其余事件返回 `None`。
fn segment_end(kind: u8) -> Option<(i32, EndOn)> {
    match kind {
        EVT_PHASE_ENDED => Some((1, EndOn::PhaseEnded)),
        EVT_SPELL_CAPTURED => Some((2, EndOn::SpellCaptured)),
        EVT_SPELL_FAILED => Some((3, EndOn::SpellFailed)),
        EVT_STAGE_CLEARED => Some((4, EndOn::StageCleared)),
        _ => None,
    }
}

fn has_segment_end(events: &[Event]) -> bool {
    events.iter().any(|e| segment_end(e.kind).is_some())
}

/// 预热随机游走（spec §4.2）：跑 `k` 帧，中途死亡 / 段落结束返回 `false`（触发重试）。
fn warmup_walk(world: &mut World, image: &EclImage, seed: u64, k: u64) -> bool {
    let mut r = splitmix64(seed ^ 0xB7);
    let mut left = k;
    while left > 0 {
        let dir = WALK_DIRS[(r % WALK_DIRS.len() as u64) as usize];
        let seg = 8 + (r % 25);
        r = splitmix64(r);
        let mut in_seg = 0;
        while in_seg < seg && left > 0 {
            let mut input = InputFrame::empty(world.frame());
            input.actions[0].buttons = dir;
            core_step(world, &TABLES_V0, image, &input);
            left -= 1;
            in_seg += 1;
            if world.view().players()[0].life_state != LIFE_ALIVE
                || has_segment_end(world.frame_events())
            {
                return false;
            }
        }
    }
    true
}

/// 开机模板缓存（spec §4.2）：键 = (镜像, mark, rank)，首次用到时 `new_game_at(0,…)` 生成
/// 并缓存，之后 `copy_into` 出新世界再 `reseed`。`reseed` 等价已在 Task 1 验证为绿。
///
/// `HashMap` 只做键查找、不参与遍历序（断层线以上允许）。
pub struct BootCache {
    templates: Mutex<HashMap<(usize, i32, i32), Box<World>>>,
}

impl Default for BootCache {
    fn default() -> Self {
        Self::new()
    }
}

impl BootCache {
    pub fn new() -> BootCache {
        BootCache {
            templates: Mutex::new(HashMap::new()),
        }
    }

    /// 已缓存的模板数（测试押运复用）。
    pub fn len(&self) -> usize {
        self.templates.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 取该起点的模板副本；未命中则用 `new_game_at(0, rank, mark, Loadout{character:1,..},
    /// image)` 生成并缓存。调用方须已 `validate`（rank/mark/image 合法），否则 panic。
    fn template(&self, cfg: &EnvConfig, start: &Start) -> Box<World> {
        let key = (start.image, start.mark, start.rank);
        let mut map = self.templates.lock().unwrap();
        let src = map.entry(key).or_insert_with(|| {
            World::new_game_at(
                0,
                start.rank,
                start.mark,
                Loadout {
                    character: 1,
                    ..Default::default()
                },
                cfg.images[start.image].0.as_ref(),
            )
            .expect("new_game_at 失败：EnvConfig 未通过 validate()")
        });
        let mut out = World::new(0);
        src.copy_into(&mut out);
        out
    }
}

/// 单个 env（spec §4）：起点、种子流、快照、预热、events、done 与自动 reset。
pub struct Env {
    cfg: Arc<EnvConfig>,
    weights: Arc<Vec<f64>>,
    cache: Arc<BootCache>,
    env_index: u32,
    counter: u64,
    world: Box<World>,
    start_index: usize,
    ep_frames: u32,
    warmup_retries: u32,
    prev_graze: u32,
    prev_score: u64,
    prev_bombs: u8,
}

impl Env {
    /// `env_index` 进种子流（同 config 的不同 env 各自独立）。构造即 reset。
    pub fn new(cfg: Arc<EnvConfig>, env_index: u32, cache: Arc<BootCache>) -> Env {
        let weights = Arc::new(cfg.starts.iter().map(|s| s.weight).collect::<Vec<f64>>());
        let mut env = Env {
            cfg,
            weights,
            cache,
            env_index,
            counter: 0,
            world: World::new(0),
            start_index: 0,
            ep_frames: 0,
            warmup_retries: 0,
            prev_graze: 0,
            prev_score: 0,
            prev_bombs: 0,
        };
        env.reset();
        env
    }

    /// 课程学习：改起点采样权重（下一次 `reset` 生效；长度须等于起点数，由调用方校验）。
    pub fn set_weights(&mut self, w: Arc<Vec<f64>>) {
        self.weights = w;
    }

    /// 只读世界（测试 / 观测编码）。
    pub fn world(&self) -> &World {
        &self.world
    }

    /// 新一局（spec §4.2）：抽起点与 `episode_seed` → 模板 `copy_into` + `reseed` →
    /// 随机预热（死亡 / 段落结束则重试，上限 `MAX_WARMUP_RETRIES`；耗尽 k = 0 开局）。
    pub fn reset(&mut self) {
        let s = splitmix64(
            self.cfg.seed
                ^ (self.env_index as u64).wrapping_mul(K1)
                ^ self.counter.wrapping_mul(K2),
        );
        self.counter += 1;

        // 起点加权采样：整数外的唯一浮点（断层线以上，相同位级运算保证确定性）。
        let sum: f64 = self.weights.iter().sum();
        let u = ((splitmix64(s ^ 1) >> 11) as f64) / ((1u64 << 53) as f64);
        let mut cum = 0.0f64;
        let mut chosen = self.weights.len().saturating_sub(1);
        for (i, &w) in self.weights.iter().enumerate() {
            cum += w;
            if u * sum < cum {
                chosen = i;
                break;
            }
        }
        self.start_index = chosen;
        let start = self.cfg.starts[chosen].clone();
        let image = &self.cfg.images[start.image].0;

        let mut accepted: Option<(Box<World>, u32)> = None;
        for retry in 0..=MAX_WARMUP_RETRIES {
            let seed = splitmix64(s ^ (retry as u64 + 2));
            let mut world = self.cache.template(&self.cfg, &start);
            world.reseed(seed);
            let k = if retry == MAX_WARMUP_RETRIES {
                0
            } else {
                splitmix64(seed ^ 0xA5) % (self.cfg.warmup_max as u64 + 1)
            };
            if warmup_walk(&mut world, image, seed, k) {
                accepted = Some((world, retry));
                break;
            }
        }
        let (world, retry) = accepted.expect("末轮 k=0 必然接受");
        self.world = world;
        self.warmup_retries = retry;
        self.ep_frames = 0;
        let (graze, score, bombs) = {
            let v = self.world.view();
            let p = &v.players()[0];
            (p.graze, p.score, p.bombs)
        };
        self.prev_graze = graze;
        self.prev_score = score;
        self.prev_bombs = bombs;
    }

    /// 跑 `frame_skip` 帧（判到 done 即停）。`done != 0` 时本函数内已自动 `reset`：
    /// 返回的 `StepOut` 是刚结束那局的统计，`world()` 已是新局第一帧。
    pub fn step(&mut self, action: u32) -> StepOut {
        let mut ev = [0i32; EVENTS];
        let mut done = DONE_NONE;
        let mut seg_hit = false;
        let image = self.cfg.images[self.cfg.starts[self.start_index].image]
            .0
            .clone();

        for _ in 0..self.cfg.frame_skip {
            let mut input = InputFrame::empty(self.world.frame());
            input.actions[0].buttons = action & ACTION_MASK;
            core_step(&mut self.world, &TABLES_V0, &image, &input);
            self.ep_frames += 1;

            for e in self.world.frame_events() {
                match e.kind {
                    EVT_PLAYER_DIED => ev[0] = ev[0].saturating_add(1),
                    EVT_ENEMY_DIED => ev[3] = ev[3].saturating_add(1),
                    EVT_SHOT_HIT_ENEMY => ev[4] = ev[4].saturating_add(1),
                    EVT_ITEM_PICKED => ev[6] = ev[6].saturating_add(1),
                    kind => {
                        if let Some((code, on)) = segment_end(kind) {
                            ev[7] = code; // 取本步最后一个
                            if self.cfg.end_on.contains(&on) {
                                seg_hit = true; // 本步任一所列事件即算
                            }
                        }
                    }
                }
            }

            // 增量列（spec §6）：graze / score / bombs 下降量，饱和到 i32。
            let (graze, score, bombs) = {
                let v = self.world.view();
                let p = &v.players()[0];
                (p.graze, p.score, p.bombs)
            };
            let dg = graze.saturating_sub(self.prev_graze).min(i32::MAX as u32) as i32;
            ev[1] = ev[1].saturating_add(dg);
            let ds = score.saturating_sub(self.prev_score).min(i32::MAX as u64) as i32;
            ev[2] = ev[2].saturating_add(ds);
            let db = self.prev_bombs.saturating_sub(bombs);
            ev[5] = ev[5].saturating_add(db as i32);
            self.prev_graze = graze;
            self.prev_score = score;
            self.prev_bombs = bombs;

            // 同帧死亡与段落结束并存 ⇒ 1 优先（spec §4.4）。
            if ev[0] > 0 {
                done = DONE_DIED;
            } else if seg_hit {
                done = DONE_SEGMENT;
            } else if self.ep_frames >= self.cfg.max_frames {
                done = DONE_TIMEOUT;
            }
            if done != DONE_NONE {
                break;
            }
        }

        let out = StepOut {
            events: ev,
            done,
            ep_frames: self.ep_frames as i32,
            warmup_retries: self.warmup_retries as i32,
            start_index: self.start_index as i32,
        };
        if done != DONE_NONE {
            self.reset();
        }
        out
    }
}
