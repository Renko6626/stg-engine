//! gdext 壳:WorldBridge(spec §4 冻结面/§12 内部结构)。纯胶水零逻辑。
//! P4/FFI 铁律:错误不 panic 穿 FFI——no-op + false + 去重日志。

use godot::classes::RenderingServer;
use godot::prelude::*;

use crate::boot::{self, Game};
use crate::frame::{self, FLOATS_PER_INSTANCE, LAYER_COUNT};

/// 去重日志位(warned 位集)。
const W_NO_GAME: u32 = 1 << 0;
const W_BAD_LAYER: u32 = 1 << 1;
const W_BAD_MM: u32 = 1 << 2;

#[derive(GodotClass)]
#[class(base=Node)]
pub struct WorldBridge {
    base: Base<Node>,
    game: Option<Game>,
    layers: [Option<Rid>; LAYER_COUNT],
    /// 每层一份定长实例缓冲(表现契约 v2 §4.8):编码器经 `as_mut_slice` 直接写入,
    /// 省掉 `Vec` → Packed 那次拷贝;`multimesh_set_buffer` 内部那次无法省。
    bufs: [PackedFloat32Array; LAYER_COUNT],
    warned: u32,
}

#[godot_api]
impl INode for WorldBridge {
    fn init(base: Base<Node>) -> Self {
        WorldBridge {
            base,
            game: None,
            layers: [None; LAYER_COUNT],
            bufs: Default::default(),
            warned: 0,
        }
    }
}

impl WorldBridge {
    fn warn_once(&mut self, bit: u32, msg: &str) {
        if self.warned & bit == 0 {
            self.warned |= bit;
            godot_error!("[stg] {msg}(同类后续不再报)");
        }
    }
}

