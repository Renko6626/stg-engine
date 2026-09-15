//! stg-rl —— 强化学习批量 env 核心（spec `docs/superpowers/specs/2026-09-15-stg-rl-env-design.md`）。
//! 断层线以上：允许线程 / rayon；stg-core 语义不变。

pub mod layout;

pub mod bundled {
    include!(concat!(env!("OUT_DIR"), "/bundled.rs"));

    /// 内置 ECL 内容包（按文件名排序）。v1 只有 `"game"`（`godot/ecl/game`）。
    pub fn bundled_sources(pack: &str) -> Option<&'static [(&'static str, &'static str)]> {
        match pack {
            "game" => Some(GAME),
            _ => None,
        }
    }
}
