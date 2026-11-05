//! 开局确定的碰撞规则；独立于角色装备与贴图，随存档、快照及回放保存。

use crate::save::{LoadError, SaveBytes, SaveReader};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum CollisionMode {
    Legacy = 0,
    Modern = 1,
}

impl std::str::FromStr for CollisionMode {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "LEGACY" => Ok(Self::Legacy),
            "MODERN" => Ok(Self::Modern),
            _ => Err("collision_mode must be LEGACY or MODERN"),
        }
    }
}

/// 字段用字节存储，保持 World 全零合法；外部通过构造函数传入合法配置。
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, crate::checksum::Checksum)]
pub struct CollisionRules {
    mode: u8,
    graze_enabled: u8,
}

impl CollisionRules {
    pub const fn new(mode: CollisionMode, graze_enabled: bool) -> Self {
        Self {
            mode: mode as u8,
            graze_enabled: graze_enabled as u8,
        }
    }

    pub const fn mode(self) -> CollisionMode {
        if self.mode == 0 {
            CollisionMode::Legacy
        } else {
            CollisionMode::Modern
        }
    }

    pub const fn graze_enabled(self) -> bool {
        self.graze_enabled != 0
    }

    pub(crate) fn from_wire(mode: u8, graze_enabled: u8) -> Option<Self> {
        (mode <= 1 && graze_enabled <= 1).then_some(Self {
            mode,
            graze_enabled,
        })
    }
}

impl Default for CollisionRules {
    fn default() -> Self {
        Self::new(CollisionMode::Legacy, true)
    }
}

impl SaveBytes for CollisionRules {
    fn write_bytes(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&[self.mode, self.graze_enabled]);
    }

    fn read_bytes(&mut self, r: &mut SaveReader<'_>) -> Result<(), LoadError> {
        let b = r.take(2)?;
        *self = Self::from_wire(b[0], b[1]).ok_or(LoadError::InvalidCollisionRules)?;
        Ok(())
    }
}