#[godot_api]
impl WorldBridge {
    // ── 输入位与层号常量(GDScript 侧 WorldBridge.BTN_* 取用)─────
    // 机械调整许可①(task-4-brief.md 简报草稿写的是 BTN_FOCUS，但 stg_core::input 词表
    // 里没有这个名字——第七个动作位实名 `BTN_SLOW`（低速，见 input.rs 注册处），按代码实名改写。
    #[constant]
    const BTN_UP: i64 = stg_core::input::BTN_UP as i64;
    #[constant]
    const BTN_DOWN: i64 = stg_core::input::BTN_DOWN as i64;
    #[constant]
    const BTN_LEFT: i64 = stg_core::input::BTN_LEFT as i64;
    #[constant]
    const BTN_RIGHT: i64 = stg_core::input::BTN_RIGHT as i64;
    #[constant]
    const BTN_SHOT: i64 = stg_core::input::BTN_SHOT as i64;
    #[constant]
    const BTN_BOMB: i64 = stg_core::input::BTN_BOMB as i64;
    #[constant]
    const BTN_SLOW: i64 = stg_core::input::BTN_SLOW as i64;
    #[constant]
    const BTN_TIMESTOP: i64 = stg_core::input::BTN_TIMESTOP as i64;
    #[constant]
    const LAYER_BULLETS: i64 = frame::LAYER_BULLETS as i64;
    #[constant]
    const LAYER_SHOTS: i64 = frame::LAYER_SHOTS as i64;
    #[constant]
    const LAYER_ITEMS: i64 = frame::LAYER_ITEMS as i64;
    #[constant]
    const LAYER_COUNT: i64 = frame::LAYER_COUNT as i64;
    // 通道 B 引擎保留请求 id 与事件 kind(表现契约 v2 §4.7):**转出来而非让 GDScript 手抄**,
    // 理由同 RANK_*——手抄镜像与 core 之间没有编译期押运。类别(即发即忘/须确认/电平镜像)
    // 见 `stg_core::reqs` 模块文档与 render-contract §4。
    #[constant]
    const REQ_ENEMY_DEATH: i64 = stg_core::consts::REQ_ENEMY_DEATH as i64;
    #[constant]
    const REQ_SPELL_DECLARE: i64 = stg_core::consts::REQ_SPELL_DECLARE as i64;
    #[constant]
    const REQ_SPELL_RESULT: i64 = stg_core::consts::REQ_SPELL_RESULT as i64;
    #[constant]
    const REQ_STAGE_CLEAR: i64 = stg_core::consts::REQ_STAGE_CLEAR as i64;
    #[constant]
    const REQ_BGM: i64 = stg_core::consts::REQ_BGM as i64;
    #[constant]
    const REQ_BG: i64 = stg_core::consts::REQ_BG as i64;
    #[constant]
    const REQ_BG_PHASE: i64 = stg_core::consts::REQ_BG_PHASE as i64;
    #[constant]
    const REQ_FX_AT: i64 = stg_core::consts::REQ_FX_AT as i64;
    #[constant]
    const REQ_FX_ATTACHED: i64 = stg_core::consts::REQ_FX_ATTACHED as i64;
    #[constant]
    const REQ_SCRIPT_BASE: i64 = stg_core::consts::REQ_SCRIPT_BASE as i64;
    #[constant]
    const EVT_ENEMY_DIED: i64 = stg_core::events::EVT_ENEMY_DIED as i64;
    #[constant]
    const EVT_PLAYER_DIED: i64 = stg_core::events::EVT_PLAYER_DIED as i64;
    #[constant]
    const EVT_FIELD_CLEARED: i64 = stg_core::events::EVT_FIELD_CLEARED as i64;
    #[constant]
    const EVT_ITEM_PICKED: i64 = stg_core::events::EVT_ITEM_PICKED as i64;
    #[constant]
    const EVT_TASK_FAULT: i64 = stg_core::events::EVT_TASK_FAULT as i64;
    #[constant]
    const EVT_SPELL_DECLARED: i64 = stg_core::events::EVT_SPELL_DECLARED as i64;
    #[constant]
    const EVT_SPELL_CAPTURED: i64 = stg_core::events::EVT_SPELL_CAPTURED as i64;
    #[constant]
    const EVT_SPELL_FAILED: i64 = stg_core::events::EVT_SPELL_FAILED as i64;
    #[constant]
    const EVT_SHOT_HIT_ENEMY: i64 = stg_core::events::EVT_SHOT_HIT_ENEMY as i64;
    #[constant]
    const VANISH_LIFE: i64 = stg_core::events::VANISH_LIFE as i64;
    #[constant]
    const VANISH_CLEARED: i64 = stg_core::events::VANISH_CLEARED as i64;
    // 难度档(`consts.rs` ①段冻结编号,值域 `0..=4`)。**转出来而非让 GDScript 手抄**:
    // 同 BTN_*/LAYER_* 的既有先例——手抄的镜像与 core 之间没有编译期押运,改了一边
    // 另一边照跑,而 `new_game_at` 的值域校验只认真实数字,抄错了它拦不住(抄成 5 才拦)。
    #[constant]
    const RANK_EASY: i64 = stg_core::consts::RANK_EASY as i64;
    #[constant]
    const RANK_NORMAL: i64 = stg_core::consts::RANK_NORMAL as i64;
    #[constant]
    const RANK_HARD: i64 = stg_core::consts::RANK_HARD as i64;
    #[constant]
    const RANK_LUNATIC: i64 = stg_core::consts::RANK_LUNATIC as i64;
    #[constant]
    const RANK_EXTRA: i64 = stg_core::consts::RANK_EXTRA as i64;

    /// 单单元开机。`rank` 同 `new_game_at`:核内有值域校验,故壳层**饱和**不截断
    /// (`as i32` 是模 2³² 回绕,会把越界值静默折回合法档)。
    #[func]
    fn new_game(&mut self, ecl_source: GString, seed: i64, rank: i64) -> bool {
        let rank = rank.clamp(i32::MIN as i64, i32::MAX as i64) as i32;
        match boot::boot(&ecl_source.to_string(), seed as u64, rank) {
            Ok(g) => {
                self.game = Some(g);
                self.warned = 0;
                true
            }
            Err(e) => {
                godot_error!("[stg] new_game 失败:{e:?}");
                false
            }
        }
    }

