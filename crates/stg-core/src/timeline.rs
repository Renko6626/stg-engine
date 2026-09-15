//! timeline —— World 之上的时间线层（时间机制内核刀 2026-09-07，spec
//! `docs/superpowers/specs/2026-09-07-timeline-observe-jump-rewind-design.md` §3）。
//!
//! 住 `step.rs` 同层（组装层之上）：快照环 + 遡行兑现 + 影子世界（観測）+ 输入日志/回放。
//! **World 对它无知**（P1/P5）——世界侧只有 `commit_death` 发请求、`rewind_landed` 收落地两个口，
//! 本模块是那条请求的唯一认领者。无浮点、无 Godot、无时钟，harness 与桥共用。
//!
//! 三条基石（spec §1）：
//! 1. **预览即真实靠同一份代码**：影子 = 克隆 + 喂一帧 `BTN_JUMP` + step N，与真跳同路。
//! 2. **一切代价在落地侧付**：恢复快照后才调 `rewind_landed`；世界侧在死分支里扣的代价随快照恢复丢弃。
//! 3. **回放格式与节拍无关**：`advance` 永远一帧；跳躍的"瞬时"是宿主连续调 `advance` 的节拍。
//!
//! 内存：`RING_DEPTH` × ~1 MB 环 + 1 MB 影子，全部在构造时分配一次（F1：不新增 std 触点，
//! `Box<World>` 分配落点与 `World::new` 同一处）。

use crate::ecl::binding::TaskStartError;
use crate::ecl::image::EclImage;
use crate::events::EVT_REWIND_REQUESTED;
use crate::input::{BTN_JUMP, InputFrame};
use crate::player::Loadout;
use crate::step::World;
use crate::tables::WorldTables;

/// 遡行落点深度：落点 = 被弹帧 − 本值（向下取整到存档帧，再钳到环里最老一帧）。
/// 策划案 2.3 暂定 0.5–1 s，先取 0.5 s。
pub const REWIND_DEPTH: u32 = 30;
/// 快照环槽数。落点最远 = 被弹帧 − 30，按键最晚在被弹后 `DEATHBOMB_WINDOW`(8) 帧 ⇒ 38，
/// 余量到 48。
pub const RING_DEPTH: usize = 48;
/// 快照步长：只存 `frame % RING_STRIDE == 0` 的帧（环的首帧例外——构造时无条件存）。
/// 遡行落点向下取整到步长倍数，所以落点永远是存档帧，不需要"就近恢复再补 step"。
/// 先 1；改 2 内存减半、落点误差 ≤1 帧。
pub const RING_STRIDE: u32 = 1;

/// 一次遡行的记录：`at` = 请求发生的帧（只作诊断/HUD），`to` = 落点帧（重放只看它），
/// `player` = 谁遡行的（落地写给谁）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cut {
    pub at: u32,
    pub to: u32,
    pub player: u8,
}

/// 时间线的出生方式——回放头的一部分。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Boot {
    /// 正典开机（`World::new_game_at` 的四元组）；可从头重放。
    NewGameAt {
        seed: u64,
        rank: i32,
        start: i32,
        loadout: Loadout,
    },
    /// 从一份存档载入（`load_state` 之后）；只记载入那一刻世界的校验和。**不可从头重放**
    /// （`Timeline::replay` 返 `BootNotReplayable`）——练习模式要把快照嵌进 log（follow-ups）。
    Snapshot { world_checksum: u64 },
}

/// 输入日志 = 线性帧数组 + 剪切表。被遡行丢弃的分支**不保留**：它对未来的唯一影响是
/// `rewind_landed` 那一笔，由 `cuts` 记下。索引约定：`frames[f]` 是 `world.frame() == f`
/// 时喂的输入（产出状态 f+1）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputLog {
    pub boot: Boot,
    pub frames: Vec<InputFrame>,
    pub cuts: Vec<Cut>,
}

/// `advance` 的结果：本帧是否发生了遡行。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Advance {
    pub rewound: Option<Cut>,
}

/// 回放播放游标（壳子刀 2026-09-11）：一份线性 log + 走到哪了。`Timeline::playback_step` 一次
/// 走一帧，宿主把它接在 `step_frame` 的位置上就是「看回放」；`Timeline::replay_with` 是同一条
/// 循环的一口气版本，两者不可能分歧。
pub struct Playback {
    log: InputLog,
    cursor: usize,
    cut_i: usize,
}

impl Playback {
    pub fn frames_total(&self) -> u32 {
        self.log.frames.len() as u32
    }
    pub fn cursor(&self) -> u32 {
        self.cursor as u32
    }
    pub fn log(&self) -> &InputLog {
        &self.log
    }
}

/// `playback_step` 的结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlaybackStep {
    /// 本帧落了 cut（世界在喂输入前被 `rewind_landed`）——播放里没有被丢弃的分支，壳不做倒放
    /// 动画，只是知道"这里录制时遡行过"。
    pub landed: Option<Cut>,
    /// log 已走完：本次没有喂帧（末帧上的 cut 仍会落）。
    pub done: bool,
}

/// 回放/日志字节层的失败（P4 式：一切坏输入 → Err，不 panic）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayError {
    /// `Boot::Snapshot` 的 log 不可从头重放。
    BootNotReplayable,
    BadMagic,
    BadFileVer {
        got: u8,
    },
    EngineVerMismatch {
        file: u32,
        engine: u32,
    },
    /// 输入词表指纹不符——同一份 `InputFrame` 字节在两版引擎里语义不同。
    VocabMismatch {
        file: u64,
        engine: u64,
    },
    TablesMismatch {
        file: u64,
        given: u64,
    },
    ImageMismatch {
        file: u64,
        given: u64,
    },
    HashMismatch {
        file: u64,
        computed: u64,
    },
    Truncated,
    TrailingBytes {
        left: usize,
    },
    BadBootTag {
        got: u8,
    },
    /// 开机失败（rank/角色/标记越界）。
    Boot(TaskStartError),
    /// 线性 log 里不该出现遡行请求（请求帧总在被丢弃的分支上）；出现即 log 与引擎不一致。
    UnexpectedRewindRequest {
        frame: u32,
    },
}

// ── 快照环 ────────────────────────────────────────────────────────────────

/// 固定槽数的快照环。槽由帧号决定（`(frame / RING_STRIDE) % RING_DEPTH`），每槽记着它现在
/// 装的是哪一帧，所以"最老一帧"是槽表的 min、淘汰是被覆写的自然结果——不维护游标。
pub struct SnapshotRing {
    slots: Vec<Box<World>>,
    /// 每槽当前装的帧号；`None` = 空槽。
    frames: Vec<Option<u32>>,
    newest: Option<u32>,
}

