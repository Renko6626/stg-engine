//! `WorldTables` —— 世界静态数据层（A3）。**owned 形态**（C11）：内部切片 `Box` 拥有，可由
//! `from_bytes` 在运行时反序列化构造，不再是编译期 `&'static`。传递形态不变：**不进 `World`**
//! （I7），`step`/相位按帧以 `&WorldTables` 引用消费。`content_hash`：组 A 恒 0，组 B 由
//! `from_bytes` 计算并校验（LIVE），coherence 守卫读它。

use std::sync::LazyLock;

use crate::items::{ITEM_POINT, ITEM_POWER, ITEM_TYPE_COUNT};
use crate::math::{Angle, Fx};
use crate::world::MAX_ENTITY_RADIUS;

/// 单张掉落表：`(item_type, count)` 行列表。type alias（非 newtype，结构与
/// `Box<[(u8, u8)]>` 完全等价）——纯为压 `clippy::type_complexity`，不改变字段实际类型。
pub type DropTable = Box<[(u8, u8)]>;

/// 全局静态数据层（A3；owned）。见模块文档「传递形态」。
#[derive(Debug, PartialEq, Eq)]
pub struct WorldTables {
    /// 内容哈希：组 B 起 LIVE（`from_bytes` 算 body 的 FNV-1a64 并自校）；组 A 恒 0。
    pub content_hash: u64,
    /// v0 一个角色；定长数组（引擎固定计数，多角色=未来）。
    pub characters: [CharacterCfg; 1],
    pub item_cfg: [ItemTypeCfg; ITEM_TYPE_COUNT],
    pub drop_tables: Box<[DropTable]>,
    pub item_gravity: Fx,
    /// 每种弹型占的连续色数（= 图集列数）。内建 = 16；mod 表自定义。
    /// **引擎不得硬编码这个数**——一切形/色判据从这里读（spec §2 硬约束一）。
    pub color_stride: u16,
    /// 弹外观表（索引 = appearance id = 图集格号 = 池 sprite 值，identity）。
    pub appearances: Box<[AppearanceCfg]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AppearanceCfg {
    pub radius: Fx,
    pub sprite: u16,
    /// 该格图集里是否真有图。`false` = 空格：创建被拒（P4-b Fault），
    /// **不是**"半径为 0 的弹"——空格行照样带本形状的半径，见 spec §4.3。
    pub valid: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemTypeCfg {
    pub score: u32,
    pub eject_speed: Fx,
    pub terminal_vy: Fx,
    pub magnet_speed: Fx,
    pub pickup_radius: Fx,
    pub attract_radius: Fx,
    /// 贴图索引(A1,2026-07-24):`item_type→贴图` 的单一真相源。**断层线以下无人读**
    /// ——渲染期消费者 join(池列方案否决评审记录:spec 2026-07-24 §2.1)。
    pub sprite: u16,
}

/// 单角色配置。owned 化后含 `ShotTypeCfg`（有 `Box`）→ **去 `Copy`、留 `Clone`**。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharacterCfg {
    pub high_speed: Fx,
    pub low_speed: Fx,
    pub inv_sqrt2: Fx,
    pub hit_radius: Fx,
    pub graze_radius: Fx,
    pub shot: ShotTypeCfg,
    /// 该角色的 bomb 描述（自机能力刀）。含 `Box` ⇒ `CharacterCfg` 继续"去 `Copy`、留 `Clone`"。
    pub bomb: BombCfg,
}

/// shottype 表：5 档 × 2 焦点 = 10 槽。owned 化后每槽独立 `Box`（**去 `Copy`、留 `Clone`**）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShotTypeCfg {
    /// `[tier 0..=4][focus 0/1]`；owned 后两焦点槽各持独立分配、内容相等（不再 `ptr::eq` 同一）。
    pub sets: [[Box<[Shooter]>; 2]; 5],
    /// 每档子机偏移（`option` 号 1..=len 查此表；v0 前四档空）。
    pub option_pos: [Box<[(Fx, Fx)]>; 5],
}

/// 一发 bomb 的完整描述（**静态数据**，住 `WorldTables`、不进 `World`——与 `ShotTypeCfg`
/// 同构，M0-17 立下的先例）。把"铺哪些 field / 多久 / 吸不吸道具"做成数据而非代码，是
/// 为了让将来的 bomb 变体成为**换表**而不是改引擎；这不违反 P5（数据不是回调）。
///
/// ⚠️ **数据变不出新形状**：`FieldPool` 只有圆。"锁定敌人的 bomb"只需加一个
/// [`BombOrigin`] 变体（跟随 = 上层每帧重铺 `life = 1`，是 `FieldPool` 设计时就写好的
/// 用法）；但"激光形状的 bomb"必须给 field 加形状字段并改碰撞行 6/7 —— 那是**改碰撞
/// 矩阵**，过评审、另开一刀（spec §13）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BombCfg {
    /// 效果时长（帧）。
    pub frames: u16,
    /// 无敌帧。**允许 > `frames`**：那正是"防炸完立刻死"的旋钮，调它不改任何结构。
    pub invuln: u16,
    /// 起爆当帧是否全屏吸道具。
    pub attract_items: bool,
    /// 起爆时铺的作用区，**按声明序**（I4）。
    pub fields: Box<[BombField]>,
}

/// bomb 铺的一条作用区。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BombField {
    pub origin: BombOrigin,
    pub radius: Fx,
    /// `FIELD_CLEAR_BULLETS` | `FIELD_DAMAGE` 的组合。
    pub flags: u8,
    pub dmg_per_frame: u16,
    pub life: u16,
}

/// 作用区圆心的来源。**用枚举而非 bool**，并在起爆处以穷尽 `match` 消费：加变体而忘了
/// 处理 ⇒ **编译不过**（D18 立下的押运手法）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BombOrigin {
    /// 场心（全屏效果用）。
    FieldCenter,
    /// **起爆那一帧**的自机位置，之后不动（裁定 #10：不跟随）。
    PlayerAtCast,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shooter {
    pub interval: u16,
    pub delay: u16,
    pub dx: Fx,
    pub dy: Fx,
    pub angle: Angle,
    pub speed: Fx,
    pub damage: u16,
    pub radius: Fx,
    pub sprite: u16,
    pub option: u8,
    pub flags: u8,
}

// ── v0 内容基元（保持现值逐字节不变）─────────────────────────────────────────

const BASE_SHOOTER: Shooter = Shooter {
    interval: 4,
    delay: 0,
    dx: Fx::ZERO,
    dy: Fx::ZERO,
    angle: Angle(49152),
    speed: Fx::from_int(12),
    damage: 1,
    radius: Fx::from_int(4),
    sprite: 0,
    option: 0,
    flags: 0,
};

const ITEM_GRAVITY_V0: Fx = Fx::from_raw(9_830);

const STD_ITEM: ItemTypeCfg = ItemTypeCfg {
    score: 0,
    eject_speed: Fx::from_int(3),
    terminal_vy: Fx::from_raw(144_179),
    magnet_speed: Fx::from_int(8),
    pickup_radius: Fx::from_int(16),
    attract_radius: Fx::from_int(40),
    sprite: 0,
};