    /// 多单元中段开机(整局流程刀 spec §3/§4 扩口;T1 `compile_units` + T6 `new_game_at`
    /// 桥面落地,冻结面 13→15)。`names`/`sources` 是两条平行数组(GDScript 侧无原生元组
    /// 容器,壳层惯例);长度不等视为调用方违约(P4-b,no-op+false)。装备四标量壳层先
    /// `clamp` 到各自域再收窄——`power` 核内 `new_game_at` 还会钳 `POWER_MAX`,双保险。
    ///
    /// `rank`/`start` 反过来:核内对二者有**值域校验**(`rank` 越 `0..=4` 返
    /// `RankOutOfRange`,`start` 认不出的 mark 返 `UnknownEntry`),所以壳层这里必须
    /// **饱和**而非 `as i32` 截断——`as` 是模 2³² 回绕,GDScript 传 `4294967296` 会截成
    /// `0` 静默以 Easy / 关首开局,把核内那道校验整个绕过去。饱和则把越界值钉在
    /// `i32::MIN/MAX`,仍然越界,核内照样响亮失败。(装备四标量无此问题:它们本就是
    /// "钳到域内"语义,没有可绕过的校验。)
    #[func]
    #[allow(clippy::too_many_arguments)] // gdext #[func] 天然参数面(GString/Rid 类比先例)；GDScript 侧无原生元组/结构体传入，装备四标量+多单元两数组只能平铺
    fn new_game_at(
        &mut self,
        names: PackedStringArray,
        sources: PackedStringArray,
        seed: i64,
        rank: i64,
        start: i64,
        character: i64,
        power: i64,
        lives: i64,
        bombs: i64,
    ) -> bool {
        if names.len() != sources.len() {
            godot_error!(
                "[stg] new_game_at:names/sources 长度不等({} vs {})",
                names.len(),
                sources.len()
            );
            return false;
        }
        let units: Vec<(String, String)> = names
            .as_slice()
            .iter()
            .zip(sources.as_slice().iter())
            .map(|(n, s)| (n.to_string(), s.to_string()))
            .collect();
        let loadout = stg_core::player::Loadout {
            character: character.clamp(0, u8::MAX as i64) as u8,
            power: power.clamp(0, u16::MAX as i64) as u16,
            lives: lives.clamp(0, u8::MAX as i64) as u8,
            bombs: bombs.clamp(0, u8::MAX as i64) as u8,
            // 时停刀（裁定 R-2）：不给桥面加 time_stops 入参——默认值 1 已够，
            // 加参数是没人要的接口扩张。
            ..Default::default()
        };
        let rank = rank.clamp(i32::MIN as i64, i32::MAX as i64) as i32;
        let start = start.clamp(i32::MIN as i64, i32::MAX as i64) as i32;
        match boot::boot_at(&units, seed as u64, rank, start, loadout) {
            Ok(g) => {
                self.game = Some(g);
                self.warned = 0;
                true
            }
            Err(e) => {
                godot_error!("[stg] new_game_at 失败:{e:?}");
                false
            }
        }
    }

    #[func]
    fn step_frame(&mut self, buttons: i64) {
        let Some(game) = self.game.as_mut() else {
            self.warn_once(W_NO_GAME, "step_frame:尚未 new_game,no-op");
            return;
        };
        let mut input = stg_core::input::InputFrame::empty(game.world.frame());
        input.actions[0].buttons = buttons as u32;
        stg_core::step::step_with_director(
            &mut game.world,
            game.tables,
            &game.image,
            &input,
            |_| {},
        );
        // 已注册层:原地编码 + 一次上传 + 可见数
        let frame_no = game.world.frame();
        let mut rs = RenderingServer::singleton();
        for layer in 0..LAYER_COUNT {
            let Some(rid) = self.layers[layer] else {
                continue;
            };
            let n = frame::encode_layer(
                game.world.view(),
                game.tables,
                frame_no,
                layer,
                self.bufs[layer].as_mut_slice(),
            );
            rs.multimesh_set_buffer(rid, &self.bufs[layer]);
            rs.multimesh_set_visible_instances(rid, n as i32);
        }
    }

