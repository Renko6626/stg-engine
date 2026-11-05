//! 烘焙表生成器（断层线【以上】，用 f64）。生成 → 与 commit 字节比对（verify）或写盘（bake）。
//! 取整一律 `round_ties_even`（round-half-to-even，§2.1）；输出小端字节。

use std::f64::consts::PI;
use std::path::PathBuf;

/// 表字节目录（cwd 无关：相对 harness crate 清单定位到 stg-core）。
pub const TABLES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../stg-core/src/math/tables");

/// 世界数据表目录（stg-core/src/tables）。
pub const WORLD_TABLES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../stg-core/src/tables");

/// sin 四分之一波：`[i32; 16385]`，`v[i] = round_ties_even(sin(i·(π/2)/16384)·65536)`，i ∈ 0..=16384。
/// 含端点：`v[16384] = 65536`（= 1.0），供四象限重建在 π/2 边界精确。
pub fn gen_sin_quarter() -> Vec<u8> {
    let mut out = Vec::with_capacity(16385 * 4);
    for i in 0..=16384i64 {
        let theta = (i as f64) * (PI / 2.0) / 16384.0;
        let v = (theta.sin() * 65536.0).round_ties_even() as i32;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// 每条曲线257个Q16.16采样；输出保留反向／超调。旧0–7行公式和字节不变。
pub fn gen_easing() -> Vec<u8> {
    use stg_core::math::easing::Easing;

    let mut out = Vec::with_capacity(Easing::ALL.len() * 257 * 4);
    for curve in Easing::ALL {
        for i in 0..=256i64 {
            let t = (i as f64) / 256.0;
            let value = match curve {
                Easing::Linear => t,
                Easing::QuadIn => t * t,
                Easing::QuadOut => 1.0 - (1.0 - t) * (1.0 - t),
                Easing::QuadInOut => {
                    if t < 0.5 {
                        2.0 * t * t
                    } else {
                        1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
                    }
                }
                Easing::CubicIn => t * t * t,
                Easing::CubicOut => 1.0 - (1.0 - t).powi(3),
                Easing::CubicInOut => {
                    if t < 0.5 {
                        4.0 * t * t * t
                    } else {
                        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
                    }
                }
                Easing::Smoothstep => t * t * (3.0 - 2.0 * t),
                Easing::QuartIn => t.powi(4),
                Easing::QuartOut => 1.0 - (1.0 - t).powi(4),
                Easing::QuartInOut => {
                    if t < 0.5 {
                        8.0 * t.powi(4)
                    } else {
                        1.0 - (2.0 - 2.0 * t).powi(4) / 2.0
                    }
                }
                Easing::QuadOutIn => out_in(t, 2),
                Easing::CubicOutIn => out_in(t, 3),
                Easing::QuartOutIn => out_in(t, 4),
                Easing::Hold => 0.0,
                Easing::StepStart => 1.0,
                Easing::SineIn => 1.0 - (PI * t / 2.0).cos(),
                Easing::SineOut => (PI * t / 2.0).sin(),
                Easing::SineInOut => (1.0 - (PI * t).cos()) / 2.0,
                Easing::SineOutIn => {
                    if t < 0.5 {
                        (PI * t).sin() / 2.0
                    } else {
                        1.0 - (PI * t).sin() / 2.0
                    }
                }
                Easing::BackInP25 => back(t, 0.25),
                Easing::BackInP30 => back(t, 0.30),
                Easing::BackInP35 => back(t, 0.35),
                Easing::BackInP38 => back(t, 0.38),
                Easing::BackInP40 => back(t, 0.40),
                Easing::BackOutP25 => 1.0 - back(1.0 - t, 0.25),
                Easing::BackOutP30 => 1.0 - back(1.0 - t, 0.30),
                Easing::BackOutP35 => 1.0 - back(1.0 - t, 0.35),
                Easing::BackOutP38 => 1.0 - back(1.0 - t, 0.38),
                Easing::BackOutP40 => 1.0 - back(1.0 - t, 0.40),
            };
            let v = (value * 65536.0).round_ties_even() as i32;
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    out
}

fn out_in(t: f64, power: i32) -> f64 {
    if t < 0.5 {
        (1.0 - (1.0 - 2.0 * t).powi(power)) / 2.0
    } else {
        0.5 + (2.0 * t - 1.0).powi(power) / 2.0
    }
}

fn back(t: f64, extremum: f64) -> f64 {
    (t * t - 2.0 * extremum * t) / (1.0 - 2.0 * extremum)
}

/// CORDIC 角度常数：`atan(2^-i)`（弧度）转 BAM，16 项 u16。
pub fn gen_atan_cordic() -> Vec<u8> {
    let mut out = Vec::with_capacity(16 * 2);
    for i in 0..16i32 {
        let rad = (2f64.powi(-i)).atan();
        let bam = (rad / (2.0 * PI) * 65536.0).round_ties_even() as i64;
        out.extend_from_slice(&(bam as u16).to_le_bytes());
    }
    out
}

/// 世界内容表 v0：owned 构造 → 规范字节（单一真相源 = `build_tables_v0`）。
pub fn gen_world_tables_v0() -> Vec<u8> {
    stg_core::tables::build_tables_v0().to_bytes()
}

/// 表生成器函数类型（消 clippy::type_complexity）。
type TableGen = fn() -> Vec<u8>;

/// 所有表的 (完整路径, 生成器) 清单——bake 与 verify 共用单一真相源。
fn registry() -> Vec<(PathBuf, TableGen)> {
    let math = PathBuf::from(TABLES_DIR);
    let world = PathBuf::from(WORLD_TABLES_DIR);
    vec![
        (math.join("sin_quarter.bin"), gen_sin_quarter as TableGen),
        (math.join("easing.bin"), gen_easing as TableGen),
        (math.join("atan_cordic.bin"), gen_atan_cordic as TableGen),
        (world.join("tables_v0.bin"), gen_world_tables_v0 as TableGen),
    ]
}

/// 生成全部表并写入 stg-core 源目录（开发者刻意重烘时用）。
pub fn bake_all() -> std::io::Result<()> {
    for (path, generate) in registry() {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let bytes = generate();
        std::fs::write(&path, &bytes)?;
        eprintln!("baked {} ({} bytes)", path.display(), bytes.len());
    }
    Ok(())
}

/// 重新生成并与 commit 的字节逐位比对（CI 防漂移）。
pub fn verify_all() -> Result<(), String> {
    for (path, generate) in registry() {
        let expected = generate();
        let actual =
            std::fs::read(&path).map_err(|e| format!("读取 {} 失败: {e}", path.display()))?;
        if actual != expected {
            return Err(format!(
                "{} 与 commit 字节不一致（生成 {} vs commit {} 字节）",
                path.display(),
                expected.len(),
                actual.len()
            ));
        }
        eprintln!("verified {} ({} bytes)", path.display(), expected.len());
    }
    Ok(())
}

#[cfg(test)]
mod table_disk_load_tests {
    /// D3 端到端自证（C11 收尾）：从磁盘 `std::fs::read` + `from_bytes` 载入已 commit 的
    /// `tables_v0.bin`，与 `include_bytes!` 内建 `TABLES_V0`（同一份字节）各自建 World 跑
    /// 120 帧，逐帧校验和须一致。二者字节相同不是巧合——本测试证的是**加载路径**：
    /// disk → `from_bytes` → `new_with_tables` → `step` 这条环真的产出可运行、位级一致的
    /// World，而不仅是"字节比对"。
    #[test]
    fn table_loaded_from_disk_runs_identically_to_builtin() {
        // 注：`World::checksum()` 是 `step.rs` 上的一个便利**固有方法**（内部转发到
        // `checksum::Checksum` trait），固有方法在方法解析中优先于同名 trait 方法命中——
        // 故此处不需要（也不能，否则 `-D warnings` 治下 unused_imports 报错）
        // `use stg_core::checksum::Checksum;` 把 trait 带入作用域。
        use stg_core::ecl::image::EclImage;
        use stg_core::input::{BTN_LEFT, BTN_SHOT, BTN_UP, InputFrame};

        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../stg-core/src/tables/tables_v0.bin"
        );
        let bytes = std::fs::read(path).expect("committed tables_v0.bin exists");
        let loaded = stg_core::tables::WorldTables::from_bytes(&bytes).expect("disk table loads");

        let mut w_disk = stg_core::World::new_with_tables(0xC11, &loaded);
        let mut w_builtin = stg_core::World::new(0xC11);
        let ecl = EclImage::empty();
        for frame in 0..120u32 {
            // 复审强化（非 idle 输入）：全程持 SHOT + 左上对角移动——让
            // `char0_update_shot` 真正读 `tables.characters[..].shot`（发弹，命中
            // `debug_assert!(shooter.interval != 0)`）、`move_player` 真正读移动
            // `CharacterCfg` 参数（高/低速、对角归一），二者都把加载表的数值喂进
            // 校验和覆盖的世界状态——而不只是出生期的 `hit_radius`/`graze_radius`。
            // 两侧逐帧同一个 `input` 值，磁盘/内建两条腿除表来源外别无二致。
            let mut input = InputFrame::empty(frame);
            input.actions[0].buttons = BTN_SHOT | BTN_LEFT | BTN_UP;
            stg_core::step(&mut w_disk, &loaded, &ecl, &input);
            stg_core::step(&mut w_builtin, &stg_core::tables::TABLES_V0, &ecl, &input);
        }
        assert_eq!(
            w_disk.checksum(),
            w_builtin.checksum(),
            "磁盘加载表与内建表逐帧一致——文件加载闭环自证"
        );
        // 非退化守卫：证明上面不是一次「全程 idle」的空转——校验和非零、且真的发出了
        // 子弹（`char0_update_shot` 读到的 `interval=4` 的 shooter 在 120 帧内必命中
        // 多次），两侧防止本测试悄悄退化回「只比对出生期字段」。
        assert_ne!(w_disk.checksum(), 0, "非退化守卫：校验和不应为零");
        // 自机弹住 `shots` 池（`bullets` 是敌方弹池，D7/D8 分池）。
        let shot_count = w_disk.body.view().shots().iter_alive().count();
        assert!(
            shot_count > 0,
            "非退化守卫：120 帧全程持 SHOT 应产出自机弹（实测 {shot_count}）"
        );
    }
}
