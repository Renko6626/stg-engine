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
    bufs: [Vec<f32>; LAYER_COUNT],
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
    const LAYER_BULLETS: i64 = frame::LAYER_BULLETS as i64;
    #[constant]
    const LAYER_SHOTS: i64 = frame::LAYER_SHOTS as i64;
    #[constant]
    const LAYER_ENEMIES: i64 = frame::LAYER_ENEMIES as i64;
    #[constant]
    const LAYER_ITEMS: i64 = frame::LAYER_ITEMS as i64;

    #[func]
    fn new_game(&mut self, ecl_source: GString, seed: i64, rank: i64) -> bool {
        match boot::boot(&ecl_source.to_string(), seed as u64, rank as i32) {
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
        };
        match boot::boot_at(&units, seed as u64, rank as i32, start as i32, loadout) {
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
        // 已注册层:编码 + 一次上传 + 可见数
        let mut rs = RenderingServer::singleton();
        for layer in 0..LAYER_COUNT {
            let Some(rid) = self.layers[layer] else {
                continue;
            };
            let n =
                frame::encode_layer(game.world.view(), game.tables, layer, &mut self.bufs[layer]);
            rs.multimesh_set_buffer(rid, &PackedFloat32Array::from(self.bufs[layer].as_slice()));
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
        self.bufs[layer] = vec![0.0; cap * FLOATS_PER_INSTANCE];
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
        d
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

    #[func]
    fn ping(&self) -> i64 {
        42
    }
}