    /// 校验判据(T5 冻结面行为微调,记文档):`multimesh_get_buffer(rid).len() ==
    /// cap × FLOATS_PER_INSTANCE`,不用 `multimesh_get_instance_count`。理由(实验判决 +
    /// 生产语义双赢):①headless dummy renderer 下 `instance_count` 恒 0(2026-07-25 实验
    /// 判决),合法注册永远被拒、上传链无法冒烟;buffer 尺寸判据在冒烟侧可用 `set_buffer`
    /// 播种定长零缓冲过闸。②`instance_count` 判不出格式错位(3D 格式 multimesh 数量对但
    /// stride 全错仍会放行);buffer 尺寸直接锁死壳依赖的 12 float/实例 stride,比旧判据
    /// 更严而非更松。③`get_buffer` 在真渲染器上是一次性注册期读回,成本可忽略。
    #[func]
    fn register_layer(&mut self, kind: i64, multimesh_rid: Rid) -> bool {
        let layer = kind as usize;
        if layer >= LAYER_COUNT {
            self.warn_once(W_BAD_LAYER, "register_layer:未知层号,no-op");
            return false;
        }
        let cap = frame::layer_cap(layer);
        let rs = RenderingServer::singleton();
        let got = rs.multimesh_get_buffer(multimesh_rid).len();
        let need = cap * FLOATS_PER_INSTANCE;
        if got != need {
            self.warn_once(
                W_BAD_MM,
                "register_layer:multimesh 缓冲尺寸不符(需 cap×12,含坏 RID/未播种;headless 下须先 set_buffer 播种),no-op",
            );
            return false;
        }
        let mut buf = PackedFloat32Array::new();
        buf.resize(cap * FLOATS_PER_INSTANCE);
        self.bufs[layer] = buf;
        self.layers[layer] = Some(multimesh_rid);
        true
    }

    #[func]
    fn take_requests(&mut self) -> Array<VarDictionary> {
        let mut arr = Array::new();
        let Some(game) = self.game.as_ref() else {
            return arr;
        };
        let f = game.world.frame() as i64;
        for r in game.world.take_requests() {
            let mut d = VarDictionary::new();
            d.set("id", r.id as i64);
            d.set("seq", r.seq as i64);
            d.set("frame", f);
            let mut args = Array::<i64>::new();
            for a in r.args {
                args.push(a as i64);
            }
            d.set("args", &args);
            arr.push(&d);
        }
        arr
    }

    #[func]
    fn save_state(&mut self) -> PackedByteArray {
        match self.game.as_ref() {
            Some(g) => PackedByteArray::from(crate::save::save(g).as_slice()),
            None => {
                self.warn_once(W_NO_GAME, "save_state:尚未 new_game,返回空");
                PackedByteArray::new()
            }
        }
    }

    #[func]
    fn load_state(&mut self, bytes: PackedByteArray) -> bool {
        let Some(game) = self.game.as_mut() else {
            self.warn_once(W_NO_GAME, "load_state:尚未 new_game,no-op");
            return false;
        };
        match crate::save::load_into(game, bytes.as_slice()) {
            Ok(()) => true,
            Err(e) => {
                godot_error!("[stg] load_state 失败:{e}(世界原状不动)");
                false
            }
        }
    }

    #[func]
    fn frame(&self) -> i64 {
        self.game.as_ref().map_or(-1, |g| g.world.frame() as i64)
    }

    #[func]
    fn checksum(&self) -> i64 {
        self.game.as_ref().map_or(0, |g| g.world.checksum() as i64)
    }

    #[func]
    fn hud_player(&self) -> VarDictionary {
        let mut d = VarDictionary::new();
        let Some(g) = self.game.as_ref() else {
            return d;
        };
        let p = &g.world.view().players()[0];
        d.set("x", p.x.raw() as f64 / 65536.0);
        d.set("y", p.y.raw() as f64 / 65536.0);
        d.set("lives", p.lives as i64);
        d.set("bombs", p.bombs as i64);
        d.set("life_pieces", p.life_pieces as i64);
        d.set("bomb_pieces", p.bomb_pieces as i64);
        d.set("power", p.power as i64);
        d.set("score", p.score as i64);
        d.set("graze", p.graze as i64);
        d.set("life_state", p.life_state as i64);
        d.set("invuln", p.invuln as i64);
        d.set("facing", p.facing as i64);
        d
    }