const ITEM_CFG_V0: [ItemTypeCfg; ITEM_TYPE_COUNT] = [
    ItemTypeCfg {
        score: 10,
        sprite: 0,
        ..STD_ITEM
    }, // POWER
    ItemTypeCfg {
        score: 100,
        sprite: 1,
        ..STD_ITEM
    }, // POINT
    ItemTypeCfg {
        score: 50,
        sprite: 2,
        ..STD_ITEM
    }, // LIFE_PIECE
    ItemTypeCfg {
        score: 50,
        sprite: 3,
        ..STD_ITEM
    }, // BOMB_PIECE
    ItemTypeCfg {
        score: 30,
        sprite: 4,
        ..STD_ITEM
    }, // STAR
];

const CHAR0_HIGH_SPEED: Fx = Fx::from_raw(294_912);
const CHAR0_LOW_SPEED: Fx = Fx::from_raw(131_072);
const CHAR0_INV_SQRT2: Fx = Fx::from_raw(46_341);
const CHAR0_HIT_RADIUS: Fx = Fx::from_raw(163_840);
const CHAR0_GRAZE_RADIUS: Fx = Fx::from_int(16);

/// tier 4 子机出生偏移（唯一非空档）。
const TIER4_OPT: (Fx, Fx) = (Fx::from_int(-20), Fx::from_int(8));

/// v0 全内容 owned 构造（**单一真相源**）：harness 烘焙与运行期 `from_bytes` round-trip 皆以此为准。
/// `content_hash` 置 0（组 B 起由 `from_bytes` 计算填真值）。
pub fn build_tables_v0() -> WorldTables {
    // 各档发射器列表（owned；每次调用新分配，两焦点槽各自持有）。
    let tier_1way = || -> Box<[Shooter]> { Box::new([BASE_SHOOTER]) };
    let tier_2way = || -> Box<[Shooter]> {
        Box::new([
            Shooter {
                dx: Fx::from_int(-8),
                ..BASE_SHOOTER
            },
            Shooter {
                dx: Fx::from_int(8),
                ..BASE_SHOOTER
            },
        ])
    };
    let tier_4way = || -> Box<[Shooter]> {
        Box::new([
            Shooter {
                dx: Fx::from_int(-12),
                ..BASE_SHOOTER
            },
            BASE_SHOOTER,
            Shooter {
                dx: Fx::from_int(12),
                ..BASE_SHOOTER
            },
            Shooter {
                option: 1,
                ..BASE_SHOOTER
            },
        ])
    };
    let empty_opt = || -> Box<[(Fx, Fx)]> { Box::new([]) };

    let shot = ShotTypeCfg {
        sets: [
            [tier_1way(), tier_1way()],
            [tier_1way(), tier_1way()],
            [tier_2way(), tier_2way()],
            [tier_2way(), tier_2way()],
            [tier_4way(), tier_4way()],
        ],
        option_pos: [
            empty_opt(),
            empty_opt(),
            empty_opt(),
            empty_opt(),
            Box::new([TIER4_OPT]),
        ],
    };

    // ── 内建内容包的弹型数据（**不是引擎结构常量**：mod 表自带自己的一份）──────
    // 12 形 × 16 色的整齐矩形，行序 = 图集行序，与三处同源：
    //   godot/assets/bullets.png（由 tools/slice_bullet_sheet.gd 从弹片切出）
    //   godot/ecl/demo/bullets.ecl（内容包词表：LASER/ARROWHEAD/…/LASERHEAD）
    //   docs/render-contract.md §3
    //
    // 半径是**世界判定半径**，与贴图占多少像素是两个独立的量：下表按切图工具实测的
    // 逐行不透明包围盒定档（窄边 9px 的细弹给 2-3，占满 16px 的圆/星给 4-5），
    // 整体沿用东方"判定明显小于观感"的口径。这是玩法旋钮，随手感调，改它不影响图集。
    const BUILTIN_COLOR_STRIDE: u16 = 16;
    //                                   laser arrow outln ball rice kuna shrd amlt bllt bact star lhead
    const SHAPE_RADIUS: [i32; 12] = [3, 4, 4, 5, 2, 3, 2, 5, 3, 3, 4, 5];
    //  12 行全满 16 色（切图工具逐格实测，掩码恒 0xFFFF）——本图集没有空格。
    //  空格机制本身仍在（`valid` 字段 + 编译期/运行期两道闸），只是内建表用不上它；
    //  它的判别式测试改挂在合成表上，见 tables.rs / syscall.rs / lang 各处的空格测试。
    const SHAPE_COLOR_MASK: [u16; 12] = [0xFFFF; 12];

    let stride = BUILTIN_COLOR_STRIDE as usize;
    let mut appearances = Vec::with_capacity(SHAPE_RADIUS.len() * stride);
    for (shape, &r) in SHAPE_RADIUS.iter().enumerate() {
        for color in 0..stride {
            appearances.push(AppearanceCfg {
                radius: Fx::from_int(r),
                sprite: (shape * stride + color) as u16, // identity
                valid: SHAPE_COLOR_MASK[shape] >> color & 1 == 1,
            });
        }
    }

    let drop_tables: Box<[DropTable]> = Box::new([
        Box::new([]) as DropTable,
        Box::new([(ITEM_POWER, 2u8), (ITEM_POINT, 1u8)]) as DropTable,
    ]);

    WorldTables {
        content_hash: 0,
        characters: [CharacterCfg {
            high_speed: CHAR0_HIGH_SPEED,
            low_speed: CHAR0_LOW_SPEED,
            inv_sqrt2: CHAR0_INV_SQRT2,
            hit_radius: CHAR0_HIT_RADIUS,
            graze_radius: CHAR0_GRAZE_RADIUS,
            shot,
            bomb: BombCfg {
                frames: 120,
                invuln: 120,
                attract_items: true,
                fields: Box::new([
                    // ① 全屏消弹：life = frames ⇒ 整段期间**逐帧消掉新飞进来的弹**，
                    //    bomb 的"保护时长"因此天然成立（spec §10.4）。
                    BombField {
                        origin: BombOrigin::FieldCenter,
                        radius: crate::field::FIELD_RADIUS_FULLSCREEN,
                        flags: crate::field::FIELD_CLEAR_BULLETS,
                        dmg_per_frame: 0,
                        life: 120,
                    },
                    // ② 起爆点伤害圆（不跟随）：120 帧 × 4 ≈ 480 伤害，约半管风铃卡血。
                    BombField {
                        origin: BombOrigin::PlayerAtCast,
                        radius: Fx::from_int(120),
                        flags: crate::field::FIELD_DAMAGE,
                        dmg_per_frame: 4,
                        life: 120,
                    },
                ]),
            },
        }],
        item_cfg: ITEM_CFG_V0,
        drop_tables,
        item_gravity: ITEM_GRAVITY_V0,
        color_stride: BUILTIN_COLOR_STRIDE,
        appearances: appearances.into_boxed_slice(),
    }
}

/// 掉落表号 → 逐类型计数（`drop_table` 从"存储状态"退化成"生成参数"的展开口，
/// 敌人死亡效果刀 2026-07-30）。
///
/// 越界表号 → 全零 + `false`；调用方据此计 `contract_viol`（P4-b，原先这条检查在
/// `settle::damage_enemy` 里，随掉落状态一起前移到生成时）。
/// 同一张表里同类型多条目**累加**（`saturating_add`：表已过 `validate`，理论上不该溢出，
/// 但不许在 debug 下 panic）。
pub fn drop_counts(tables: &WorldTables, table: u16) -> ([u8; ITEM_TYPE_COUNT], bool) {
    let mut out = [0u8; ITEM_TYPE_COUNT];
    let Some(rows) = tables.drop_tables.get(table as usize) else {
        return (out, false);
    };
    for &(ty, n) in rows.iter() {
        // `validate` 已保证 `ty < ITEM_TYPE_COUNT`；`get_mut` 是防御性的，不 panic。
        if let Some(slot) = out.get_mut(ty as usize) {
            *slot = slot.saturating_add(n);
        }
    }
    (out, true)
}