/// 某帧是不是存档帧（`RING_STRIDE` 的倍数）。单独成函数是为了把"步长今天是 1"这件事
/// 与 clippy 的 `modulo_one` 隔开——常量本来就是要改的。
#[inline]
#[allow(clippy::modulo_one, clippy::manual_is_multiple_of)]
pub const fn is_stored_frame(frame: u32) -> bool {
    frame % RING_STRIDE == 0
}

impl SnapshotRing {
    fn new() -> Self {
        SnapshotRing {
            slots: (0..RING_DEPTH).map(|_| World::new(0)).collect(),
            frames: vec![None; RING_DEPTH],
            newest: None,
        }
    }

    #[inline]
    fn idx(frame: u32) -> usize {
        ((frame / RING_STRIDE) as usize) % RING_DEPTH
    }

    /// 存一帧（覆写同槽旧帧）。
    fn push(&mut self, w: &World) {
        let f = w.frame();
        let i = Self::idx(f);
        w.copy_into(&mut self.slots[i]);
        self.frames[i] = Some(f);
        self.newest = Some(f);
    }

    /// 环里最老的一帧（空环 → None）。只数 `≤ newest` 的槽——遡行作废的槽不算。
    pub fn oldest(&self) -> Option<u32> {
        let newest = self.newest?;
        self.frames
            .iter()
            .flatten()
            .copied()
            .filter(|f| *f <= newest)
            .min()
    }

    pub fn newest(&self) -> Option<u32> {
        self.newest
    }

    /// 取某帧的快照；不在环里（未存 / 已淘汰 / 比 `newest` 新的作废槽）→ None。
    pub fn get(&self, frame: u32) -> Option<&World> {
        let i = Self::idx(frame);
        (self.frames[i] == Some(frame) && self.newest.is_some_and(|n| frame <= n))
            .then(|| &*self.slots[i])
    }

    /// 遡行被丢弃分支上的快照（`frame > newest`，槽尚未被新帧覆写）。**只给宿主倒放用**：
    /// 遡行发生后、下一次 `advance` 之前，这些槽原封不动，宿主可以从请求帧倒着读到落点、
    /// 画"逐帧倒退"；一旦时间线继续推进，它们会被同槽新帧逐个覆写。不在环里 → None。
    pub fn get_discarded(&self, frame: u32) -> Option<&World> {
        let i = Self::idx(frame);
        (self.frames[i] == Some(frame) && self.newest.is_some_and(|n| frame > n))
            .then(|| &*self.slots[i])
    }

    /// 遡行后把比 `frame` 新的槽作废（只动 `newest`，槽内容留给 `get_discarded`）。
    fn truncate_to(&mut self, frame: u32) {
        self.newest = Some(frame);
    }
}

// ── 时间线 ────────────────────────────────────────────────────────────────

/// 权威世界 + 它的历史（环）+ 它的未来（影子）+ 它的来路（log）。
pub struct Timeline {
    world: Box<World>,
    ring: SnapshotRing,
    shadow: Box<World>,
    /// 影子下一步是否要喂 `BTN_JUMP`（`preview_begin` 置 1，首步清 0）。
    shadow_arm_jump: bool,
    log: InputLog,
    tables: &'static WorldTables,
    image: EclImage,
}

impl Timeline {
    /// 正典开机（`World::new_game_at` 同款四元组），环首帧 = 开局世界（帧 0）。
    pub fn new_game_at(
        seed: u64,
        rank: i32,
        start: i32,
        loadout: Loadout,
        image: EclImage,
    ) -> Result<Timeline, TaskStartError> {
        let world = World::new_game_at(seed, rank, start, loadout, &image)?;
        Ok(Self::from_world(
            world,
            image,
            Boot::NewGameAt {
                seed,
                rank,
                start,
                loadout,
            },
        ))
    }

    /// 从任意世界起一条时间线（`load_state` 之后 / 测试手摆场景）：环清空后无条件存入
    /// 当前帧当首帧，log 重开。`boot` 说明它的来路（决定能否重放）。
    /// 表恒绑内建 `TABLES_V0`（follow-ups A3 口径，与桥同）。
    pub fn from_world(world: Box<World>, image: EclImage, boot: Boot) -> Timeline {
        let mut ring = SnapshotRing::new();
        ring.push(&world);
        Timeline {
            world,
            ring,
            shadow: World::new(0),
            shadow_arm_jump: false,
            log: InputLog {
                boot,
                frames: Vec::new(),
                cuts: Vec::new(),
            },
            tables: &crate::tables::TABLES_V0,
            image,
        }
    }

    // ── 读口 ──