    /// 敌人木偶喂料(表现契约 v2 §4.4;纯模块 `crate::puppets`)。返回 Dictionary,每键一条
    /// 压缩列(按池索引升序):`index,gen,sprite,anm_state,state_age,hit_flash` →
    /// `PackedInt32Array`;`x,y` → `PackedFloat32Array`。**无 `dying` 列**(step 后读不到)。
    /// 未开局 → 空字典(同 hud_* 口径)。
    #[func]
    fn puppets(&self) -> VarDictionary {
        let mut d = VarDictionary::new();
        let Some(g) = self.game.as_ref() else {
            return d;
        };
        let c = crate::puppets::encode_puppets(g.world.view(), g.world.frame());
        d.set("index", &PackedInt32Array::from(c.index.as_slice()));
        d.set("gen", &PackedInt32Array::from(c.generation.as_slice()));
        d.set("x", &PackedFloat32Array::from(c.x.as_slice()));
        d.set("y", &PackedFloat32Array::from(c.y.as_slice()));
        d.set("sprite", &PackedInt32Array::from(c.sprite.as_slice()));
        d.set("anm_state", &PackedInt32Array::from(c.anm_state.as_slice()));
        d.set("state_age", &PackedInt32Array::from(c.state_age.as_slice()));
        d.set("hit_flash", &PackedInt32Array::from(c.hit_flash.as_slice()));
        d
    }

    /// 本帧离开池的敌弹(表现契约 v2 §4.5;核内第四条纯输出缓冲 `vanished`):
    /// `x,y` → `PackedFloat32Array`,`sprite` → `PackedInt32Array`,`reason` →
    /// `PackedByteArray`(`VANISH_LIFE`/`VANISH_CLEARED`)。只记场内、越界不记。
    /// **帧内缓冲,下一次 step 的 begin 清空——必须在两次 step 之间取走。**
    #[func]
    fn vanished(&self) -> VarDictionary {
        let mut d = VarDictionary::new();
        let Some(g) = self.game.as_ref() else {
            return d;
        };
        let v = g.world.vanished();
        let xs: Vec<f32> = v.iter().map(|r| r.x.raw() as f32 / 65536.0).collect();
        let ys: Vec<f32> = v.iter().map(|r| r.y.raw() as f32 / 65536.0).collect();
        let sprites: Vec<i32> = v.iter().map(|r| r.sprite as i32).collect();
        let reasons: Vec<u8> = v.iter().map(|r| r.reason).collect();
        d.set("x", &PackedFloat32Array::from(xs.as_slice()));
        d.set("y", &PackedFloat32Array::from(ys.as_slice()));
        d.set("sprite", &PackedInt32Array::from(sprites.as_slice()));
        d.set("reason", &PackedByteArray::from(reasons.as_slice()));
        d
    }

    /// 按句柄读实体位置(表现契约 v2 §4.6):`fx_on` 依附特效的唯一跟随手段。`kind` 目前
    /// 只认敌人(0);句柄有效返 `Vector2`,失效/越界/未开局返 `null`——壳侧据此回收跟随节点。
    #[func]
    fn entity_pos(&self, kind: i64, index: i64, generation: i64) -> Variant {
        let Some(g) = self.game.as_ref() else {
            return Variant::nil();
        };
        if kind != 0 {
            return Variant::nil();
        }
        let (Ok(index), Ok(generation)) = (u16::try_from(index), u16::try_from(generation)) else {
            return Variant::nil();
        };
        let p = g.world.view().enemies();
        let h = stg_core::enemy::EnemyHandle { index, generation };
        match p.get(h) {
            Some(i) => Vector2::new(
                p.x()[i].raw() as f32 / 65536.0,
                p.y()[i].raw() as f32 / 65536.0,
            )
            .to_variant(),
            None => Variant::nil(),
        }
    }

    #[func]
    fn hud_boss(&self, i: i64) -> VarDictionary {
        let mut d = VarDictionary::new();
        let Some(g) = self.game.as_ref() else {
            return d;
        };
        let Some(s) = g.world.view().boss_ui().get(i as usize) else {
            return d;
        };
        d.set("active", s.active as i64);
        d.set("hp_ratio", s.hp_ratio.raw() as f64 / 65536.0);
        d.set("spell_id", s.spell_id as i64);
        d.set("timer_frames", s.timer_frames as i64);
        d.set("phase_left", s.phase_left as i64);
        d
    }