/// 内建默认表：从提交的规范字节反序列化（**证明 core 跑在加载的字节上**；金向量走此路径）。
pub static TABLES_V0: LazyLock<WorldTables> = LazyLock::new(|| {
    WorldTables::from_bytes(include_bytes!("tables/tables_v0.bin"))
        .expect("baked v0 table must satisfy the runtime contract")
});

impl WorldTables {
    /// 表校验（debug/测试用）：`interval > 0`、`radius` 双边入 `[0, MAX_ENTITY_RADIUS]`、
    /// `option` 号 `<=` 该档子机数、`drop_tables` 条目类型合法、角色判定/擦弹半径同域、
    /// appearance 表逐行半径同域（M1 T3）。
    pub fn validate(&self) -> bool {
        // 颜色轴：表必须是 `形数 × color_stride` 的整齐矩形
        let stride = self.color_stride as usize;
        if stride == 0
            || self.appearances.is_empty()
            || !self.appearances.len().is_multiple_of(stride)
        {
            return false;
        }
        // 每形第 0 色必须有图——内容包词表里的弹型名恒指向可用格（spec §5）
        if self
            .appearances
            .chunks_exact(stride)
            .any(|shape_row| !shape_row[0].valid)
        {
            return false;
        }
        if !self.appearances.iter().all(|a| radius_in_range(a.radius)) {
            return false;
        }
        for c in &self.characters {
            if !radius_in_range(c.hit_radius) || !radius_in_range(c.graze_radius) {
                return false;
            }
            for tier in 0..5 {
                let opt_len = c.shot.option_pos[tier].len();
                for focus in 0..2 {
                    for shooter in c.shot.sets[tier][focus].iter() {
                        if shooter.interval == 0 {
                            return false;
                        }
                        if !radius_in_range(shooter.radius) {
                            return false;
                        }
                        if shooter.option != 0 && shooter.option as usize > opt_len {
                            return false;
                        }
                    }
                }
            }
            // bomb 描述层校验（自机能力刀）。半径用与其余池同一把尺 `radius_in_range`
            // （`create_field` 也会双边钳，但表校验先拦更响亮）。
            if c.bomb.frames == 0 {
                return false;
            }
            for f in c.bomb.fields.iter() {
                if f.life == 0 {
                    return false;
                }
                if f.flags & !(crate::field::FIELD_CLEAR_BULLETS | crate::field::FIELD_DAMAGE) != 0
                {
                    return false;
                }
                if !radius_in_range(f.radius) {
                    return false;
                }
            }
        }
        if !self
            .drop_tables
            .iter()
            .flat_map(|t| t.iter())
            .all(|&(ty, _)| (ty as usize) < ITEM_TYPE_COUNT)
        {
            return false;
        }
        // **每张掉落表的条目须按 `ty` 严格升序**（D11，2026-09-03 落地）。
        //
        // 为什么这是格式硬契约而不只是洁癖：掉落早已从"死时查表逐条撒"迁成"敌身上按类型
        // 计数、死时按**类型升序**撒"（`world::settle::spill_drops`），条目书写序在运行期
        // 被 `drop_counts` 的展开彻底抹掉——集合相同，但**世界 RNG 的抽取顺序**会随书写序
        // 改变（每颗掉落的 `(vx, vy)` 都抽 RNG）。于是一张非升序的内容包表会**静默**产出
        // 另一条世界线：三平台仍然一致，所以金向量闸门照绿（见 CLAUDE.md「金向量闸门的
        // 能力边界」）。升序即"书写序 == 撒出序"，把这条巧合升级成可校验的契约。
        //
        // 严格升序顺带禁掉同类型重复条目（`[(POWER,1),(POWER,2)]`）——它们本来就会被
        // `drop_counts` 累加成一条，写两行只会让作者以为能控制顺序。
        if !self
            .drop_tables
            .iter()
            .all(|t| t.windows(2).all(|w| w[0].0 < w[1].0))
        {
            return false;
        }
        // join 校验（防 FM1）：每个 ② 表符号 id 必须是 appearances 的合法行。**② 段自
        // 颜色轴刀（2026-07-26）起为空**（弹型名归内容包），故本循环当前不执行；机制保留
        // ——② 段将来重新长出**可加载表行**的符号时自动生效，届时按 tag 分流。
        // （道具类型符号不是那种行，它们在 ① 段——理由见 `consts.rs` ② 段注释。）
        for c in crate::consts::TABLE_SYMBOLS {
            if (c.value as usize) >= self.appearances.len() {
                return false;
            }
        }
        true
    }
}

fn radius_in_range(r: Fx) -> bool {
    r.raw() >= 0 && r.raw() <= MAX_ENTITY_RADIUS.raw()
}

/// 表加载错误（构造前资产环节；返 Result 不 panic，不触模拟确定性）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TableLoadError {
    Truncated,
    BadMagic,
    UnsupportedVersion(u16),
    HashMismatch,
    ArityMismatch {
        field: &'static str,
        expected: usize,
        actual: usize,
    },
    /// 枚举判别值超出已定义范围（如 `bomb_origin` 读到 2）。坏字节不得静默变默认值。
    BadDiscriminant {
        field: &'static str,
        value: u8,
    },
    ValidateFailed,
}

/// 规范字节读取游标（小端；越界→Truncated）。
struct Reader<'a> {
    b: &'a [u8],
    p: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], TableLoadError> {
        let end = self.p.checked_add(n).ok_or(TableLoadError::Truncated)?;
        let s = self.b.get(self.p..end).ok_or(TableLoadError::Truncated)?;
        self.p = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, TableLoadError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, TableLoadError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, TableLoadError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> Result<i32, TableLoadError> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn fx(&mut self) -> Result<Fx, TableLoadError> {
        Ok(Fx::from_raw(self.i32()?))
    }
    fn angle(&mut self) -> Result<Angle, TableLoadError> {
        Ok(Angle(self.u16()?))
    }
}

fn write_shooter(out: &mut Vec<u8>, s: &Shooter) {
    out.extend_from_slice(&s.interval.to_le_bytes());
    out.extend_from_slice(&s.delay.to_le_bytes());
    out.extend_from_slice(&s.dx.raw().to_le_bytes());
    out.extend_from_slice(&s.dy.raw().to_le_bytes());
    out.extend_from_slice(&s.angle.raw().to_le_bytes());
    out.extend_from_slice(&s.speed.raw().to_le_bytes());
    out.extend_from_slice(&s.damage.to_le_bytes());
    out.extend_from_slice(&s.radius.raw().to_le_bytes());
    out.extend_from_slice(&s.sprite.to_le_bytes());
    out.push(s.option);
    out.push(s.flags);
}

fn read_shooter(r: &mut Reader) -> Result<Shooter, TableLoadError> {
    Ok(Shooter {
        interval: r.u16()?,
        delay: r.u16()?,
        dx: r.fx()?,
        dy: r.fx()?,
        angle: r.angle()?,
        speed: r.fx()?,
        damage: r.u16()?,
        radius: r.fx()?,
        sprite: r.u16()?,
        option: r.u8()?,
        flags: r.u8()?,
    })
}

