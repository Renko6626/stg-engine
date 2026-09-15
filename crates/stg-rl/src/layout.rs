//! proto v1 表布局（逐字照 stg-agent-proto `c/sa_layout.h`；夹具测试 `tests/layout.rs` 押运）。

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldType {
    Fx,
    Angle,
    U8,
    U16,
    U32,
    I32,
}

impl FieldType {
    pub fn name(self) -> &'static str {
        match self {
            Self::Fx => "fx",
            Self::Angle => "angle",
            Self::U8 => "u8",
            Self::U16 => "u16",
            Self::U32 => "u32",
            Self::I32 => "i32",
        }
    }
    pub fn size(self) -> usize {
        match self {
            Self::U8 => 1,
            Self::Angle | Self::U16 => 2,
            Self::Fx | Self::U32 | Self::I32 => 4,
        }
    }
}

pub struct FieldDef {
    pub name: &'static str,
    pub ty: FieldType,
    pub off: usize,
}

pub struct TableDef {
    pub id: u8,
    pub name: &'static str,
    pub stride: usize,
    pub fields: &'static [FieldDef],
}

macro_rules! f {
    ($n:literal, $t:ident, $o:expr) => {
        FieldDef {
            name: $n,
            ty: FieldType::$t,
            off: $o,
        }
    };
}

pub mod off {
    pub mod player {
        pub const X: usize = 0;
        pub const Y: usize = 4;
        pub const HIT_R: usize = 8;
        pub const SPEED: usize = 12;
        pub const SPEED_F: usize = 16;
        pub const FOCUS: usize = 20;
        pub const STATE: usize = 21;
        pub const LIVES: usize = 22;
        pub const BOMBS: usize = 23;
        pub const LFRAG: usize = 24;
        pub const BFRAG: usize = 25;
        pub const POWER: usize = 26;
        pub const SCORE: usize = 28;
        pub const GRAZE: usize = 32;
    }
    pub mod bullet {
        pub const X: usize = 0;
        pub const Y: usize = 4;
        pub const VX: usize = 8;
        pub const VY: usize = 12;
        pub const SPEED: usize = 16;
        pub const ANGLE: usize = 20;
        pub const RADIUS: usize = 22;
        pub const FLAGS: usize = 26;
        pub const STATE: usize = 27;
        pub const TYPE: usize = 28;
    }
    pub mod enemy {
        pub const X: usize = 0;
        pub const Y: usize = 4;
        pub const HURT_W: usize = 8;
        pub const HURT_H: usize = 12;
        pub const HIT_W: usize = 16;
        pub const HIT_H: usize = 20;
        pub const HP: usize = 24;
        pub const HP_MAX: usize = 28;
        pub const FLAGS: usize = 32;
        pub const ID: usize = 34;
    }
    pub mod laser {
        pub const X: usize = 0;
        pub const Y: usize = 4;
        pub const ANGLE: usize = 8;
        pub const START: usize = 10;
        pub const END: usize = 14;
        pub const START_LEN: usize = 18;
        pub const SPEED: usize = 22;
        pub const HALF_H: usize = 26;
        pub const OMEGA: usize = 30;
        pub const VX: usize = 34;
        pub const VY: usize = 38;
        pub const T_ACTIVE: usize = 42;
        pub const STATE: usize = 46;
        pub const TYPE: usize = 47;
    }
    pub mod item {
        pub const X: usize = 0;
        pub const Y: usize = 4;
        pub const VX: usize = 8;
        pub const VY: usize = 12;
        pub const KIND: usize = 16;
        pub const FLAGS: usize = 17;
    }
}