    pub fn world(&self) -> &World {
        &self.world
    }
    /// **带外**可变访问：改了权威世界而 log 不知情 ⇒ 这条时间线的 log 不再能重放出同样的
    /// 状态。只给调试/测试/练习模式（手摆场景、作弊菜单）用；正常玩法一切改动走 `advance`。
    pub fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }
    pub fn frame(&self) -> u32 {
        self.world.frame()
    }
    pub fn image(&self) -> &EclImage {
        &self.image
    }
    pub fn tables(&self) -> &'static WorldTables {
        self.tables
    }
    pub fn log(&self) -> &InputLog {
        &self.log
    }
    /// 环里某帧的快照（历史 = 现在这条时间线上的帧）。
    pub fn ring_get(&self, frame: u32) -> Option<&World> {
        self.ring.get(frame)
    }
    /// 刚被遡行丢弃的分支上的快照（宿主倒放动画用，见 `SnapshotRing::get_discarded`）。
    pub fn ring_get_discarded(&self, frame: u32) -> Option<&World> {
        self.ring.get_discarded(frame)
    }
    pub fn ring(&self) -> &SnapshotRing {
        &self.ring
    }

    // ── 推进 ──

    /// 一次只走一帧：step → 记 log → 存环 → 认领遡行请求（恢复 + 落地 + 覆写环槽 + 截 log）。
    ///
    /// 跳躍的快进**不在这里**：宿主看到 `LIFE_JUMPING` 自己连续调本函数（喂空帧）。
    pub fn advance(&mut self, input: &InputFrame) -> Advance {
        self.step_raw(input);
        let req = self
            .world
            .frame_events()
            .iter()
            .find(|e| e.kind == EVT_REWIND_REQUESTED)
            .map(|e| (e.a_index as usize, e.data[0] as u32));
        let Some((player, hit_frame)) = req else {
            return Advance { rewound: None };
        };
        let cut = self.rewind(player, hit_frame);
        Advance { rewound: Some(cut) }
    }

    /// step + 记 log + 按步长存环。**不**认领遡行请求（回放用它，请求在那里是错）。
    fn step_raw(&mut self, input: &InputFrame) {
        crate::step::step(&mut self.world, self.tables, &self.image, input);
        self.log.frames.push(*input);
        if is_stored_frame(self.world.frame()) {
            self.ring.push(&self.world);
        }
    }

    /// 遡行兑现：落点 = `hit_frame − REWIND_DEPTH` 向下取整到存档帧，再钳到环里最老一帧
    /// （环永不空——构造时就存了首帧——所以请求永远能兑现；开局早期被弹就是一次短遡行）。
    fn rewind(&mut self, player: usize, hit_frame: u32) -> Cut {
        let at = self.world.frame();
        let want = hit_frame.saturating_sub(REWIND_DEPTH) / RING_STRIDE * RING_STRIDE;
        let oldest = self.ring.oldest().expect("环构造时已存首帧，不可能为空");
        let to = want.max(oldest);
        let snap = self
            .ring
            .get(to)
            .expect("落点是 [oldest, newest] 内的存档帧（或 oldest 本身），必在环里");
        snap.copy_into(&mut self.world);
        self.world.body.rewind_landed(player);
        // 环永远是"现在这条历史"：落地后的世界覆写回落点槽，比它新的槽作废。
        self.ring.push(&self.world);
        self.ring.truncate_to(to);
        self.log.frames.truncate(to as usize);
        // 落在被丢弃分支上的旧 cut（to 比本次落点新）一并丢弃；等于本次落点的保留——
        // 那次落地已经烙在环槽里，重放要按序再落一次。
        self.log.cuts.retain(|c| c.to <= to);
        let cut = Cut {
            at,
            to,
            player: player as u8,
        };
        self.log.cuts.push(cut);
        cut
    }

    /// 封印历史（关底用）：环里只留当前帧，之后的遡行最远只能退到这里。关卡结算是遡行的
    /// **硬边界**——否则恢复 step 后 30 帧内被弹会退到上一关挂牌之前，`EVT_STAGE_CLEARED`
    /// 重发、结算页弹两遍。log 不动（回放照样从头重放；封印只影响遡行落点）。
    pub fn seal_history(&mut self) {
        self.ring.frames.fill(None);
        self.ring.newest = None;
        self.ring.push(&self.world);
    }

    // ── 影子世界（観測）──

    /// 开始一次预览：权威世界克隆进影子，清掉影子自机 0 的 `BTN_JUMP` 旧电平（玩家正按着键
    /// 时沿检测才认得出下一步那一下），下一步喂跳躍位。**不碰环、不碰 log、不碰权威世界。**
    pub fn preview_begin(&mut self) {
        self.world.copy_into(&mut self.shadow);
        self.shadow.body.players[0].prev_input &= !BTN_JUMP;
        self.shadow.body.players[0].input &= !BTN_JUMP;
        self.shadow_arm_jump = true;
    }

    /// 影子走一步：首步 = 只带 `BTN_JUMP` 的一帧，之后空帧。返回影子当前状态。
    pub fn preview_step(&mut self) -> &World {
        let mut f = InputFrame::empty(self.shadow.frame());
        if self.shadow_arm_jump {
            f.actions[0].buttons = BTN_JUMP;
            self.shadow_arm_jump = false;
        }
        crate::step::step(&mut self.shadow, self.tables, &self.image, &f);
        &self.shadow
    }

    /// `preview_begin` + 起跳那一步 + 再走 `n` 步的糖：`n` = 跳过的帧数，返回的影子就是
    /// **落地那一帧**（`n == JUMP_FRAMES` 时自机刚回 ALIVE）。与真跳对应：真世界喂一帧
    /// `BTN_JUMP` 再走 `n` 帧空帧。
    pub fn preview(&mut self, n: u32) -> &World {
        self.preview_begin();
        for _ in 0..=n {
            self.preview_step();
        }
        &self.shadow
    }

    /// 影子当前状态（上一次 `preview_*` 之后）。
    pub fn shadow(&self) -> &World {
        &self.shadow
    }

    // ── 回放 ──

    /// 从 log 重建一条时间线：按 `frames` 逐帧走，走到某条 cut 的 `to` 帧（喂该帧输入
    /// **之前**）就落地一次并覆写环槽——不重跑被丢弃的分支。线性 log 里出现遡行请求 =
    /// log 与引擎不一致 → `UnexpectedRewindRequest`。
    pub fn replay(log: &InputLog, image: EclImage) -> Result<Timeline, ReplayError> {
        Self::replay_with(log, image, |_| {})
    }

    /// `replay` 的可观测版：每帧推进后（含落地后）调一次 `observe(&world)`，harness 用它采样
    /// 校验和流。**同一条循环**——就是 `start_playback` + 循环 `playback_step` 到 `done`。
    pub fn replay_with(
        log: &InputLog,
        image: EclImage,
        mut observe: impl FnMut(&World),
    ) -> Result<Timeline, ReplayError> {
        let (mut t, mut pb) = Timeline::start_playback(log.clone(), image)?;
        loop {
            let r = t.playback_step(&mut pb)?;
            if r.landed.is_some() || !r.done {
                observe(&t.world);
            }
            if r.done {
                break;
            }
        }
        Ok(t)
    }

    /// 从一份 log 开机进入播放（`Boot::NewGameAt` 才能从头重放）。
    pub fn start_playback(
        log: InputLog,
        image: EclImage,
    ) -> Result<(Timeline, Playback), ReplayError> {
        let Boot::NewGameAt {
            seed,
            rank,
            start,
            loadout,
        } = log.boot
        else {
            return Err(ReplayError::BootNotReplayable);
        };
        let t =
            Timeline::new_game_at(seed, rank, start, loadout, image).map_err(ReplayError::Boot)?;
        Ok((
            t,
            Playback {
                log,
                cursor: 0,
                cut_i: 0,
            },
        ))
    }

    /// 播放一帧：先落**当前帧**上的 cut（`rewind_landed` + 覆写环槽 + 记 cut），再喂
    /// `frames[cursor]`。log 走完 → `done`（不喂帧，但末帧上的 cut 照落）。线性 log 里出现
    /// 遡行请求 = log 与引擎不一致 → `UnexpectedRewindRequest`。
    pub fn playback_step(&mut self, pb: &mut Playback) -> Result<PlaybackStep, ReplayError> {
        let f = self.frame();
        debug_assert_eq!(
            f as usize, pb.cursor,
            "log 索引与帧号约定：frames[f] 在 frame()==f 时喂"
        );
        let mut landed = None;
        while pb.cut_i < pb.log.cuts.len() && pb.log.cuts[pb.cut_i].to == f {
            let c = pb.log.cuts[pb.cut_i];
            self.world.body.rewind_landed(c.player as usize);
            self.ring.push(&self.world);
            self.log.cuts.push(c);
            landed = Some(c);
            pb.cut_i += 1;
        }
        if pb.cursor >= pb.log.frames.len() {
            return Ok(PlaybackStep { landed, done: true });
        }
        let input = pb.log.frames[pb.cursor];
        pb.cursor += 1;
        self.step_raw(&input);
        if self
            .world
            .frame_events()
            .iter()
            .any(|e| e.kind == EVT_REWIND_REQUESTED)
        {
            return Err(ReplayError::UnexpectedRewindRequest {
                frame: self.frame(),
            });
        }
        Ok(PlaybackStep {
            landed,
            done: false,
        })
    }

    /// log 的字节形态（格式见 [`InputLog::to_bytes`]）。
    pub fn log_bytes(&self) -> Vec<u8> {
        self.log
            .to_bytes(self.world.tables_hash(), self.image.content_hash())
    }
}

