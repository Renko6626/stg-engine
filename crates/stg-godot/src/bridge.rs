//! gdext 壳：WorldBridge 节点。纯胶水零逻辑（spec §12）。

use godot::prelude::*;

#[derive(GodotClass)]
#[class(base=Node)]
pub struct WorldBridge {
    base: Base<Node>,
}

#[godot_api]
impl INode for WorldBridge {
    fn init(base: Base<Node>) -> Self {
        WorldBridge { base }
    }
}

#[godot_api]
impl WorldBridge {
    /// 加载冒烟探针：注册即 42。
    #[func]
    fn ping(&self) -> i64 {
        42
    }
}