pub const PLAYER: TableDef = TableDef {
    id: 1,
    name: "player",
    stride: 36,
    fields: &[
        f!("x", Fx, 0),
        f!("y", Fx, 4),
        f!("hit_radius", Fx, 8),
        f!("speed", Fx, 12),
        f!("speed_focus", Fx, 16),
        f!("focus", U8, 20),
        f!("state", U8, 21),
        f!("lives", U8, 22),
        f!("bombs", U8, 23),
        f!("life_frags", U8, 24),
        f!("bomb_frags", U8, 25),
        f!("power", U16, 26),
        f!("score", U32, 28),
        f!("graze", U32, 32),
    ],
};
pub const BULLETS: TableDef = TableDef {
    id: 2,
    name: "bullets",
    stride: 30,
    fields: &[
        f!("x", Fx, 0),
        f!("y", Fx, 4),
        f!("vx", Fx, 8),
        f!("vy", Fx, 12),
        f!("speed", Fx, 16),
        f!("angle", Angle, 20),
        f!("radius", Fx, 22),
        f!("flags", U8, 26),
        f!("state", U8, 27),
        f!("type", U16, 28),
    ],
};
pub const ENEMIES: TableDef = TableDef {
    id: 3,
    name: "enemies",
    stride: 38,
    fields: &[
        f!("x", Fx, 0),
        f!("y", Fx, 4),
        f!("hurt_w", Fx, 8),
        f!("hurt_h", Fx, 12),
        f!("hit_w", Fx, 16),
        f!("hit_h", Fx, 20),
        f!("hp", I32, 24),
        f!("hp_max", I32, 28),
        f!("flags", U16, 32),
        f!("id", U32, 34),
    ],
};
pub const LASERS: TableDef = TableDef {
    id: 4,
    name: "lasers",
    stride: 48,
    fields: &[
        f!("x", Fx, 0),
        f!("y", Fx, 4),
        f!("angle", Angle, 8),
        f!("start", Fx, 10),
        f!("end", Fx, 14),
        f!("start_len", Fx, 18),
        f!("speed", Fx, 22),
        f!("half_h", Fx, 26),
        f!("omega", Fx, 30),
        f!("vx", Fx, 34),
        f!("vy", Fx, 38),
        f!("t_active", I32, 42),
        f!("state", U8, 46),
        f!("type", U8, 47),
    ],
};
pub const ITEMS: TableDef = TableDef {
    id: 5,
    name: "items",
    stride: 18,
    fields: &[
        f!("x", Fx, 0),
        f!("y", Fx, 4),
        f!("vx", Fx, 8),
        f!("vy", Fx, 12),
        f!("kind", U8, 16),
        f!("flags", U8, 17),
    ],
};
pub const TABLES: [&TableDef; 5] = [&PLAYER, &BULLETS, &ENEMIES, &LASERS, &ITEMS];

pub const ENEMIES_CAP: usize = 256;
pub const LASERS_CAP: usize = 64;
pub const ITEMS_CAP: usize = 1024;
pub const BULLETS_CAP_DEFAULT: usize = 1024;
pub const BULLETS_CAP_MAX: usize = 8192;
pub const ACTION_MASK: u32 = 0x7F;
pub const EVENT_COLUMNS: [&str; 8] = [
    "died",
    "graze",
    "score",
    "enemies_killed",
    "shot_hits",
    "bombs_used",
    "items_picked",
    "segment_end",
];
const ACTION_NAMES: [&str; 7] = ["UP", "DOWN", "LEFT", "RIGHT", "SHOT", "BOMB", "SLOW"];

/// proto HELLO JSON（键序同 C `sa_hello_json`）。`bullets_cap` 须已由调用方校验在 `1..=BULLETS_CAP_MAX`。
pub fn hello_json(bullets_cap: usize) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    write!(
        s,
        "{{\"proto\":1,\"backend\":\"stg-engine@{}\",\"policy\":\"rl\",\"obs_timing\":\"prev-frame-final\",\"tick_hz\":60,",
        stg_core::ENGINE_VER
    )
    .unwrap();
    s.push_str("\"field\":{\"half_w\":192,\"height\":448,\"origin\":\"center-top\",\"move_area\":{\"xmin\":-192,\"xmax\":192,\"ymin\":0,\"ymax\":448}},");
    s.push_str("\"number\":{\"fx\":\"q16.16-i32-le\",\"angle\":\"bam-u16-le\"},\"actions\":[");
    for (i, n) in ACTION_NAMES.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        write!(s, "{{\"bit\":{i},\"name\":\"{n}\"}}").unwrap();
    }
    s.push_str("],\"tables\":[");
    let caps = [1, bullets_cap, ENEMIES_CAP, LASERS_CAP, ITEMS_CAP];
    for (i, t) in TABLES.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        write!(
            s,
            "{{\"id\":{},\"name\":\"{}\",\"cap\":{},\"stride\":{},\"fields\":[",
            t.id, t.name, caps[i], t.stride
        )
        .unwrap();
        for (j, f) in t.fields.iter().enumerate() {
            if j > 0 {
                s.push(',');
            }
            write!(
                s,
                "{{\"name\":\"{}\",\"type\":\"{}\",\"off\":{}}}",
                f.name,
                f.ty.name(),
                f.off
            )
            .unwrap();
        }
        s.push_str("]}");
    }
    s.push_str("]}");
    s
}