// ── InputLog 字节格式 v2 ───────────────────────────────────────────────────
//
// magic "STGR" | file_ver u8 | ENGINE_VER u32 | tables_hash u64 | image_hash u64 | vocab_hash u64
// | boot: tag u8 (+ 0: seed u64, rank i32, start i32, loadout{character u8, power u16, lives u8,
//   bombs u8} / 1: world_checksum u64)（v2 玩法刀：删 time_stops）
// | n_frames u32 | frames[n]: frame u32, (buttons u32, _pad u32) × MAX_PLAYERS
// | n_cuts u32 | cuts[n]: at u32, to u32, player u8
// | fnv u64（对以上全部字节的 FNV-1a 64）
// 纪律同 `save.rs`：小端、无 padding、定长自描述。

pub(crate) const LOG_MAGIC: [u8; 4] = *b"STGR";
pub(crate) const LOG_FILE_VER: u8 = 2;

impl InputLog {
    pub fn to_bytes(&self, tables_hash: u64, image_hash: u64) -> Vec<u8> {
        let mut out = Vec::with_capacity(64 + self.frames.len() * 20 + self.cuts.len() * 9);
        out.extend_from_slice(&LOG_MAGIC);
        out.push(LOG_FILE_VER);
        out.extend_from_slice(&crate::ENGINE_VER.to_le_bytes());
        out.extend_from_slice(&tables_hash.to_le_bytes());
        out.extend_from_slice(&image_hash.to_le_bytes());
        out.extend_from_slice(&crate::input::actions_vocab_hash().to_le_bytes());
        match self.boot {
            Boot::NewGameAt {
                seed,
                rank,
                start,
                loadout,
            } => {
                out.push(0);
                out.extend_from_slice(&seed.to_le_bytes());
                out.extend_from_slice(&rank.to_le_bytes());
                out.extend_from_slice(&start.to_le_bytes());
                out.push(loadout.character);
                out.extend_from_slice(&loadout.power.to_le_bytes());
                out.push(loadout.lives);
                out.push(loadout.bombs);
            }
            Boot::Snapshot { world_checksum } => {
                out.push(1);
                out.extend_from_slice(&world_checksum.to_le_bytes());
            }
        }
        out.extend_from_slice(&(self.frames.len() as u32).to_le_bytes());
        for f in &self.frames {
            out.extend_from_slice(&f.frame.to_le_bytes());
            for a in &f.actions {
                out.extend_from_slice(&a.buttons.to_le_bytes());
                out.extend_from_slice(&a._pad.to_le_bytes());
            }
        }
        out.extend_from_slice(&(self.cuts.len() as u32).to_le_bytes());
        for c in &self.cuts {
            out.extend_from_slice(&c.at.to_le_bytes());
            out.extend_from_slice(&c.to.to_le_bytes());
            out.push(c.player);
        }
        let fnv = crate::checksum::fnv1a64(&out);
        out.extend_from_slice(&fnv.to_le_bytes());
        out
    }