/// 头 16B：magic(4) + version(2) + reserved(2) + content_hash(8)。body = 其后全部字节。
const TABLE_MAGIC: &[u8; 4] = b"STGT";
const TABLE_VERSION: u16 = 3;
const TABLE_HEADER: usize = 16;

impl WorldTables {
    /// 序列化为规范字节（i32/u16 小端，无 float）。`content_hash` = FNV-1a64(body) 回填。
    pub fn to_bytes(&self) -> Vec<u8> {
        use crate::checksum::Fnv1a64;
        let mut out = Vec::new();
        out.extend_from_slice(TABLE_MAGIC);
        out.extend_from_slice(&TABLE_VERSION.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // reserved
        out.extend_from_slice(&0u64.to_le_bytes()); // content_hash 占位（偏移 8..16）

        out.extend_from_slice(&self.item_gravity.raw().to_le_bytes());
        out.extend_from_slice(&self.color_stride.to_le_bytes());
        out.extend_from_slice(&(self.appearances.len() as u32).to_le_bytes());
        for a in self.appearances.iter() {
            out.extend_from_slice(&a.radius.raw().to_le_bytes());
            out.extend_from_slice(&a.sprite.to_le_bytes());
            out.push(u8::from(a.valid));
        }
        out.extend_from_slice(&(self.item_cfg.len() as u32).to_le_bytes());
        for it in self.item_cfg.iter() {
            out.extend_from_slice(&it.score.to_le_bytes());
            out.extend_from_slice(&it.eject_speed.raw().to_le_bytes());
            out.extend_from_slice(&it.terminal_vy.raw().to_le_bytes());
            out.extend_from_slice(&it.magnet_speed.raw().to_le_bytes());
            out.extend_from_slice(&it.pickup_radius.raw().to_le_bytes());
            out.extend_from_slice(&it.attract_radius.raw().to_le_bytes());
            out.extend_from_slice(&it.sprite.to_le_bytes());
        }
        out.extend_from_slice(&(self.drop_tables.len() as u32).to_le_bytes());
        for tbl in self.drop_tables.iter() {
            out.extend_from_slice(&(tbl.len() as u32).to_le_bytes());
            for &(ty, qty) in tbl.iter() {
                out.push(ty);
                out.push(qty);
            }
        }
        out.extend_from_slice(&(self.characters.len() as u32).to_le_bytes());
        for c in self.characters.iter() {
            out.extend_from_slice(&c.high_speed.raw().to_le_bytes());
            out.extend_from_slice(&c.low_speed.raw().to_le_bytes());
            out.extend_from_slice(&c.inv_sqrt2.raw().to_le_bytes());
            out.extend_from_slice(&c.hit_radius.raw().to_le_bytes());
            out.extend_from_slice(&c.graze_radius.raw().to_le_bytes());
            for tier in 0..5 {
                for focus in 0..2 {
                    let list = &c.shot.sets[tier][focus];
                    out.extend_from_slice(&(list.len() as u32).to_le_bytes());
                    for s in list.iter() {
                        write_shooter(&mut out, s);
                    }
                }
            }
            for tier in 0..5 {
                let op = &c.shot.option_pos[tier];
                out.extend_from_slice(&(op.len() as u32).to_le_bytes());
                for &(x, y) in op.iter() {
                    out.extend_from_slice(&x.raw().to_le_bytes());
                    out.extend_from_slice(&y.raw().to_le_bytes());
                }
            }
            // bomb 段（自机能力刀）：frames/invuln/attract_items + 变长 fields。
            // **写入顺序 = 读出顺序**，与 `from_bytes` 的对应块逐字对齐。
            out.extend_from_slice(&c.bomb.frames.to_le_bytes());
            out.extend_from_slice(&c.bomb.invuln.to_le_bytes());
            out.push(u8::from(c.bomb.attract_items));
            out.extend_from_slice(&(c.bomb.fields.len() as u32).to_le_bytes());
            for f in c.bomb.fields.iter() {
                out.push(match f.origin {
                    BombOrigin::FieldCenter => 0u8,
                    BombOrigin::PlayerAtCast => 1u8,
                });
                out.extend_from_slice(&f.radius.raw().to_le_bytes());
                out.push(f.flags);
                out.extend_from_slice(&f.dmg_per_frame.to_le_bytes());
                out.extend_from_slice(&f.life.to_le_bytes());
            }
        }

        let mut h = Fnv1a64::new();
        h.write_bytes(&out[TABLE_HEADER..]);
        out[8..TABLE_HEADER].copy_from_slice(&h.finish().to_le_bytes());
        out
    }

    /// 从规范字节反序列化（只读整数，守 I1）。校验 magic/version、自校 body FNV、arity、`validate`。
    pub fn from_bytes(buf: &[u8]) -> Result<WorldTables, TableLoadError> {
        use crate::checksum::Fnv1a64;
        if buf.len() < TABLE_HEADER {
            return Err(TableLoadError::Truncated);
        }
        if &buf[0..4] != TABLE_MAGIC {
            return Err(TableLoadError::BadMagic);
        }
        let version = u16::from_le_bytes(buf[4..6].try_into().unwrap());
        if version != TABLE_VERSION {
            return Err(TableLoadError::UnsupportedVersion(version));
        }
        let stored = u64::from_le_bytes(buf[8..TABLE_HEADER].try_into().unwrap());
        let mut h = Fnv1a64::new();
        h.write_bytes(&buf[TABLE_HEADER..]);
        if h.finish() != stored {
            return Err(TableLoadError::HashMismatch);
        }

        let mut r = Reader {
            b: buf,
            p: TABLE_HEADER,
        };
        let item_gravity = r.fx()?;
        let color_stride = r.u16()?;

        let na = r.u32()? as usize;
        let mut appearances = Vec::with_capacity(na);
        for _ in 0..na {
            appearances.push(AppearanceCfg {
                radius: r.fx()?,
                sprite: r.u16()?,
                valid: r.u8()? != 0,
            });
        }

        let ni = r.u32()? as usize;
        if ni != ITEM_TYPE_COUNT {
            return Err(TableLoadError::ArityMismatch {
                field: "item_cfg",
                expected: ITEM_TYPE_COUNT,
                actual: ni,
            });
        }
        let mut item_vec = Vec::with_capacity(ni);
        for _ in 0..ni {
            item_vec.push(ItemTypeCfg {
                score: r.u32()?,
                eject_speed: r.fx()?,
                terminal_vy: r.fx()?,
                magnet_speed: r.fx()?,
                pickup_radius: r.fx()?,
                attract_radius: r.fx()?,
                sprite: r.u16()?,
            });
        }
        let item_cfg: [ItemTypeCfg; ITEM_TYPE_COUNT] = item_vec
            .try_into()
            .expect("count checked == ITEM_TYPE_COUNT");

        let nd = r.u32()? as usize;
        let mut drops: Vec<Box<[(u8, u8)]>> = Vec::with_capacity(nd);
        for _ in 0..nd {
            let inner = r.u32()? as usize;
            let mut row = Vec::with_capacity(inner);
            for _ in 0..inner {
                row.push((r.u8()?, r.u8()?));
            }
            drops.push(row.into_boxed_slice());
        }

        let nc = r.u32()? as usize;
        if nc != 1 {
            return Err(TableLoadError::ArityMismatch {
                field: "characters",
                expected: 1,
                actual: nc,
            });
        }
        let high_speed = r.fx()?;
        let low_speed = r.fx()?;
        let inv_sqrt2 = r.fx()?;
        let hit_radius = r.fx()?;
        let graze_radius = r.fx()?;
        let mut sets: [[Box<[Shooter]>; 2]; 5] =
            std::array::from_fn(|_| std::array::from_fn(|_| Box::default()));
        for tier_sets in sets.iter_mut() {
            for slot in tier_sets.iter_mut() {
                let n = r.u32()? as usize;
                let mut v = Vec::with_capacity(n);
                for _ in 0..n {
                    v.push(read_shooter(&mut r)?);
                }
                *slot = v.into_boxed_slice();
            }
        }
        let mut option_pos: [Box<[(Fx, Fx)]>; 5] = std::array::from_fn(|_| Box::default());
        for slot in option_pos.iter_mut() {
            let m = r.u32()? as usize;
            let mut v = Vec::with_capacity(m);
            for _ in 0..m {
                v.push((r.fx()?, r.fx()?));
            }
            *slot = v.into_boxed_slice();
        }

        // bomb 段：顺序与 `to_bytes` 逐字对应。
        let bomb_frames = r.u16()?;
        let bomb_invuln = r.u16()?;
        let bomb_attract = r.u8()? != 0;
        let nbf = r.u32()? as usize;
        let mut bomb_fields = Vec::with_capacity(nbf);
        for _ in 0..nbf {
            let d = r.u8()?;
            let origin = match d {
                0 => BombOrigin::FieldCenter,
                1 => BombOrigin::PlayerAtCast,
                // 坏字节**必须拒**，不得静默变成默认值——那是静默数据损坏。
                _ => {
                    return Err(TableLoadError::BadDiscriminant {
                        field: "bomb_origin",
                        value: d,
                    });
                }
            };
            bomb_fields.push(BombField {
                origin,
                radius: r.fx()?,
                flags: r.u8()?,
                dmg_per_frame: r.u16()?,
                life: r.u16()?,
            });
        }

        let t = WorldTables {
            content_hash: stored,
            characters: [CharacterCfg {
                high_speed,
                low_speed,
                inv_sqrt2,
                hit_radius,
                graze_radius,
                shot: ShotTypeCfg { sets, option_pos },
                bomb: BombCfg {
                    frames: bomb_frames,
                    invuln: bomb_invuln,
                    attract_items: bomb_attract,
                    fields: bomb_fields.into_boxed_slice(),
                },
            }],
            item_cfg,
            drop_tables: drops.into_boxed_slice(),
            item_gravity,
            color_stride,
            appearances: appearances.into_boxed_slice(),
        };
        if !t.validate() {
            return Err(TableLoadError::ValidateFailed);
        }
        Ok(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A1 判别式:序列化往返保 sprite(互异非零判别值,S1 纪律)。
    #[test]
    fn item_sprite_roundtrips_with_distinct_values() {
        let mut t = build_tables_v0();
        let vals: [u16; ITEM_TYPE_COUNT] = [11, 22, 33, 44, 55];
        for (i, v) in vals.iter().enumerate() {
            t.item_cfg[i].sprite = *v;
        }
        let back = WorldTables::from_bytes(&t.to_bytes()).expect("roundtrip");
        for (i, v) in vals.iter().enumerate() {
            assert_eq!(back.item_cfg[i].sprite, *v, "row {i}");
        }
    }

    /// A1 内容健全:v0 五行 sprite 互异(占位序号 0..=4)。
    #[test]
    fn item_cfg_v0_sprites_distinct() {
        let t = build_tables_v0();
        for a in 0..ITEM_TYPE_COUNT {
            for b in (a + 1)..ITEM_TYPE_COUNT {
                assert_ne!(t.item_cfg[a].sprite, t.item_cfg[b].sprite, "{a} vs {b}");
            }
        }
    }

    /// `TABLES_V0` 必须过表校验——v0 内容自洽的钉死。
    #[test]
    fn tables_v0_validates() {
        assert!(TABLES_V0.validate());
    }

    /// 内建表经 `from_bytes` 载入，`content_hash` 必须 LIVE（非 0），且与直接烘焙同源自洽。
    #[test]
    fn builtin_tables_v0_has_live_content_hash() {
        assert_ne!(
            TABLES_V0.content_hash, 0,
            "内建表经 from_bytes 载入，hash 应非 0"
        );
        // 与直接烘焙同源的自洽：include 的字节 == build_tables_v0().to_bytes()
        let from_builder = WorldTables::from_bytes(&build_tables_v0().to_bytes()).unwrap();
        assert_eq!(TABLES_V0.content_hash, from_builder.content_hash);
    }

    /// 形状 + v0 内容量拍板：5 档×2 态槽全非悬垂、同档两焦点槽内容相等
    /// （owned 后各自独立分配，非 `std::ptr::eq`）、tier0 一路 / tier2 两路 / tier4
    /// 三路+恰一个子机 shooter。
    #[test]
    fn tables_v0_shape() {
        let shot = &TABLES_V0.characters[0].shot;

        for tier in 0..5 {
            let [a, b] = &shot.sets[tier];
            assert!(!a.is_empty(), "tier {tier} focus0 非悬垂空表");
            assert_eq!(a, b, "tier {tier} 两焦点槽内容相等（owned 后各自独立分配）");
        }

        assert_eq!(shot.sets[0][0].len(), 1, "tier0 一路直射");
        assert_eq!(shot.sets[1][0].len(), 1, "tier1 同 tier0（1 路）");
        assert_eq!(shot.sets[2][0].len(), 2, "tier2 两路");
        assert_eq!(shot.sets[3][0].len(), 2, "tier3 同 tier2（两路）");
        assert_eq!(shot.sets[4][0].len(), 4, "tier4 三路本体 + 1 路子机");

        // tier2 两路对称偏移 ∓8px。
        let t2dx: Vec<i32> = shot.sets[2][0].iter().map(|s| s.dx.raw()).collect();
        assert_eq!(t2dx, vec![Fx::from_int(-8).raw(), Fx::from_int(8).raw()]);

        // tier4：恰一个子机 shooter（option != 0），其余三路 option == 0。
        let option_shooters: Vec<&Shooter> =
            shot.sets[4][0].iter().filter(|s| s.option != 0).collect();
        assert_eq!(option_shooters.len(), 1, "tier4 恰一个子机 shooter");
        assert_eq!(option_shooters[0].option, 1);
        assert_eq!(option_shooters[0].dx, Fx::ZERO);
        assert_eq!(option_shooters[0].dy, Fx::ZERO);

        // 子机出生偏移表：tier4 恰一行 (-20px, 8px)，tier0..=3 空。
        assert_eq!(shot.option_pos[4].len(), 1, "tier4 子机出生点表恰一行");
        assert_eq!(shot.option_pos[4][0], (Fx::from_int(-20), Fx::from_int(8)));
        for tier in 0..4 {
            assert!(shot.option_pos[tier].is_empty(), "tier{tier} 无子机");
        }
    }

    /// 配置表逐行健全（迁自 `items.rs`：M0-17 T2 道具表搬家）：分值/物理参数为正、
    /// 拾取半径 ≤ MAX_ENTITY_RADIUS（行 5 加法证明前提）。
    #[test]
    fn item_cfg_rows_sane() {
        use crate::items::ITEM_POINT;
        for (t, cfg) in TABLES_V0.item_cfg.iter().enumerate() {
            assert!(cfg.score > 0, "type {t}");
            assert!(cfg.eject_speed.raw() > 0 && cfg.terminal_vy.raw() > 0);
            assert!(cfg.magnet_speed.raw() > 0 && cfg.attract_radius.raw() > 0);
            assert!(
                cfg.pickup_radius.raw() > 0
                    && cfg.pickup_radius.raw() <= crate::world::MAX_ENTITY_RADIUS.raw(),
                "type {t} 拾取半径越出行 5 加法安全域"
            );
        }
        assert_eq!(TABLES_V0.item_cfg[ITEM_POINT as usize].score, 100);
    }

    /// 掉落表（迁自 `items.rs`）：表 0 恒空（enemy.drop_table 零初始化默认 = 不掉）；
    /// 表 1 = 标准杂鱼。
    #[test]
    fn drop_tables_shape() {
        use crate::items::{ITEM_POINT, ITEM_POWER, ITEM_TYPE_COUNT};
        assert!(TABLES_V0.drop_tables[0].is_empty());
        assert_eq!(
            &*TABLES_V0.drop_tables[1],
            &[(ITEM_POWER, 2), (ITEM_POINT, 1)]
        );
        assert!(
            TABLES_V0
                .drop_tables
                .iter()
                .flat_map(|t| t.iter())
                .all(|&(ty, _)| (ty as usize) < ITEM_TYPE_COUNT),
            "掉落表条目类型必须合法——扩展四步第④步的脚下网"
        );
    }

    /// `drop_counts`：内建表 1 展开成逐类型计数；越界表号降级成全零 + false（P4-b）。
    #[test]
    fn drop_counts_expands_table_and_degrades_out_of_range() {
        let (c1, ok1) = drop_counts(&TABLES_V0, 1);
        assert!(ok1);
        assert_eq!(c1[crate::items::ITEM_POWER as usize], 2);
        assert_eq!(c1[crate::items::ITEM_POINT as usize], 1);
        assert_eq!(
            c1.iter().map(|&n| n as u32).sum::<u32>(),
            3,
            "别的类型必须是 0"
        );

        let (c0, ok0) = drop_counts(&TABLES_V0, 0);
        assert!(ok0, "表 0 是合法的空表，不是越界");
        assert_eq!(c0, [0u8; crate::items::ITEM_TYPE_COUNT]);

        let bad = TABLES_V0.drop_tables.len() as u16 + 9;
        let (cb, okb) = drop_counts(&TABLES_V0, bad);
        assert!(!okb, "越界表号必须报 false");
        assert_eq!(cb, [0u8; crate::items::ITEM_TYPE_COUNT]);
    }

    /// D11：掉落表条目须按 `ty` **严格升序**——非升序 / 重复类型各一条判别腿。
    ///
    /// **为什么要判别式而不是"能过就行"**：条目序在运行期被 `drop_counts` 抹掉，集合相同、
    /// 只有世界 RNG 的抽取顺序变——这是三平台**一致地错**的形态，金向量闸门抓不到。
    ///
    /// **判别力**：把 `validate` 的判据从 `<`（严格升序）放松成 `<=` ⇒ 第二条（重复类型）
    /// 转绿即红；把整条校验删掉 ⇒ 两条都红；只测降序而不测重复 ⇒ `<=` 那种写法逃掉。
    /// 第三条正例押住"内建表本来就满足"这个前提（它是本条不需要重烘焙 `tables_v0.bin`
    /// 的全部理由）。
    #[test]
    fn validate_rejects_drop_table_not_strictly_ascending() {
        use crate::items::{ITEM_POINT, ITEM_POWER};

        let with_drops = |rows: Vec<(u8, u8)>| {
            let mut t = build_tables_v0();
            t.drop_tables = Box::new([Box::new([]) as DropTable, rows.into_boxed_slice()]);
            t
        };

        // ① 降序：撒出去仍是 POWER 在前，但作者以为是 POINT 在前 ⇒ RNG 消耗序与预期不符
        assert!(
            !with_drops(vec![(ITEM_POINT, 1), (ITEM_POWER, 2)]).validate(),
            "掉落表条目降序必须被拒"
        );
        // ② 同类型重复：会被 drop_counts 累加成一条，写两行纯属误解
        assert!(
            !with_drops(vec![(ITEM_POWER, 1), (ITEM_POWER, 2)]).validate(),
            "同类型重复条目必须被拒（严格升序，不是非降序）"
        );
        // ③ 正例 + 内建表：升序照过，且 v0 内建表**本来就满足**（故不需要重烘焙）
        assert!(with_drops(vec![(ITEM_POWER, 2), (ITEM_POINT, 1)]).validate());
        assert!(build_tables_v0().validate(), "内建 v0 表须满足新契约");
    }

    /// 判别腿：interval=0 / radius 超上限 / option 号越界的坏表各自 `validate() == false`。
    #[test]
    fn validate_rejects_bad() {
        fn bad_with_shot(shot: ShotTypeCfg) -> WorldTables {
            let mut t = build_tables_v0();
            t.characters[0].shot = shot;
            t
        }

        // interval=0：tier0 换成一个 interval 为 0 的坏 shooter。
        {
            let bad: Box<[Shooter]> = Box::new([Shooter {
                interval: 0,
                ..BASE_SHOOTER
            }]);
            let mut shot = build_tables_v0().characters[0].shot.clone();
            shot.sets[0] = [bad.clone(), bad];
            assert!(
                !bad_with_shot(shot).validate(),
                "interval=0 必须被 validate 拒绝"
            );
        }

        // radius 超上限：MAX_ENTITY_RADIUS = 1024px，给 2000px。
        {
            let bad: Box<[Shooter]> = Box::new([Shooter {
                radius: Fx::from_int(2000),
                ..BASE_SHOOTER
            }]);
            let mut shot = build_tables_v0().characters[0].shot.clone();
            shot.sets[0] = [bad.clone(), bad];
            assert!(
                !bad_with_shot(shot).validate(),
                "radius 超 MAX_ENTITY_RADIUS 必须被 validate 拒绝"
            );
        }

        // option 号越界：tier0 的 option_pos[0] 是空表，option=1 越界（1 > 0）。
        {
            let bad: Box<[Shooter]> = Box::new([Shooter {
                option: 1,
                ..BASE_SHOOTER
            }]);
            let mut shot = build_tables_v0().characters[0].shot.clone();
            shot.sets[0] = [bad.clone(), bad];
            assert!(
                !bad_with_shot(shot).validate(),
                "option 号超出该档子机数必须被 validate 拒绝"
            );
        }
    }

    /// 内建 appearance 表 = 12 形 × 16 色的整齐矩形（identity + 同形同半径 + 空格掩码）。
    /// 判别力：对调 `SHAPE_RADIUS` 中两个**不同**的值必须让本测试变红。
    #[test]
    fn appearances_v0_is_12x16_grid() {
        let t = &*TABLES_V0;
        assert_eq!(t.color_stride, 16, "内建内容包图集 16 列");
        assert_eq!(t.appearances.len(), 12 * 16);

        // identity：表索引 ≡ 图集格号 ≡ 池 sprite 值
        for (i, a) in t.appearances.iter().enumerate() {
            assert_eq!(
                a.sprite as usize, i,
                "第 {i} 行 sprite 必须等于行号（identity）"
            );
        }

        // 逐形半径钉死（判别腿：SHAPE_RADIUS 错序即红）。
        // 行序 = 图集行序：laser/arrowhead/outline/ball/rice/kunai/shard/amulet/
        //                  bullet/bacteria/star/laserhead
        let expect = [3, 4, 4, 5, 2, 3, 2, 5, 3, 3, 4, 5];
        for (shape, &r) in expect.iter().enumerate() {
            for color in 0..16usize {
                assert_eq!(
                    t.appearances[shape * 16 + color].radius,
                    Fx::from_int(r),
                    "形 {shape} 色 {color} 半径应为 {r}px（同形 16 行必然同半径）"
                );
            }
        }

        // 本图集 12 行全满 16 色（切图工具逐格实测），无空格。
        // 空格机制本身的判别式测试挂在**合成表**上（见本模块
        // `blank_cell_survives_bytes_roundtrip` 与 `ecl::syscall` / `lang` 各处），
        // 不靠内建表提供靶子——数据要如实反映美术。
        assert!(
            t.appearances.iter().all(|a| a.valid),
            "当前图集没有空格格；若换了有缺色的美术，请连同 SHAPE_COLOR_MASK 一起更新本断言"
        );
    }

    /// validate 新三条：stride 非零 / 行数是 stride 整数倍 / 每形第 0 色必须有图。
    #[test]
    fn validate_rejects_bad_color_grid() {
        // ① stride 为 0
        let mut bad = build_tables_v0();
        bad.color_stride = 0;
        assert!(!bad.validate(), "color_stride = 0 必须被拒");

        // ② 行数不是 stride 的整数倍（矩形被破坏）
        let mut ragged = build_tables_v0();
        let mut rows = ragged.appearances.to_vec();
        rows.pop();
        ragged.appearances = rows.into_boxed_slice();
        assert!(!ragged.validate(), "行数非 stride 整数倍必须被拒");

        // ③ 某形第 0 色是空格（形状名会指向不可用格）
        let mut hole = build_tables_v0();
        let mut rows = hole.appearances.to_vec();
        rows[2 * 16].valid = false; // 第 2 形第 0 色
        hole.appearances = rows.into_boxed_slice();
        assert!(!hole.validate(), "某形第 0 色为空格必须被拒");

        // 判别力反证：原表必须通过
        assert!(build_tables_v0().validate(), "内建表本身必须过 validate");
    }

    /// 规范字节往返必须带上 `color_stride` 与逐行 `valid`（防"新字段没进格式"）。
    #[test]
    fn bytes_roundtrip_carries_stride_and_valid() {
        let t = build_tables_v0();
        let back = WorldTables::from_bytes(&t.to_bytes()).expect("往返必须成功");
        assert_eq!(back.color_stride, t.color_stride);
        assert_eq!(back.appearances.len(), t.appearances.len());
        for (i, (a, b)) in t
            .appearances
            .iter()
            .zip(back.appearances.iter())
            .enumerate()
        {
            assert_eq!(a.valid, b.valid, "第 {i} 行 valid 未往返");
            assert_eq!(a.sprite, b.sprite, "第 {i} 行 sprite 未往返");
            assert_eq!(a.radius, b.radius, "第 {i} 行 radius 未往返");
        }
        // 上面那趟只证明"全 true 原样往返"——对 `valid` 几乎没有判别力（当前图集
        // 12 行全满，无空格）。补一趟**合成空格表**：把某一格标成空格再往返，
        // 断言它没有在序列化里被悄悄抹平成 true。判别力所在：若 `to_bytes`/
        // `from_bytes` 漏掉 valid 字段，这一条立刻红，而上面那趟仍会绿。
        let mut holed = build_tables_v0();
        let mut rows = holed.appearances.to_vec();
        let hole = 3 * 16 + 7; // 任取一格（第 3 形第 7 色）
        rows[hole].valid = false;
        holed.appearances = rows.into_boxed_slice();

        let back2 = WorldTables::from_bytes(&holed.to_bytes()).expect("合成空格表往返必须成功");
        assert!(
            !back2.appearances[hole].valid,
            "空格标记未往返（被抹成 true）"
        );
        assert_eq!(
            back2.appearances.iter().filter(|a| !a.valid).count(),
            1,
            "只应有这一格是空格——多出来说明往返把别的格也弄坏了"
        );
    }

    /// appearance 表 validate 判别腿：半径超上限的坏行必须被拒绝。
    #[test]
    fn validate_rejects_bad_appearance_radius() {
        let mut bad = build_tables_v0();
        bad.color_stride = 1; // 单行矩形，不让颜色轴矩形检查抢先拦截
        bad.appearances = Box::new([AppearanceCfg {
            radius: Fx::from_int(2000), // 超 MAX_ENTITY_RADIUS(1024)
            sprite: 0,
            valid: true,
        }]);
        assert!(!bad.validate(), "appearance 半径超上限必须被 validate 拒绝");
    }

    // ② 表符号相关的两条测试（`validate_rejects_table_symbol_without_appearance_row`
    // 的 FM1 判别腿、`builtin_appearances_exactly_cover_table_symbols` 的 coverage
    // 断言）随颜色轴刀 T4 清空 ② 段一并退场——`TABLE_SYMBOLS` 现在是空表，两者都退化成
    // 空断言（前者甚至会因 join 循环不执行而反转成红）。`validate` 里的 join 校验本身
    // **保留**：机制仍在，② 段将来重新长出行时自动生效。

    /// B14 债：角色 hit/graze 半径越界的负向腿（此前只有正向覆盖）。
    #[test]
    fn validate_rejects_bad_character_radius() {
        let mut t = build_tables_v0();
        t.characters[0].hit_radius = Fx::from_int(2000); // 超 MAX_ENTITY_RADIUS(1024)
        assert!(!t.validate(), "角色 hit_radius 超上限必须被 validate 拒绝");
        let mut t2 = build_tables_v0();
        t2.characters[0].graze_radius = Fx::from_int(2000);
        assert!(
            !t2.validate(),
            "角色 graze_radius 超上限必须被 validate 拒绝"
        );
    }

    #[test]
    fn to_from_bytes_round_trip_preserves_all_fields() {
        let mut t = build_tables_v0();
        let bytes = t.to_bytes();
        let back = WorldTables::from_bytes(&bytes).expect("round-trip must load");
        assert_ne!(back.content_hash, 0, "from_bytes 计算真 content_hash");
        t.content_hash = back.content_hash; // 对齐 from_bytes 填的唯一字段
        assert_eq!(t, back, "round-trip 逐字段一致");
    }

    #[test]
    fn to_bytes_is_deterministic() {
        assert_eq!(build_tables_v0().to_bytes(), build_tables_v0().to_bytes());
    }

    #[test]
    fn content_hash_changes_when_a_value_changes() {
        let h0 = {
            let b = build_tables_v0().to_bytes();
            WorldTables::from_bytes(&b).unwrap().content_hash
        };
        let mut t = build_tables_v0();
        t.item_gravity = Fx::from_raw(9_831); // 改一个 body 值
        let h1 = {
            let b = t.to_bytes();
            WorldTables::from_bytes(&b).unwrap().content_hash
        };
        assert_ne!(h0, h1, "改 body 任一值 → content_hash 变");
    }

    #[test]
    fn from_bytes_rejects_bad_magic_version_truncation_and_tamper() {
        let good = build_tables_v0().to_bytes();

        let mut bad_magic = good.clone();
        bad_magic[0] = b'X';
        assert_eq!(
            WorldTables::from_bytes(&bad_magic),
            Err(TableLoadError::BadMagic)
        );

        // 坏版本样本必须相对 `TABLE_VERSION` 取值（`TABLE_VERSION + 1`），不可写死字面量——
        // 字面量会在下次 `TABLE_VERSION` bump 时与新的"当前版本"撞车，此测试曾因此误报过。
        let mut bad_ver = good.clone();
        let bad_version_value = TABLE_VERSION + 1;
        bad_ver[4..6].copy_from_slice(&bad_version_value.to_le_bytes());
        assert_eq!(
            WorldTables::from_bytes(&bad_ver),
            Err(TableLoadError::UnsupportedVersion(bad_version_value))
        );

        assert_eq!(
            WorldTables::from_bytes(&good[..8]),
            Err(TableLoadError::Truncated)
        );

        let mut tampered = good.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 0xFF; // 改 body 尾字节但不重算 hash
        assert_eq!(
            WorldTables::from_bytes(&tampered),
            Err(TableLoadError::HashMismatch)
        );
    }

    #[test]
    fn from_bytes_rejects_arity_mismatch() {
        // 手工造一份 hash 自洽、但 item_cfg 计数 != ITEM_TYPE_COUNT 的 buffer。
        use crate::checksum::Fnv1a64;
        let mut body = Vec::new();
        body.extend_from_slice(&Fx::ZERO.raw().to_le_bytes()); // item_gravity
        body.extend_from_slice(&0u16.to_le_bytes()); // color_stride 0
        body.extend_from_slice(&0u32.to_le_bytes()); // appearances count 0
        body.extend_from_slice(&3u32.to_le_bytes()); // item_cfg count 3 (!= 5)
        let mut buf = Vec::new();
        buf.extend_from_slice(TABLE_MAGIC);
        buf.extend_from_slice(&TABLE_VERSION.to_le_bytes());
        buf.extend_from_slice(&0u16.to_le_bytes());
        let mut h = Fnv1a64::new();
        h.write_bytes(&body);
        buf.extend_from_slice(&h.finish().to_le_bytes());
        buf.extend_from_slice(&body);
        assert_eq!(
            WorldTables::from_bytes(&buf),
            Err(TableLoadError::ArityMismatch {
                field: "item_cfg",
                expected: 5,
                actual: 3
            })
        );
    }
    /// v0 内容锚点：两条 field（全屏消弹 + 起爆点伤害圆），120 帧，吸道具。
    /// 改内容要有意识地改本测试。
    #[test]
    fn bomb_cfg_v0_is_two_fields() {
        let b = &TABLES_V0.characters[0].bomb;
        assert_eq!((b.frames, b.invuln, b.attract_items), (120, 120, true));
        assert_eq!(b.fields.len(), 2);
        assert_eq!(b.fields[0].origin, BombOrigin::FieldCenter);
        assert_eq!(b.fields[0].flags, crate::field::FIELD_CLEAR_BULLETS);
        assert_eq!(b.fields[1].origin, BombOrigin::PlayerAtCast);
        assert_eq!(b.fields[1].flags, crate::field::FIELD_DAMAGE);
        assert_eq!(b.fields[1].dmg_per_frame, 4);
        // 两条都活满整段 —— 消弹 field 的 life = frames 是"持续保护"的来源（spec §10.4）
        assert!(b.fields.iter().all(|f| f.life == b.frames));
    }

    /// validate 的四条：radius 越界 / flags 含未定义位 / frames==0 / life==0 都要被拒。
    /// 判别力=四条各造一个坏表，只测一条的话另外三条的校验漏写也绿。
    #[test]
    fn bomb_cfg_validate_rejects_bad_rows() {
        let mk = |mutate: &dyn Fn(&mut BombCfg)| {
            let mut t = build_tables_v0();
            let mut b = t.characters[0].bomb.clone();
            mutate(&mut b);
            t.characters[0].bomb = b;
            t
        };
        assert!(
            !mk(&|b| b.fields[0].radius = Fx::from_int(-1)).validate(),
            "radius 越界须拒"
        );
        assert!(
            !mk(&|b| b.fields[0].flags = 0x80).validate(),
            "未定义 flags 位须拒"
        );
        assert!(!mk(&|b| b.frames = 0).validate(), "frames==0 须拒");
        assert!(!mk(&|b| b.fields[0].life = 0).validate(), "life==0 须拒");
        assert!(build_tables_v0().validate(), "内建表本身必须合法");
    }

    /// bomb 段的 to_bytes/from_bytes 往返：写出去再读回来必须逐字段相等。
    /// 判别力：漏写任何一个字段、或读写顺序错位，这条都会红（而只测"能解析"的写法不会）。
    #[test]
    fn bomb_cfg_survives_a_bytes_roundtrip() {
        let t0 = build_tables_v0();
        let t1 = WorldTables::from_bytes(&t0.to_bytes()).expect("往返应成功");
        assert_eq!(t1.characters[0].bomb, t0.characters[0].bomb);
    }

    /// 坏的 origin 判别值必须被**拒绝**，不得静默变成默认值（静默 = 数据损坏）。
    ///
    /// ⚠️ `from_bytes` 的 hash 自校**先于**字段解析（`tables.rs` 的 `HashMismatch` 分支在
    /// 建 `Reader` 之前），所以"改一个字节就交给 from_bytes"只会撞 `HashMismatch`、根本走
    /// 不到判别值那行。故本测试**篡改后重算 body 的 FNV 回填头部**——造一份"自洽但内容非法"
    /// 的表，这正是外部内容包能真实递进来的形态（哈希只证完整性，不证合法性）。
    ///
    /// 定位方式：按写入顺序，v0 第一条 bomb field 的字节是
    /// `origin=0x00` ⧺ `radius = Fx::from_int(400).raw() = 26_214_400 = 0x0190_0000`
    /// 的小端 `00 00 90 01` ⧺ `flags = FIELD_CLEAR_BULLETS = 0x01`。这个 6 字节窗口在
    /// 整份表里唯一（断言里押着"唯一"，模式若不再唯一这条会红而不是悄悄改错地方）。
    #[test]
    fn bomb_origin_rejects_unknown_discriminant() {
        use crate::checksum::Fnv1a64;
        let mut bytes = build_tables_v0().to_bytes();
        const PAT: [u8; 6] = [0x00, 0x00, 0x00, 0x90, 0x01, 0x01];
        let hits: Vec<usize> = bytes
            .windows(PAT.len())
            .enumerate()
            .filter(|(_, w)| *w == PAT)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            hits.len(),
            1,
            "定位模式必须唯一（表变了就改这条，别让它悄悄错位）"
        );
        bytes[hits[0]] = 7; // 非法 origin
        // 重算 body 哈希回填，绕开先于解析的完整性闸门。
        let mut h = Fnv1a64::new();
        h.write_bytes(&bytes[TABLE_HEADER..]);
        let fixed = h.finish().to_le_bytes();
        bytes[8..TABLE_HEADER].copy_from_slice(&fixed);
        match WorldTables::from_bytes(&bytes) {
            Err(TableLoadError::BadDiscriminant { field, value }) => {
                assert_eq!((field, value), ("bomb_origin", 7));
            }
            other => panic!("未知 origin 判别值须被拒，实得 {other:?}"),
        }
    }
}