    #[func]
    fn hud_spell(&self, i: i64) -> VarDictionary {
        let mut d = VarDictionary::new();
        let Some(g) = self.game.as_ref() else {
            return d;
        };
        let Some(s) = g.world.view().spells().get(i as usize) else {
            return d;
        };
        d.set("active", s.active as i64);
        d.set("spell_id", s.spell_id as i64);
        d.set("frames_left", s.frames_left as i64);
        d.set("bonus_now", s.bonus_now as i64);
        d.set("capture_ok", s.capture_ok as i64);
        d.set("flags", s.flags as i64);
        d
    }

    /// 表现锚点四读口(整局流程刀 spec §4 扩口;`WorldView` 四方法直转发)。未开局同
    /// `hud_*` 既有口径:返回空字典而非零填充,GDScript 侧靠 `Dictionary.has()`/`get()`
    /// 默认值区分"未开局"与"锚点仍是初值 0"。
    #[func]
    fn anchors(&self) -> VarDictionary {
        let mut d = VarDictionary::new();
        let Some(g) = self.game.as_ref() else {
            return d;
        };
        let v = g.world.view();
        d.set("bgm", v.bgm_id() as i64);
        d.set("bg", v.bg_id() as i64);
        d.set("bg_phase", v.bg_phase() as i64);
        d.set("bg_phase_frame", v.bg_phase_frame() as i64);
        d
    }

    #[func]
    fn player_pos(&self) -> Vector2 {
        let Some(g) = self.game.as_ref() else {
            return Vector2::ZERO;
        };
        let p = &g.world.view().players()[0];
        Vector2::new(p.x.raw() as f32 / 65536.0, p.y.raw() as f32 / 65536.0)
    }

    #[func]
    fn fields_info(&self) -> Array<VarDictionary> {
        let mut arr = Array::new();
        let Some(g) = self.game.as_ref() else {
            return arr;
        };
        let view = g.world.view();
        let p = view.fields();
        let (xs, ys, rads, lives) = (p.x(), p.y(), p.radius(), p.life());
        for i in p.iter_alive() {
            let mut d = VarDictionary::new();
            d.set("x", xs[i].raw() as f64 / 65536.0);
            d.set("y", ys[i].raw() as f64 / 65536.0);
            d.set("radius", rads[i].raw() as f64 / 65536.0);
            d.set("life", lives[i] as i64);
            arr.push(&d);
        }
        arr
    }

    /// 本帧世界大事记（通道 A 的批量事实出口）。每条：
    /// `kind`（`stg_core::events::EVT_*`）/ `x`,`y`（世界坐标，已转浮点）/
    /// `a_index`,`a_gen`（相关实体句柄）/ `data0`,`data1`（逐 kind 约定，见 `events.rs`）。
    ///
    /// **生命周期**：帧内缓冲，下一次 `step` 的 `begin` 清空——必须在两次 step 之间取走。
    /// 与通道 B 的 `take_requests` 是**不同的东西**：请求是脚本/引擎主动发的离散演出指令
    /// （引擎保留 1..=63），事件是世界每帧产出的**事实流**（敌死 / 自机死 / 消弹 / 拾取 /
    /// 符卡宣言收卡失败 / 任务 fault / 自机弹命中）。表现层要"跟着世界发生的事做反应"
    /// （火花、音效、伤害数字）走这条。
    #[func]
    fn frame_events(&self) -> Array<VarDictionary> {
        let mut arr = Array::new();
        let Some(g) = self.game.as_ref() else {
            return arr;
        };
        for ev in g.world.frame_events() {
            let mut d = VarDictionary::new();
            d.set("kind", ev.kind as i64);
            d.set("x", ev.x.raw() as f64 / 65536.0);
            d.set("y", ev.y.raw() as f64 / 65536.0);
            d.set("a_index", ev.a_index as i64);
            d.set("a_gen", ev.a_gen as i64);
            d.set("data0", ev.data[0] as i64);
            d.set("data1", ev.data[1] as i64);
            arr.push(&d);
        }
        arr
    }

    #[func]
    fn ping(&self) -> i64 {
        42
    }
}