    /// 解析并校验头（引擎版 / 词表 / 表 / 镜像 / 整体 FNV）。表或镜像哈希任一侧为 0 视为
    /// 未绑定放行（同 `load_bytes` 口径）。
    pub fn from_bytes(
        bytes: &[u8],
        tables_hash: u64,
        image_hash: u64,
    ) -> Result<InputLog, ReplayError> {
        use crate::save::SaveReader;
        // 尾部 FNV 先验：整体损坏优先于任何字段级解释。
        if bytes.len() < 8 {
            return Err(ReplayError::Truncated);
        }
        let (body, tail) = bytes.split_at(bytes.len() - 8);
        let f_fnv = u64::from_le_bytes(tail.try_into().unwrap());
        let computed = crate::checksum::fnv1a64(body);
        if f_fnv != computed {
            return Err(ReplayError::HashMismatch {
                file: f_fnv,
                computed,
            });
        }
        let mut r = SaveReader::new(body);
        fn take<'a>(r: &mut SaveReader<'a>, n: usize) -> Result<&'a [u8], ReplayError> {
            r.take(n).map_err(|_| ReplayError::Truncated)
        }
        if take(&mut r, 4)? != LOG_MAGIC {
            return Err(ReplayError::BadMagic);
        }
        let ver = take(&mut r, 1)?[0];
        if ver != LOG_FILE_VER {
            return Err(ReplayError::BadFileVer { got: ver });
        }
        fn u32_of(r: &mut SaveReader<'_>) -> Result<u32, ReplayError> {
            Ok(u32::from_le_bytes(take(r, 4)?.try_into().unwrap()))
        }
        fn u64_of(r: &mut SaveReader<'_>) -> Result<u64, ReplayError> {
            Ok(u64::from_le_bytes(take(r, 8)?.try_into().unwrap()))
        }
        let eng = u32_of(&mut r)?;
        if eng != crate::ENGINE_VER {
            return Err(ReplayError::EngineVerMismatch {
                file: eng,
                engine: crate::ENGINE_VER,
            });
        }
        let f_tables = u64_of(&mut r)?;
        let f_image = u64_of(&mut r)?;
        let f_vocab = u64_of(&mut r)?;
        let vocab = crate::input::actions_vocab_hash();
        if f_vocab != vocab {
            return Err(ReplayError::VocabMismatch {
                file: f_vocab,
                engine: vocab,
            });
        }
        if f_tables != 0 && tables_hash != 0 && f_tables != tables_hash {
            return Err(ReplayError::TablesMismatch {
                file: f_tables,
                given: tables_hash,
            });
        }
        if f_image != 0 && image_hash != 0 && f_image != image_hash {
            return Err(ReplayError::ImageMismatch {
                file: f_image,
                given: image_hash,
            });
        }
        let boot = match take(&mut r, 1)?[0] {
            0 => {
                let seed = u64_of(&mut r)?;
                let rank = u32_of(&mut r)? as i32;
                let start = u32_of(&mut r)? as i32;
                let character = take(&mut r, 1)?[0];
                let power = u16::from_le_bytes(take(&mut r, 2)?.try_into().unwrap());
                let lives = take(&mut r, 1)?[0];
                let bombs = take(&mut r, 1)?[0];
                Boot::NewGameAt {
                    seed,
                    rank,
                    start,
                    loadout: Loadout {
                        character,
                        power,
                        lives,
                        bombs,
                    },
                }
            }
            1 => Boot::Snapshot {
                world_checksum: u64_of(&mut r)?,
            },
            got => return Err(ReplayError::BadBootTag { got }),
        };
        let n_frames = u32_of(&mut r)? as usize;
        // 分配前用剩余长度封顶（C11 记的那条"蓄意 count 预分配 OOM"教训）。
        let per_frame = 4 + 8 * crate::MAX_PLAYERS;
        if n_frames.saturating_mul(per_frame) > r.remaining() {
            return Err(ReplayError::Truncated);
        }
        let mut frames = Vec::with_capacity(n_frames);
        for _ in 0..n_frames {
            let mut f = InputFrame::empty(u32_of(&mut r)?);
            for a in f.actions.iter_mut() {
                a.buttons = u32_of(&mut r)?;
                a._pad = u32_of(&mut r)?;
            }
            frames.push(f);
        }
        let n_cuts = u32_of(&mut r)? as usize;
        if n_cuts.saturating_mul(9) > r.remaining() {
            return Err(ReplayError::Truncated);
        }
        let mut cuts = Vec::with_capacity(n_cuts);
        for _ in 0..n_cuts {
            let at = u32_of(&mut r)?;
            let to = u32_of(&mut r)?;
            let player = take(&mut r, 1)?[0];
            cuts.push(Cut { at, to, player });
        }
        if r.remaining() != 0 {
            return Err(ReplayError::TrailingBytes {
                left: r.remaining(),
            });
        }
        Ok(InputLog { boot, frames, cuts })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Fx;
    use crate::player::{
        DEATHBOMB_WINDOW, JUMP_FRAMES, LIFE_ALIVE, LIFE_DEATHWINDOW, LIFE_JUMPING, REWIND_INVULN,
    };
    use crate::world::test_support::bullet_at;

    fn keys(buttons: u32) -> InputFrame {
        let mut f = InputFrame::empty(0);
        f.actions[0].buttons = buttons;
        f
    }

    /// 带 root 的最小镜像（`new_game_at` 需要 `main`）：`sub main() {}` 立即结束。
    fn root_image() -> EclImage {
        use crate::ecl::image::{SubInit, SubKind, test_image};
        test_image(
            vec![crate::ecl::ops::OP_END as u32],
            vec![SubInit::new(0, SubKind::Root, vec![])],
            vec![],
            Some(0),
        )
    }

    /// 空镜像时间线（手摆场景用）。
    fn bare(seed: u64) -> Timeline {
        Timeline::from_world(
            World::new(seed),
            EclImage::empty(),
            Boot::NewGameAt {
                seed,
                rank: 0,
                start: 0,
                loadout: Loadout::default(),
            },
        )
    }

    /// 在权威世界里放一颗压在自机身上的弹（带外改动——只在测试里造中弹用）。
    fn plant_hit(t: &mut Timeline) {
        let (x, y) = (
            t.world.body.players[0].x.raw() >> 16,
            t.world.body.players[0].y.raw() >> 16,
        );
        bullet_at(&mut t.world, x, y);
    }

    /// 中弹后空跑到决死窗口耗尽 → 返回 (请求帧, 遡行 cut)（玩法刀：死亡即遡行，无遡行键）。
    fn die_and_rewind(t: &mut Timeline) -> (u32, Cut) {
        for _ in 0..=DEATHBOMB_WINDOW {
            let at = t.frame() + 1;
            if let Some(c) = t.advance(&InputFrame::empty(0)).rewound {
                return (at, c);
            }
        }
        panic!("决死窗口耗尽必须遡行");
    }

    // ── 环 ──

    #[test]
    fn ring_stores_first_frame_and_evicts_oldest_by_slot_reuse() {
        let mut t = bare(1);
        assert_eq!(t.ring.oldest(), Some(0));
        assert_eq!(t.ring.newest(), Some(0));
        assert!(t.ring_get(0).is_some());
        for _ in 0..(RING_DEPTH as u32 * RING_STRIDE + 5) {
            t.advance(&InputFrame::empty(0));
        }
        let newest = t.frame();
        assert_eq!(t.ring.newest(), Some(newest));
        // 帧 0 已被同槽新帧覆写淘汰；最老一帧 = newest − (DEPTH−1)·STRIDE
        assert!(t.ring_get(0).is_none());
        let oldest = newest - (RING_DEPTH as u32 - 1) * RING_STRIDE;
        assert_eq!(t.ring.oldest(), Some(oldest));
        assert_eq!(t.ring_get(oldest).unwrap().frame(), oldest);
        assert!(t.ring_get(oldest - RING_STRIDE).is_none());
    }

    #[test]
    fn ring_snapshot_matches_world_checksum_at_that_frame() {
        let mut t = bare(2);
        let mut sums = Vec::new();
        for _ in 0..10 {
            t.advance(&keys(crate::input::BTN_LEFT));
            sums.push(t.world().checksum());
        }
        for (i, s) in sums.iter().enumerate() {
            let f = i as u32 + 1;
            if is_stored_frame(f) {
                assert_eq!(t.ring_get(f).unwrap().checksum(), *s, "帧 {f}");
            }
        }
    }

    // ── 影子 == 真跳 ──

    /// 影子走 N 步的校验和 == 权威世界喂一帧 BTN_JUMP 再走 N 帧空帧；且 != 不跳的世界（防空转）。
    /// 权威世界、环、log 在 preview 前后不变。
    #[test]
    fn preview_equals_a_real_jump_and_touches_nothing() {
        let mut t = bare(3);
        // 给影子一些内容：几颗从头顶 40px 处瞄自机的弹（14 帧内到），不跳会中弹、跳了就穿过——
        // "缺席"真的改变演化；若弹够不着自机，跳与不跳落地后逐位同（跳躍本身零痕迹）
        for k in 0..5 {
            let h = bullet_at(&mut t.world, -40 + k * 20, 344);
            t.world.body.set_bullet_speed(h, Fx::from_int(3));
            t.world
                .body
                .aim_bullet_at_player(h, crate::math::Angle::ZERO);
        }
        for _ in 0..3 {
            t.advance(&keys(crate::input::BTN_RIGHT));
        }
        let w0 = t.world().checksum();
        let r0: Vec<_> = (0..=t.frame())
            .map(|f| t.ring_get(f).map(|w| w.checksum()))
            .collect();
        let l0 = t.log().clone();

        let n = JUMP_FRAMES as u32;
        let shadow_sum = t.preview(n).checksum();
        assert_eq!(
            t.shadow().body.players[0].life_state,
            LIFE_ALIVE,
            "N 步后影子已回 ALIVE"
        );

        assert_eq!(t.world().checksum(), w0, "preview 不碰权威世界");
        let r1: Vec<_> = (0..=t.frame())
            .map(|f| t.ring_get(f).map(|w| w.checksum()))
            .collect();
        assert_eq!(r1, r0, "preview 不碰环");
        assert_eq!(t.log(), &l0, "preview 不碰 log");

        // 真跳：同一世界喂一帧 JUMP 再走 N−1 帧空帧（首帧就是 JUMP 那帧）
        let mut real = bare(3);
        for k in 0..5 {
            let h = bullet_at(&mut real.world, -40 + k * 20, 344);
            real.world.body.set_bullet_speed(h, Fx::from_int(3));
            real.world
                .body
                .aim_bullet_at_player(h, crate::math::Angle::ZERO);
        }
        for _ in 0..3 {
            real.advance(&keys(crate::input::BTN_RIGHT));
        }
        real.advance(&keys(BTN_JUMP));
        assert_eq!(real.world().body.players[0].life_state, LIFE_JUMPING);
        for _ in 0..n {
            real.advance(&InputFrame::empty(0));
        }
        assert_eq!(real.world().checksum(), shadow_sum, "预览即真跳");

        // 不跳的世界（同输入流但首帧不按 JUMP）必不同——否则上面那条是空转
        let mut stay = bare(3);
        for k in 0..5 {
            let h = bullet_at(&mut stay.world, -40 + k * 20, 344);
            stay.world.body.set_bullet_speed(h, Fx::from_int(3));
            stay.world
                .body
                .aim_bullet_at_player(h, crate::math::Angle::ZERO);
        }
        for _ in 0..3 {
            stay.advance(&keys(crate::input::BTN_RIGHT));
        }
        for _ in 0..=n {
            stay.advance(&InputFrame::empty(0));
        }
        assert_ne!(stay.world().checksum(), shadow_sum, "不跳的未来必须不同");
    }

    /// 玩家正按着 JUMP 时开始预览：影子仍要跳（prev_input 的旧电平被清）。
    #[test]
    fn preview_jumps_even_if_jump_key_is_held_in_the_real_world() {
        let mut t = bare(4);
        // 真世界：按住 JUMP 跨过整个跳躍并回到 ALIVE，此时 input/prev_input 都带 JUMP
        for _ in 0..(JUMP_FRAMES as u32 + 2) {
            t.advance(&keys(BTN_JUMP));
        }
        assert_eq!(t.world().body.players[0].life_state, LIFE_ALIVE);
        assert_ne!(t.world().body.players[0].prev_input & BTN_JUMP, 0);
        // 冷却是另一条门禁（玩法刀），本条只押「旧电平被清」——清掉冷却再预览。
        t.world.body.players[0].jump_cd = 0;
        t.preview_begin();
        t.preview_step();
        assert_eq!(
            t.shadow().body.players[0].life_state,
            LIFE_JUMPING,
            "影子首步必须起跳"
        );
    }

    /// 冷却中観測（玩法刀 spec §3）：影子喂的 JUMP 被门禁拒，影子自机仍在场。
    #[test]
    fn preview_during_cooldown_keeps_the_shadow_player_present() {
        let mut t = bare(15);
        t.world.body.players[0].jump_cd = 100;
        t.preview_begin();
        t.preview_step();
        assert_eq!(t.shadow().body.players[0].life_state, LIFE_ALIVE);
    }

    // ── 遡行 ──

    /// 第 H 帧中弹、H+3 按键：帧号回到 H−30，校验和 == 环槽 H−30 加落地写，log 截断，cut 记录，
    /// 环最新槽 = 落点。
    #[test]
    fn rewind_restores_ring_frame_and_lands() {
        let mut t = bare(5);
        // 走到 H=40 再中弹
        while t.frame() < 40 {
            t.advance(&keys(crate::input::BTN_LEFT));
        }
        let expect_to = 40 - REWIND_DEPTH;
        let snap_sum = t.ring_get(expect_to).unwrap().checksum();
        let snap_lives = t.ring_get(expect_to).unwrap().body.players[0].lives;
        plant_hit(&mut t);
        t.advance(&InputFrame::empty(0)); // 帧 40 内中弹 → hit_frame=40
        assert_eq!(t.world().body.players[0].life_state, LIFE_DEATHWINDOW);
        assert_eq!(t.world().body.players[0].hit_frame, 40);
        let (at, cut) = die_and_rewind(&mut t);
        assert_eq!(
            cut,
            Cut {
                at,
                to: expect_to,
                player: 0
            }
        );
        assert_eq!(t.frame(), expect_to);
        assert_eq!(t.world().body.players[0].life_state, LIFE_ALIVE);
        assert_eq!(t.world().body.players[0].invuln, REWIND_INVULN);
        assert_eq!(
            t.world().body.players[0].lives,
            snap_lives - 1,
            "残机从快照 −1"
        );
        assert_eq!(t.world().body.players[0].deaths, 1);
        // 落地写 = 快照 + invuln/残机/偏差值：三者还原后校验和须等于原快照
        let mut probe = World::new(0);
        t.world().copy_into(&mut probe);
        probe.body.players[0].invuln = 0;
        probe.body.players[0].lives += 1;
        probe.body.players[0].deaths -= 1;
        assert_eq!(probe.checksum(), snap_sum, "恢复的是环里那一帧");
        assert_eq!(t.ring.newest(), Some(expect_to));
        assert!(t.ring_get(expect_to + 1).is_none(), "比落点新的槽作废");
        // 请求帧 at 已存进环（死亡即遡行：窗口耗尽才请求，at=49 已超环深 48）——最老一帧
        // 由请求帧决定，不受落地后作废槽影响。
        assert_eq!(
            t.ring.oldest(),
            Some(at.saturating_sub(RING_DEPTH as u32 - 1)),
            "最老一帧不受作废槽影响"
        );
        let g = t
            .ring_get_discarded(at - 1)
            .expect("被丢弃分支在下一次 advance 前仍可倒放");
        assert_eq!(g.frame(), at - 1);
        assert!(
            t.ring_get_discarded(expect_to).is_none(),
            "落点本身不算丢弃"
        );
        assert_eq!(
            t.ring_get(expect_to).unwrap().checksum(),
            t.world().checksum(),
            "落点槽被落地后的世界覆写"
        );
        assert_eq!(t.log().frames.len(), expect_to as usize);
        assert_eq!(t.log().cuts, vec![cut]);
        assert!(
            t.world().frame_events().is_empty(),
            "恢复出的世界无陈旧事件"
        );
        t.advance(&InputFrame::empty(0));
        assert!(
            t.ring_get_discarded(expect_to + 1).is_none(),
            "推进后同槽被新帧覆写，丢弃帧消失"
        );
    }

    /// 开局第 5 帧中弹 → 钳到帧 0（校验和 == 初始世界加落地写）。
    #[test]
    fn early_rewind_clamps_to_oldest_frame() {
        let mut t = bare(6);
        let init_sum = t.ring_get(0).unwrap().checksum();
        while t.frame() < 5 {
            t.advance(&InputFrame::empty(0));
        }
        plant_hit(&mut t);
        t.advance(&InputFrame::empty(0));
        let (_, cut) = die_and_rewind(&mut t);
        assert_eq!(cut.to, 0);
        assert_eq!(t.frame(), 0);
        let mut probe = World::new(0);
        t.world().copy_into(&mut probe);
        probe.body.players[0].invuln = 0;
        probe.body.players[0].lives += 1;
        probe.body.players[0].deaths -= 1;
        assert_eq!(probe.checksum(), init_sum);
    }

    /// 封印后遡行最远只到封印帧（关底硬边界）。
    #[test]
    fn seal_history_bounds_rewind_to_the_seal_frame() {
        let mut t = bare(14);
        while t.frame() < 50 {
            t.advance(&InputFrame::empty(0));
        }
        t.seal_history();
        assert_eq!(t.ring.oldest(), Some(50));
        assert!(t.ring_get(49).is_none());
        for _ in 0..5 {
            t.advance(&InputFrame::empty(0));
        }
        plant_hit(&mut t);
        t.advance(&InputFrame::empty(0)); // hit_frame = 55 → 想退到 25，钳到 50
        let (_, cut) = die_and_rewind(&mut t);
        assert_eq!(cut.to, 50);
        assert_eq!(t.log().frames.len(), 50, "log 仍是从头的线性历史");
    }

    /// 最后一条命：窗口耗尽 → GAMEOVER，timeline 不遡行（玩法刀 spec §4.2 ②）。
    #[test]
    fn last_life_death_is_gameover_without_rewind() {
        let mut t = bare(7);
        t.world.body.players[0].lives = 1;
        plant_hit(&mut t);
        t.advance(&InputFrame::empty(0));
        for _ in 0..DEATHBOMB_WINDOW {
            assert!(t.advance(&InputFrame::empty(0)).rewound.is_none());
        }
        assert_eq!(
            t.world().body.players[0].life_state,
            crate::player::LIFE_GAMEOVER
        );
        assert_eq!(t.world().body.players[0].deaths, 1);
    }

    /// 落地代价从快照重算 + 符卡失格 + 残机下限 1 的边缘（快照残机 1、死分支里奖命到 2 再死）。
    #[test]
    fn landing_recomputes_the_cost_from_the_snapshot() {
        let mut t = bare(16);
        let boss = crate::world::test_support::spawn_enemy(&mut t.world, 0, 100, 1000);
        assert!(
            t.world
                .body
                .spell_begin_internal(0, boss, 1, 3000, 1000, 0, 100)
        );
        t.world.body.players[0].lives = 1;
        t.ring.push(&t.world);
        while t.frame() < 40 {
            t.advance(&InputFrame::empty(0));
        }
        t.world.body.players[0].lives = 2; // 死分支里「吃到奖命」（带外写，同 plant_hit）
        plant_hit(&mut t);
        t.advance(&InputFrame::empty(0));
        let (_, cut) = die_and_rewind(&mut t);
        assert_eq!(cut.to, 40 - REWIND_DEPTH);
        let p = t.world().body.players[0];
        assert_eq!(p.life_state, LIFE_ALIVE);
        assert_eq!(p.lives, 1, "快照残机 1 − 1 → 钳 1");
        assert_eq!(p.deaths, 1);
        assert_eq!(t.world().body.spells[0].capture_ok, 0, "落地作废资格");
    }

    // ── 回放闸 ──

    /// 现场（随机输入 + 跳躍 + 中弹 + 遡行）录 log → 从头重放出逐帧相同的校验和流。
    /// 中弹靠**场上预置的静止弹阵**（构造时一次性摆好，属于初始状态；回放用同一份构造），
    /// 随机移动会把自机撞上去。
    #[test]
    fn replay_reproduces_live_checksums_including_jumps_and_rewinds() {
        fn build(seed: u64) -> Timeline {
            let mut t = bare(seed);
            // 静止弹阵：自机出生点(0,384)四周一圈，随机走两三步就会撞到
            for k in 0..12 {
                let ang = crate::math::Angle((k as u32 * 65536 / 12) as u16);
                let (dx, dy) = crate::math::polar_to_vec(Fx::from_int(24), ang);
                bullet_at(&mut t.world, dx.raw() >> 16, 384 + (dy.raw() >> 16));
            }
            // 死亡即遡行（玩法刀）：给足命，免得随机走位打到 GAMEOVER
            t.world.body.players[0].lives = 200;
            // 构造后的世界才是"初始状态"：环首帧要重存，否则环里是空场
            t.ring.push(&t.world);
            t
        }
        let mut live = build(11);
        let mut rng = crate::rng::Pcg32::new(11, 7);
        let mut stream: Vec<(u32, u64)> = Vec::new(); // (frame, checksum) 只记存活下来的
        let mut rewinds = 0;
        let mut jumps = 0;
        // 500 次推进：冷却 600 帧下只够跳 1 次，覆盖「跳躍 + 遡行」混合回放已足够；
        // 每次推进都存环 + 全量校验和（~1 MB），次数直接决定 debug 耗时（1400 次 ≈ 38 s）。
        for _ in 0..500 {
            let st = live.world().body.players[0].life_state;
            let mut b = match rng.rand_range(4) {
                0 => crate::input::BTN_LEFT,
                1 => crate::input::BTN_RIGHT,
                2 => crate::input::BTN_UP,
                _ => crate::input::BTN_DOWN,
            };
            let try_jump = st == LIFE_ALIVE && live.frame() % 37 == 20;
            if try_jump {
                b = BTN_JUMP;
            }
            let adv = live.advance(&keys(b));
            if try_jump && live.world().body.players[0].life_state == LIFE_JUMPING {
                jumps += 1;
            }
            if let Some(c) = adv.rewound {
                rewinds += 1;
                stream.retain(|(f, _)| *f <= c.to);
            }
            stream.push((live.frame(), live.world().checksum()));
        }
        assert!(rewinds >= 2, "场景必须真的遡行过（实测 {rewinds}）");
        assert!(jumps >= 1, "场景必须真的跳躍过（实测 {jumps}）");
        let log = live.log().clone();
        assert_eq!(log.cuts.len() as u32, rewinds.min(log.cuts.len() as u32));
        assert_eq!(log.frames.len() as u32, live.frame());

        // 重放：同构造 + 逐帧喂 log；对拍存活帧的校验和流
        let mut rep = build(11);
        let mut ci = 0;
        // 同一帧号可能有两个版本（落地前/落地后）——两边都只留最后一个
        let mut got: std::collections::BTreeMap<u32, u64> = std::collections::BTreeMap::new();
        for (i, input) in log.frames.iter().enumerate() {
            let f = i as u32;
            while ci < log.cuts.len() && log.cuts[ci].to == f {
                rep.world.body.rewind_landed(log.cuts[ci].player as usize);
                got.insert(f, rep.world().checksum());
                ci += 1;
            }
            let adv = rep.advance(input);
            assert!(adv.rewound.is_none(), "线性 log 里不该有遡行请求");
            got.insert(rep.frame(), rep.world().checksum());
        }
        let mut last: std::collections::BTreeMap<u32, u64> = std::collections::BTreeMap::new();
        for (f, s) in &stream {
            last.insert(*f, *s);
        }
        for (f, s) in &got {
            assert_eq!(last.get(f), Some(s), "帧 {f} 校验和分歧");
        }
        assert_eq!(rep.world().checksum(), live.world().checksum());
    }

    /// `Timeline::replay` 正典路径（真开机四元组）+ 字节往返 + Snapshot 拒重放。
    #[test]
    fn replay_from_boot_and_bytes_round_trip() {
        let image = root_image();
        let ld = Loadout::default();
        let mut live = Timeline::new_game_at(9, 1, 0, ld, image.clone()).unwrap();
        // 空镜像开局：自机在场，随机走 + 周期跳躍（无弹，不会中弹——遡行路径由上一条测）
        let mut rng = crate::rng::Pcg32::new(9, 3);
        for _ in 0..120 {
            let mut b = match rng.rand_range(4) {
                0 => crate::input::BTN_LEFT,
                1 => crate::input::BTN_RIGHT,
                2 => crate::input::BTN_UP,
                _ => crate::input::BTN_DOWN,
            };
            if live.frame() % 50 == 10 {
                b = BTN_JUMP;
            }
            live.advance(&keys(b));
        }
        let bytes = live.log_bytes();
        let log2 =
            InputLog::from_bytes(&bytes, live.world().tables_hash(), image.content_hash()).unwrap();
        assert_eq!(&log2, live.log());
        let rep = Timeline::replay(&log2, image.clone()).unwrap();
        assert_eq!(rep.world().checksum(), live.world().checksum());
        assert_eq!(rep.log(), live.log());
        // 篡改一个字节 → HashMismatch
        let mut bad = bytes.clone();
        bad[40] ^= 1;
        assert!(matches!(
            InputLog::from_bytes(&bad, 0, 0),
            Err(ReplayError::HashMismatch { .. })
        ));
        // Snapshot 出身拒重放
        let snap_log = InputLog {
            boot: Boot::Snapshot { world_checksum: 1 },
            frames: vec![],
            cuts: vec![],
        };
        assert_eq!(
            Timeline::replay(&snap_log, image).err(),
            Some(ReplayError::BootNotReplayable)
        );
    }

    /// `playback_step` 逐帧播放（壳子刀）：末态 == `Timeline::replay`，cut 帧上报 `landed`，
    /// 走完报 `done` 且再调仍 `done`、帧号不动。
    #[test]
    fn playback_step_matches_replay_and_reports_landing_and_done() {
        let image = root_image();
        let ld = Loadout::default();
        let log = InputLog {
            boot: Boot::NewGameAt {
                seed: 21,
                rank: 1,
                start: 0,
                loadout: ld,
            },
            frames: (0..40).map(InputFrame::empty).collect(),
            cuts: vec![Cut {
                at: 70,
                to: 25,
                player: 0,
            }],
        };
        let rep = Timeline::replay(&log, image.clone()).unwrap();
        let (mut t, mut pb) = Timeline::start_playback(log.clone(), image).unwrap();
        assert_eq!(pb.frames_total(), 40);
        let mut landed_at = None;
        loop {
            let r = t.playback_step(&mut pb).unwrap();
            if let Some(c) = r.landed {
                assert_eq!(c.to, 25);
                landed_at = Some(t.frame() - 1); // 落在喂 frames[25] 之前，喂完帧号已是 26
            }
            if r.done {
                break;
            }
        }
        assert_eq!(landed_at, Some(25));
        assert_eq!(t.frame(), 40);
        assert_eq!(pb.cursor(), 40);
        assert_eq!(
            t.world().checksum(),
            rep.world().checksum(),
            "逐帧播放 == 一口气重放"
        );
        assert_eq!(t.log(), rep.log());
        let again = t.playback_step(&mut pb).unwrap();
        assert!(again.done);
        assert_eq!(t.frame(), 40, "播完再调不喂帧");
    }

    /// 同一落点两条 cut（spec §3.4）：重放按序各落一次；`invuln` 取 max 所以与落一次逐位同，
    /// 但 cuts 表须原样保留（HUD/偏差值将来要数次数）。末帧上的 cut 也要落（`to == frames.len()`）。
    #[test]
    fn replay_applies_every_cut_in_order_including_trailing_ones() {
        let image = root_image();
        let ld = Loadout::default();
        let mut manual = Timeline::new_game_at(13, 0, 0, ld, image.clone()).unwrap();
        for _ in 0..20 {
            manual.advance(&InputFrame::empty(0));
        }
        manual.world.body.rewind_landed(0);
        manual.world.body.rewind_landed(0);
        let log = InputLog {
            boot: Boot::NewGameAt {
                seed: 13,
                rank: 0,
                start: 0,
                loadout: ld,
            },
            frames: (0..20).map(InputFrame::empty).collect(),
            cuts: vec![
                Cut {
                    at: 55,
                    to: 20,
                    player: 0,
                },
                Cut {
                    at: 77,
                    to: 20,
                    player: 0,
                },
            ],
        };
        let rep = Timeline::replay(&log, image).unwrap();
        assert_eq!(rep.frame(), 20);
        assert_eq!(rep.world().checksum(), manual.world().checksum());
        assert_eq!(rep.world().body.players[0].invuln, REWIND_INVULN);
        assert_eq!(rep.log().cuts, log.cuts, "cuts 原样保留");
        assert_eq!(
            rep.ring_get(20).unwrap().checksum(),
            rep.world().checksum(),
            "落地后覆写环槽"
        );
    }

    /// Classic 机体挂 Timeline：被弹致死不遡行，帧号连续，落在场底（经典机体刀 spec §3.3）。
    #[test]
    fn classic_death_under_timeline_does_not_rewind() {
        let mut w = World::new(1);
        w.body.players[0] =
            crate::player::PlayerState::spawn(1, &crate::tables::TABLES_V0.characters[1]);
        // 先挪离出生点：出生点就是 (0,384)，不挪的话「落在场底」断言对重生是瞎的。
        w.body.players[0].x = crate::math::Fx::from_int(-100);
        w.body.players[0].y = crate::math::Fx::from_int(200);
        let mut t =
            Timeline::from_world(w, EclImage::empty(), Boot::Snapshot { world_checksum: 0 });
        plant_hit(&mut t);
        for _ in 0..=(DEATHBOMB_WINDOW as u32 + 2) {
            let before = t.frame();
            assert_eq!(
                t.advance(&InputFrame::empty(0)).rewound,
                None,
                "Classic 不得遡行"
            );
            assert_eq!(t.frame(), before + 1, "帧号连续");
        }
        assert_eq!(t.world.body.players[0].deaths, 1, "确实死过一次");
        assert_eq!(t.world.body.players[0].y, crate::math::Fx::from_int(384));
    }
}
