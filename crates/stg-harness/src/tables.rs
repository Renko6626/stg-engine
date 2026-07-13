//! 烘焙表生成器（断层线【以上】，用 f64）。生成 → 与 commit 字节比对（verify）或写盘（bake）。
//! 取整一律 `round_ties_even`（round-half-to-even，§2.1）；输出小端字节。

use std::f64::consts::PI;
use std::path::PathBuf;

/// 表字节目录（cwd 无关：相对 harness crate 清单定位到 stg-core）。
pub const TABLES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../stg-core/src/math/tables");

fn table_path(name: &str) -> PathBuf {
    PathBuf::from(TABLES_DIR).join(name)
}

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

/// 表生成器函数类型（消 clippy::type_complexity）。
type TableGen = fn() -> Vec<u8>;

/// 所有表的 (文件名, 生成器) 清单——bake 与 verify 共用单一真相源。
fn registry() -> Vec<(&'static str, TableGen)> {
    vec![("sin_quarter.bin", gen_sin_quarter as TableGen)]
}

/// 生成全部表并写入 stg-core 源目录（开发者刻意重烘时用）。
pub fn bake_all() -> std::io::Result<()> {
    std::fs::create_dir_all(TABLES_DIR)?;
    for (name, generate) in registry() {
        let bytes = generate();
        std::fs::write(table_path(name), &bytes)?;
        eprintln!("baked {name} ({} bytes)", bytes.len());
    }
    Ok(())
}

/// 重新生成并与 commit 的字节逐位比对（CI 防漂移）。
pub fn verify_all() -> Result<(), String> {
    for (name, generate) in registry() {
        let expected = generate();
        let path = table_path(name);
        let actual =
            std::fs::read(&path).map_err(|e| format!("读取 {} 失败: {e}", path.display()))?;
        if actual != expected {
            return Err(format!(
                "{name} 与 commit 字节不一致（生成 {} 字节 vs commit {} 字节）",
                expected.len(),
                actual.len()
            ));
        }
        eprintln!("verified {name} ({} bytes)", expected.len());
    }
    Ok(())
}
