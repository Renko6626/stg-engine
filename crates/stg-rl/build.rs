//! 把 `godot/ecl/game/*.ecl` 按文件名排序嵌进 crate（spec 偏差 Ruling 1）。
use std::{env, fs, path::PathBuf};

fn main() {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../godot/ecl/game");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut files: Vec<_> = fs::read_dir(&root)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "ecl"))
        .collect();
    files.sort();
    let mut out = String::from("pub static GAME: &[(&str, &str)] = &[\n");
    for p in &files {
        println!("cargo:rerun-if-changed={}", p.display());
        let name = p.file_name().unwrap().to_str().unwrap();
        out.push_str(&format!(
            "    ({name:?}, include_str!({:?})),\n",
            p.canonicalize().unwrap()
        ));
    }
    out.push_str("];\n");
    fs::write(
        PathBuf::from(env::var("OUT_DIR").unwrap()).join("bundled.rs"),
        out,
    )
    .unwrap();
}
