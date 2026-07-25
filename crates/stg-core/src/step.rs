//! 组装层（stg_core::step）—— §3.5/A4 宪法顺序的唯一持有者（P2）。World 定义 + 构造 + step。

use std::alloc::{Layout, alloc_zeroed, handle_alloc_error};

use crate::ecl::image::{EclImage, SubId, SubKind};
use crate::rng::Pcg32;
use crate::world::{
    PH_DIRECTOR, PH_ECL_HOOK, POOL_TASK, RNG_SEQ, STATUS_BAD_ARGS, STATUS_POOL_FULL, WorldBody,
};

/// 权威可变状态。
///
/// `tasks`（M1 起）：ECL 任务池物理住组装层以满足 memcpy 快照（P1：world 不知道"任务"存在，
/// `WorldBody` 内不 import `ecl::*`）。T2 起相位 2（`PH_DIRECTOR`）导演槽跑
/// `ecl::vm::run_tasks` 驱动它（`step_with_director` 内，注入的导演闭包之前）。
#[repr(C)]
#[derive(crate::checksum::Checksum, crate::save::SaveBytes)]
pub struct World {
    pub body: WorldBody,
    /// 读走 `tasks()`。
    pub(crate) tasks: crate::ecl::task::TaskPool,
    /// One-shot flag: 1 after a successful `start_main`, stays 1 even after
    /// the main task ends or faults.  Prevents re-starting the root script.
    /// Copied in `copy_into` and automatically included in checksum via derive.
    pub(crate) ecl_main_started: u8,
    /// 本 World 绑定的表 `content_hash`（`new_with_tables` 记录）。coherence 守卫读它对
    /// `EclImage.content_hash` 一次比对。常量存活期不变，跨机一致，正常入校验和。
    pub(crate) tables_hash: u64,
    /// 构造用的 RNG 种子（provenance）。种子在构造期被揉进 `body.rng` 的初始 state 后不可反推，
    /// 本字段是其唯一留存形态——供回放头/握手/调试读回（`seed()`）。属"初始状态"、存活期不变、
    /// 跨机一致；同 `tables_hash` 正常入校验和（恒定不分叉，校验无害）。
    pub(crate) seed: u64,
}

/// 手写占位 Debug——只为满足 `Result::unwrap_err`/`expect_err` 的 `T: Debug` 约束（存档
/// 判别测试用它断言 `LoadError` 分支，`Ok` 分支不该真的走到）。不逐字段展开：`World`
/// 挂着 ~1MB 的池 SoA 数组，`#[derive(Debug)]` 会把整条依赖链（各池/`Event`/`RenderReq`
/// 等纯输出类型）拖进 Debug 义务，得不偿失——`checksum()`/`save_bytes()` 才是真正的状态
/// 摘要通道，这里给个占位输出即可。
impl std::fmt::Debug for World {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("World").finish_non_exhaustive()
    }
}

impl World {
    /// 堆零初始化的新世界，再播种 rng。
    ///
    /// **为何堆零构造 + 一处 unsafe**：`World` 是 POD（全整数/数组、无 Drop、无引用、无枚举无效
    /// 判别式），**全零是合法值**（空池 alive 全 0、gen/字段 0、frame 0、diag 0）。按值构造 ~450KB
    /// 的 World 会在栈上放巨型临时量（debug 未优化时栈溢出）；`alloc_zeroed` 直接在堆上零构造，
    /// 避开栈。这是 stg-core 唯一一处 unsafe，也是设计既定的 World 堆分配落点。
    pub fn new(seed: u64) -> Box<World> {
        Self::new_with_tables(seed, &crate::tables::TABLES_V0)
    }

    /// 用指定表构造：自机取 `tables.characters[0]`，并记录 `tables_hash = tables.content_hash`
    /// 供启动期 coherence 守卫。`new(seed)` 委托此函数绑内建 `TABLES_V0`。
    pub fn new_with_tables(seed: u64, tables: &crate::tables::WorldTables) -> Box<World> {
        let layout = Layout::new::<World>();
        // SAFETY: World 全零合法（见上）；layout 由类型给出；分配失败走 handle_alloc_error；
        // Box::from_raw 接管同一全局分配器的这块内存，Drop 时正确释放。
        let mut w: Box<World> = unsafe {
            let ptr = alloc_zeroed(layout) as *mut World;
            if ptr.is_null() {
                handle_alloc_error(layout);
            }
            Box::from_raw(ptr)
        };
        w.body.rng = Pcg32::new(seed, RNG_SEQ);
        // 自机 1 出场（角色 0，取 `tables.characters[0]`）；自机 2 保持全零=不在场。
        w.body.players[0] = crate::player::PlayerState::spawn(0, &tables.characters[0]);
        w.tables_hash = tables.content_hash;
        w.seed = seed;
        w
    }

    /// 正典开机完全体(整局流程刀 spec §2.2/§3):回放/握手身份 = (seed, rank, start, loadout,
    /// image_hash)。start=0 从头;非 0 查标记表把根任务 pc 直接搁到 main 内落点(landing pad,
    /// 编译器保证合法指令边界)。中段启动是"规范态"开局(符卡练习语义):被跳过流程的世界
    /// 效果由脚本 mark 块+自动补偿承担(见 stg-ecl-compiler codegen `scan_mark_compensation`)。
    ///
    /// 两条宿主期响亮错(P4-a)钉在 `World::new` 分配**之前**完成,而非事后回滚:
    /// `loadout.character` 越 `tables.characters.len()` → `InvalidCharacter`;`start != 0`
    /// 且标记表查无该 id(含负值——标记表 id 恒正,天然不命中)→ `UnknownMark`。两项校验
    /// 全通过才分配 `World`,失败路径下从未存在过半初始化的 `Box<World>`——`Err` 分支
    /// 不持有、也无需丢弃任何世界实例。
    pub fn new_game_at(
        seed: u64,
        rank: i32,
        start: i32,
        loadout: crate::player::Loadout,
        image: &crate::ecl::image::EclImage,
    ) -> Result<Box<World>, crate::ecl::binding::TaskStartError> {
        use crate::ecl::binding::TaskStartError;
        let tables = &crate::tables::TABLES_V0;
        if loadout.character as usize >= tables.characters.len() {
            return Err(TaskStartError::InvalidCharacter(loadout.character));
        }
        let landing = if start != 0 {
            Some(
                image
                    .resolve_mark(start)
                    .ok_or(TaskStartError::UnknownMark(start))?,
            )
        } else {
            None
        };
        let mut w = World::new(seed);
        w.body.players[0] = crate::player::PlayerState::spawn(
            loadout.character,
            &tables.characters[loadout.character as usize],
        );
        let p = &mut w.body.players[0];
        p.power = loadout.power.min(crate::items::POWER_MAX);
        p.lives = loadout.lives;
        p.bombs = loadout.bombs;
        w.body.set_var(crate::consts::GVAR_RANK, rank);
        let root_idx = w.start_main(image)?;
        if let Some(ip) = landing {
            w.tasks.slots[root_idx as usize].pc = ip;
        }
        Ok(w)
    }

    /// 正典开局(spec 2026-07-24 §2.3)——回放可移植性与联机握手 §7.2"初始状态由
    /// 双方从同一确定性初始化各自构造"的**唯一入口**:new + 写 `GVAR_RANK` +
    /// `start_main`(Stage 属主)。场景实体摆放归脚本(`spawn_enemy`/`boss_set`/
    /// `spell_begin` 均为 builtin);**编译不下沉**,只吃成品镜像(依赖方向不可反转)。
    /// rainbow 金向量的手摆 boss boot 是冻结遗产,不迁移(spec §2.3)。
    ///
    /// 委托 `new_game_at(seed, rank, 0, Loadout::default(), image)`(整局流程刀 spec
    /// §2.2):零行为差——默认装备与从头启动逐位同旧实现(见判别测试
    /// `new_game_delegates_bitwise_to_default_path`)。
    pub fn new_game(
        seed: u64,
        rank: i32,
        image: &crate::ecl::image::EclImage,
    ) -> Result<Box<World>, crate::ecl::binding::TaskStartError> {
        Self::new_game_at(seed, rank, 0, crate::player::Loadout::default(), image)
    }

    /// 整块快照（安全逐字段，I7/D11）。
    pub fn copy_into(&self, dst: &mut World) {
        let s = &self.body;
        let d = &mut dst.body;
        d.frame = s.frame;
        d.rng = s.rng;
        d.globals = s.globals;
        s.bullets.copy_into(&mut d.bullets);
        d.players = s.players; // [PlayerState; N] 是 Copy
        d.boss_ui = s.boss_ui;
        d.spells = s.spells; // [SpellSlot; MAX_BOSSES] 是 Copy
        d.spell_seq = s.spell_seq; // [u16; MAX_BOSSES] 持久代际计数器，随快照往返（ABA 修复）
        d.bgm_id = s.bgm_id;
        d.bg_id = s.bg_id;
        d.bg_phase = s.bg_phase;
        d.bg_phase_frame = s.bg_phase_frame;
        s.shots.copy_into(&mut d.shots);
        s.enemies.copy_into(&mut d.enemies);
        s.fields.copy_into(&mut d.fields);
        s.items.copy_into(&mut d.items);
        s.xforms.copy_into(&mut d.xforms);
        d.signals = s.signals;
        d.diag = s.diag;
        d.last_status = s.last_status;
        // 帧内私有输出缓冲（hits/events/reqs）checksum-skip、不随快照复制数组本体——安全性今天靠
        // "begin 在任何生产者跑之前清 len" 这条相位顺序撑着。但 events 是 pub，规格给了两个未来
        // 消费者（phase-8 ECL 钩子、M2 表现层）；若表现层在 rollback 恢复后读到清旧数组前的
        // events，会重放刚回滚掉的帧里的"幽灵死亡"。显式清 len，把这条从相位顺序的巧合变成
        // 明写的契约：恢复出的 World 必须无陈旧输出。
        d.hits_len = 0;
        d.events_len = 0;
        d.reqs_len = 0;
        #[cfg(debug_assertions)]
        {
            d.phase_guard = s.phase_guard;
        }
        self.tasks.copy_into(&mut dst.tasks);
        dst.ecl_main_started = self.ecl_main_started;
        dst.tables_hash = self.tables_hash;
        dst.seed = self.seed;
    }

    #[inline]
    pub fn checksum(&self) -> u64 {
        crate::checksum::Checksum::checksum(self)
    }

    /// 存档(spec L1):身份头 v1 + 字段级规范字节载荷。~2-3ms(大头是载荷 FNV),随地存档
    /// 零感知。`image` 只取 content_hash 入头(载荷不含镜像——静态数据不进 World,I7)。
    pub fn save_bytes(&self, image: &crate::ecl::image::EclImage) -> Vec<u8> {
        use crate::save::SaveBytes;
        let mut payload = Vec::with_capacity(1 << 20);
        SaveBytes::write_bytes(self, &mut payload);
        let fnv = crate::checksum::fnv1a64(&payload);
        let mut out = Vec::with_capacity(crate::save::SAVE_HEADER_LEN + payload.len());
        out.extend_from_slice(&crate::save::SAVE_MAGIC);
        out.push(crate::save::SAVE_FILE_VER);
        out.extend_from_slice(&crate::ENGINE_VER.to_le_bytes());
        out.extend_from_slice(&self.tables_hash.to_le_bytes());
        out.extend_from_slice(&image.content_hash().to_le_bytes());
        out.extend_from_slice(&self.seed.to_le_bytes());
        out.extend_from_slice(&self.body.frame().to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&fnv.to_le_bytes());
        out.extend_from_slice(&payload);
        out
    }

    /// 读档:头校验 + coherence(任一侧 0 = 未绑定跳过,同 `start_main` 守卫口径)→
    /// 堆零构造 + 逐字段读回。P4 式 Err 不 panic;skip 字段因零构造契约保持空。
    pub fn load_bytes(
        bytes: &[u8],
        tables: &crate::tables::WorldTables,
        image: &crate::ecl::image::EclImage,
    ) -> Result<Box<World>, crate::save::LoadError> {
        use crate::save::{LoadError, SaveBytes, SaveReader};
        let mut r = SaveReader::new(bytes);
        if r.take(4)? != crate::save::SAVE_MAGIC {
            return Err(LoadError::BadMagic);
        }
        let ver = r.take(1)?[0];
        if ver != crate::save::SAVE_FILE_VER {
            return Err(LoadError::BadFileVer { got: ver });
        }
        let eng = u32::from_le_bytes(r.take(4)?.try_into().unwrap());
        if eng != crate::ENGINE_VER {
            return Err(LoadError::EngineVerMismatch {
                file: eng,
                engine: crate::ENGINE_VER,
            });
        }
        let f_tables = u64::from_le_bytes(r.take(8)?.try_into().unwrap());
        let f_image = u64::from_le_bytes(r.take(8)?.try_into().unwrap());
        let _seed = u64::from_le_bytes(r.take(8)?.try_into().unwrap());
        let _frame = u32::from_le_bytes(r.take(4)?.try_into().unwrap());
        let plen = u32::from_le_bytes(r.take(4)?.try_into().unwrap()) as usize;
        let f_fnv = u64::from_le_bytes(r.take(8)?.try_into().unwrap());
        if f_tables != 0 && tables.content_hash != 0 && f_tables != tables.content_hash {
            return Err(LoadError::TablesMismatch {
                file: f_tables,
                given: tables.content_hash,
            });
        }
        let i_hash = image.content_hash();
        if f_image != 0 && i_hash != 0 && f_image != i_hash {
            return Err(LoadError::ImageMismatch {
                file: f_image,
                given: i_hash,
            });
        }
        let payload = r.take(plen)?;
        if r.remaining() != 0 {
            return Err(LoadError::TrailingBytes {
                left: r.remaining(),
            });
        }
        let fnv = crate::checksum::fnv1a64(payload);
        if fnv != f_fnv {
            return Err(LoadError::HashMismatch {
                file: f_fnv,
                computed: fnv,
            });
        }
        let mut w = World::new(0); // 堆零构造 + 播种——随后整个被字段读回覆盖(含 rng/seed)
        let mut pr = SaveReader::new(payload);
        SaveBytes::read_bytes(&mut *w, &mut pr)?;
        if pr.remaining() != 0 {
            return Err(LoadError::TrailingBytes {
                left: pr.remaining(),
            });
        }
        Ok(w)
    }

    /// 构造用的 RNG 种子（provenance）。见 `World.seed` 字段文档——回放头/握手/调试从这里读回。
    #[inline]
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// 通道 A 只读视图（委派 `WorldBody::view`）——godot/表现层持 `World`，经它读世界状态。
    pub fn view(&self) -> crate::world::WorldView<'_> {
        self.body.view()
    }

    /// 通道 B 出口委派（镜像 `view()`；幂等语义见 `WorldBody::take_requests`）。
    pub fn take_requests(&self) -> &[crate::reqs::RenderReq] {
        self.body.take_requests()
    }

    /// 帧号读口委派(见 `WorldBody::frame`)。
    pub fn frame(&self) -> u32 {
        self.body.frame()
    }

    /// 世界大事记读口委派(见 `WorldBody::frame_events`)。
    pub fn frame_events(&self) -> &[crate::events::Event] {
        self.body.frame_events()
    }

    /// 任务池只读口:mutator 全 `pub(crate)`,`&TaskPool` 交出去只能读——同 `&Pool`
    /// 之于通道 A 的论证(刀 A/通道 A);整池重赋值(D6 事故面)从此路封死。
    pub fn tasks(&self) -> &crate::ecl::task::TaskPool {
        &self.tasks
    }

    /// Internal task spawn (pub(crate) for VM opcodes/syscalls; used by SPAWN op
    /// and sys_create_bullet in ecl::vm and ecl::syscall).
    ///
    /// Validates the script exists in the image, checks it is not CallOnly,
    /// verifies arg count matches parameters.  Bad args → contract_viol + STATUS_BAD_ARGS;
    /// pool full → pool_full[POOL_TASK] + STATUS_POOL_FULL.
    /// Returns `None` on failure (no task created, caller gets -1 or None).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn spawn_sub_internal(
        &mut self,
        ecl: &EclImage,
        script: SubId,
        args: &[i32],
        owner: (u8, u16, u16),
    ) -> Option<u16> {
        let Some(meta) = ecl.sub_meta(script) else {
            self.body.diag.contract_viol = self.body.diag.contract_viol.wrapping_add(1);
            self.body.last_status = STATUS_BAD_ARGS;
            return None;
        };
        if meta.kind() == SubKind::CallOnly
            || ecl
                .param_types(script)
                .is_none_or(|params| params.len() != args.len())
        {
            self.body.diag.contract_viol = self.body.diag.contract_viol.wrapping_add(1);
            self.body.last_status = STATUS_BAD_ARGS;
            return None;
        }
        let frame = self.body.frame;
        match self.tasks.spawn(script, meta.code_entry(), owner, 0, frame) {
            Some(idx) => {
                self.tasks.slots[idx as usize].locals[..args.len()].copy_from_slice(args);
                Some(idx)
            }
            None => {
                self.body.diag.pool_full[POOL_TASK] =
                    self.body.diag.pool_full[POOL_TASK].wrapping_add(1);
                self.body.last_status = STATUS_POOL_FULL;
                None
            }
        }
    }
}

/// 空导演 = 纯世界模拟（P2：空租户零次循环）。
pub fn step(
    world: &mut World,
    tables: &crate::tables::WorldTables,
    ecl: &EclImage,
    input: &crate::input::InputFrame,
) {
    step_with_director(world, tables, ecl, input, |_| {});
}

/// §3.5 宪法顺序（导演槽在 step-3 跑一次）。相位由 world 出，顺序由此焊死，PhaseGuard 押运。
///
/// `tables: &WorldTables`（M0-17 T2 起穿线；D12 既定签名形态）/`ecl: &EclImage`（M1 T2 起
/// 穿线，同款设计）：**都不进 `World`**（I7 无引用；静态数据不进快照/校验和——两机同表/同镜像
/// 由二进制同一性/内容哈希保证）。ECL 任务运行器是相位 2（`PH_DIRECTOR`）导演槽的默认租户
/// （P2 既定），在注入的导演闭包**之前**跑（"二者共存"，见 `crate::ecl::vm::run_tasks` 文档）。
pub fn step_with_director<F: FnMut(&mut WorldBody)>(
    world: &mut World,
    tables: &crate::tables::WorldTables,
    ecl: &EclImage,
    input: &crate::input::InputFrame,
    mut director: F,
) {
    let b = &mut world.body;
    b.begin(); // 0
    b.decode_input(input); // 1
    b.phase_enter(PH_DIRECTOR); // 2：导演槽（护栏在组装层押）
    crate::ecl::vm::run_tasks(&mut world.tasks, b, ecl, tables); // 默认租户：ECL 任务运行器先跑
    director(b);
    b.update_players(tables); // 3
    b.run_transforms(); // 4
    b.integrate(tables); // 5
    b.collide(tables); // 6
    b.settle(tables); // 7
    b.phase_enter(PH_ECL_HOOK); // 8：ECL 事件挂钩槽（M0-4 空）
    b.cleanup(); // 9
    b.advance(); // 10
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bullets::{BulletHandle, BulletInit, BulletPool};
    use crate::ecl::image::{EclValueType, EntryInit, SubInit, SubKind, test_image};
    use crate::ecl::ops::*;
    use crate::ecl::task::{OWNER_BULLET, OWNER_ENEMY, OWNER_STAGE, TASK_CAP};
    use crate::input::InputFrame;
    use crate::math::{Angle, Fx};
    use crate::world::{POOL_BULLET, STATUS_POOL_FULL};

    fn root_image(code: Vec<u32>) -> EclImage {
        test_image(
            code,
            vec![SubInit::new(0, SubKind::Root, vec![])],
            vec![],
            Some(0),
        )
    }

    fn multi_image(code: Vec<u32>, specs: &[(u32, SubKind, usize)]) -> EclImage {
        let subs = specs
            .iter()
            .map(|&(entry, kind, params)| {
                SubInit::new(entry, kind, vec![EclValueType::Int; params])
            })
            .collect();
        let entries = specs
            .iter()
            .enumerate()
            .filter(|(_, (_, kind, _))| *kind == SubKind::Async)
            .map(|(index, _)| EntryInit::new(format!("async_{index:05}"), index as u16))
            .collect();
        let root = specs
            .iter()
            .position(|(_, kind, _)| *kind == SubKind::Root)
            .map(|index| index as u16);
        test_image(code, subs, entries, root)
    }

    fn spawn_test(
        world: &mut World,
        image: &EclImage,
        raw: u16,
        owner: (u8, u16, u16),
    ) -> Option<u16> {
        world.spawn_sub_internal(image, image.sub_id(raw)?, &[], owner)
    }

    fn straight(x: i32, y: i32, vx: i32, vy: i32, life: u16) -> BulletInit {
        BulletInit {
            x: Fx::from_int(x),
            y: Fx::from_int(y),
            vx: Fx::from_int(vx),
            vy: Fx::from_int(vy),
            speed: Fx::ZERO,
            angle: Angle::ZERO,
            ang_vel: 0,
            accel: Fx::ZERO,
            ax: Fx::ZERO,
            ay: Fx::ZERO,
            sprite: 0,
            radius: Fx::from_int(2),
            delay: 0,
            life,
            flags: 0,
            grazed_by: 0,
            transform_head: 0xFFFF,
            xform_wait: 0,
            xform_next: 0,
        }
    }

    #[test]
    fn new_records_default_table_hash() {
        let w = World::new(0x1234);
        assert_ne!(w.tables_hash, 0);
        assert_eq!(w.tables_hash, crate::tables::TABLES_V0.content_hash);
    }

    #[test]
    fn new_records_seed_and_snapshot_copies_it() {
        let w = World::new(0xDEAD_BEEF);
        assert_eq!(w.seed(), 0xDEAD_BEEF);
        // 随快照复制——否则恢复出的 World 校验和会分叉。
        let mut dst = World::new(0x0000_0001); // 不同种子
        w.copy_into(&mut dst);
        assert_eq!(dst.seed(), 0xDEAD_BEEF, "copy_into 必须复制 seed");
    }

    #[test]
    fn create_bullet_ok_and_pool_full() {
        let mut w = World::new(1);
        assert_ne!(
            w.body.create_bullet(straight(0, 0, 0, 0, 0xFFFF)),
            BulletHandle::NULL
        );
        for _ in 0..(BulletPool::CAP - 1) {
            w.body.create_bullet(straight(0, 0, 0, 0, 0xFFFF));
        }
        assert_eq!(
            w.body.create_bullet(straight(0, 0, 0, 0, 0xFFFF)),
            BulletHandle::NULL
        );
        assert_eq!(w.body.diag.pool_full[POOL_BULLET], 1);
        assert_eq!(w.body.last_status, STATUS_POOL_FULL);
    }

    fn slot(wait: u16, op: u8, a0: i32, a1: i32) -> crate::xform::XformSlot {
        crate::xform::XformSlot {
            wait,
            op,
            _pad: 0,
            args: [a0, a1],
        }
    }

    /// 成功路径：序列拷贝进段、尾部清零（复用段的陈值不可泄漏）、transform_head 被覆写。
    #[test]
    fn create_with_xform_copies_and_zero_fills_tail() {
        let mut w = World::new(1);
        // 先污染 0 号段（占用→写脏→还段），验证复用时尾零
        let s0 = w.body.xforms.alloc().unwrap();
        for sl in w.body.xforms.seg_slots_mut(s0) {
            sl.args[0] = -1;
        }
        w.body.xforms.free(s0);
        let seq = [slot(3, crate::xform::OP_SET_SPEED, 65536, 0)];
        let h = w
            .body
            .create_bullet_with_xform(straight(0, 0, 0, 0, 0xFFFF), &seq);
        assert_ne!(h, BulletHandle::NULL);
        let i = w.body.bullets.get(h).unwrap();
        let seg = w.body.bullets.transform_head[i];
        assert_eq!(seg, 0, "最低空段");
        let slots = w.body.xforms.seg_slots(seg);
        assert_eq!(slots[0], seq[0]);
        assert!(
            slots[1..].iter().all(|s| *s == Default::default()),
            "尾部必须清零 = 天然 END"
        );
    }

    /// 坏参整体失败：>16 槽 / 含未知 op → NULL + BAD_ARGS 计数 + 零副作用（弹与段都不产生）。
    #[test]
    fn create_with_xform_bad_args_total_failure() {
        let mut w = World::new(1);
        let long = [slot(0, crate::xform::OP_SET_SPEED, 1, 0); 17];
        assert_eq!(
            w.body
                .create_bullet_with_xform(straight(0, 0, 0, 0, 1), &long),
            BulletHandle::NULL
        );
        let unknown = [slot(0, 99, 0, 0)]; // 99 = 族外垃圾值，未实现
        assert_eq!(
            w.body
                .create_bullet_with_xform(straight(0, 0, 0, 0, 1), &unknown),
            BulletHandle::NULL
        );
        assert_eq!(w.body.diag.contract_viol, 2);
        assert_eq!(w.body.last_status, crate::world::STATUS_BAD_ARGS);
        assert_eq!(w.body.bullets.iter_alive().count(), 0);
        assert_eq!(w.body.xforms.alloc().unwrap(), 0, "无段泄漏");
    }

    /// 段满 → NULL + POOL_FULL(XFORM)，零副作用。
    #[test]
    fn create_with_xform_segpool_full() {
        let mut w = World::new(1);
        for _ in 0..crate::xform::SEG_CAP {
            w.body.xforms.alloc().unwrap();
        }
        let seq = [slot(0, crate::xform::OP_SET_SPEED, 1, 0)];
        assert_eq!(
            w.body
                .create_bullet_with_xform(straight(0, 0, 0, 0, 1), &seq),
            BulletHandle::NULL
        );
        assert_eq!(w.body.diag.pool_full[crate::world::POOL_XFORM], 1);
        assert_eq!(
            w.body.bullets.iter_alive().count(),
            0,
            "宁缺勿哑：弹也不产生"
        );
    }

    /// 弹池满 → 还段回滚（先段后弹的另一半）。
    #[test]
    fn create_with_xform_bulletpool_full_rolls_back_segment() {
        let mut w = World::new(1);
        for _ in 0..BulletPool::CAP {
            w.body.create_bullet(straight(0, 0, 0, 0, 0xFFFF));
        }
        let seq = [slot(0, crate::xform::OP_SET_SPEED, 1, 0)];
        assert_eq!(
            w.body
                .create_bullet_with_xform(straight(0, 0, 0, 0, 1), &seq),
            BulletHandle::NULL
        );
        assert_eq!(w.body.diag.pool_full[POOL_BULLET], 1);
        assert_eq!(w.body.xforms.alloc().unwrap(), 0, "段已回滚归还");
    }

    /// A1-(2)：扩展槽的字节不判 op——scratch 位置放任意垃圾值也必须过 create。
    #[test]
    fn create_validation_skips_extension_slots() {
        let mut w = World::new(1);
        let seq = [
            slot(0, crate::xform::OP_STEP_SPEED, 65536, 4),
            slot(0, 99, -1, -1), // 扩展槽：垃圾字节合法（会被 fire 时的 scratch 覆写）
            slot(0, crate::xform::OP_SET_SPRITE, 1, 0),
        ];
        assert_ne!(
            w.body
                .create_bullet_with_xform(straight(0, 0, 0, 0, 1), &seq),
            BulletHandle::NULL,
            "scratch 槽不得被当 op 判"
        );
    }

    /// STEP 在末槽（槽 15）没有扩展槽空间 → BAD_ARGS 整体失败。
    #[test]
    fn create_rejects_step_without_extension_room() {
        let mut w = World::new(1);
        let mut seq = [slot(0, crate::xform::OP_SET_SPRITE, 0, 0); 16];
        seq[15] = slot(0, crate::xform::OP_STEP_SPEED, 65536, 4);
        assert_eq!(
            w.body
                .create_bullet_with_xform(straight(0, 0, 0, 0, 1), &seq),
            BulletHandle::NULL
        );
        assert_eq!(w.body.last_status, crate::world::STATUS_BAD_ARGS);
    }

    /// A1-(3)：LOOP target 指进扩展槽中间 → BAD_ARGS；指向 zero-tail（END）→ 合法。
    #[test]
    fn create_validates_loop_target_boundaries() {
        let mut w = World::new(1);
        let bad = [
            slot(0, crate::xform::OP_STEP_SPEED, 65536, 4), // 槽0（扩展槽=1）
            slot(0, 0, 0, 0),                               // 扩展槽
            slot(0, crate::xform::OP_LOOP, 1, 0),           // target=1 = 扩展槽中间 → 拒
        ];
        assert_eq!(
            w.body
                .create_bullet_with_xform(straight(0, 0, 0, 0, 1), &bad),
            BulletHandle::NULL
        );
        let ok = [
            slot(0, crate::xform::OP_SET_SPRITE, 1, 0),
            slot(0, crate::xform::OP_LOOP, 10, 3), // target=10 在 zero-tail：落地即 END，合法
        ];
        assert_ne!(
            w.body
                .create_bullet_with_xform(straight(0, 0, 0, 0, 1), &ok),
            BulletHandle::NULL
        );
    }

    /// easing id ≥ 8 → BAD_ARGS（作者错误 create 期就拒）。
    #[test]
    fn create_rejects_bad_easing_id() {
        let mut w = World::new(1);
        let seq = [slot(0, crate::xform::OP_STEP_SPEED, 65536, 4 | (8 << 16))];
        assert_eq!(
            w.body
                .create_bullet_with_xform(straight(0, 0, 0, 0, 1), &seq),
            BulletHandle::NULL
        );
    }

    /// create_bullet（哑弹路径）覆写 transform_head——调用方伪造段号无效。
    #[test]
    fn create_bullet_overrides_forged_transform_head() {
        let mut w = World::new(1);
        let mut init = straight(0, 0, 0, 0, 0xFFFF);
        init.transform_head = 7; // 伪造
        let h = w.body.create_bullet(init);
        let i = w.body.bullets.get(h).unwrap();
        assert_eq!(w.body.bullets.transform_head[i], crate::xform::XFORM_NONE);
    }

    /// 网格几何 + 迭代序判别：3 角 × 2 速——槽 idx = i×2+k，每颗 vx/vy 与
    /// polar_to_vec(speed0+k·Δs, angle0+i·Δa) 参考逐位相等（角度外层速度内层的序被槽号钉死）。
    #[test]
    fn batch_grid_geometry_and_slot_order() {
        use crate::math::geom::polar_to_vec;
        let mut w = World::new(1);
        let n = w.body.create_bullets_batch(
            straight(0, 100, 0, 0, 0xFFFF),
            &[],
            3,
            Angle(4096),
            8192,
            2,
            Fx::from_int(1),
            Fx::from_raw(32768), // 1.0 步进 0.5
        );
        assert_eq!(n, 6);
        for i in 0..3u16 {
            for k in 0..2u16 {
                let slot = (i * 2 + k) as usize;
                let ang = Angle(4096).add_delta((8192 * i as i32) as i16);
                let spd = Fx::from_raw(65536 + 32768 * k as i32);
                let (rvx, rvy) = polar_to_vec(spd, ang);
                assert_eq!(w.body.bullets.vx[slot], rvx, "槽 {slot} vx（角外速内序）");
                assert_eq!(w.body.bullets.vy[slot], rvy, "槽 {slot} vy");
                assert_eq!(w.body.bullets.speed[slot], spd);
                assert_eq!(w.body.bullets.angle[slot], ang);
            }
        }
    }

    /// 环回绕：8-way 整环从 61440 起步——第 2 颗角度过 65536 自动回绕闭合。
    #[test]
    fn batch_ring_wraps_full_circle() {
        let mut w = World::new(1);
        let n = w.body.create_bullets_batch(
            straight(0, 100, 0, 0, 0xFFFF),
            &[],
            8,
            Angle(61440),
            8192,
            1,
            Fx::from_int(2),
            Fx::ZERO,
        );
        assert_eq!(n, 8);
        assert_eq!(
            w.body.bullets.angle[1],
            Angle(4096),
            "61440+8192 回绕 = 4096"
        );
        assert_eq!(w.body.bullets.angle[7], Angle(53248));
    }

    /// 超量与零轴整体拒：实发 0 + BAD_ARGS + 零副作用。
    #[test]
    fn batch_rejects_oversize_and_zero_axis() {
        let mut w = World::new(1);
        let cv0 = w.body.diag.contract_viol;
        assert_eq!(
            w.body.create_bullets_batch(
                straight(0, 0, 0, 0, 1),
                &[],
                100,
                Angle::ZERO,
                0,
                100,
                Fx::ONE,
                Fx::ZERO
            ),
            0,
            "100×100 > 8192 拒"
        );
        assert_eq!(
            w.body.create_bullets_batch(
                straight(0, 0, 0, 0, 1),
                &[],
                0,
                Angle::ZERO,
                0,
                5,
                Fx::ONE,
                Fx::ZERO
            ),
            0,
            "零轴拒"
        );
        assert_eq!(w.body.diag.contract_viol, cv0 + 2);
        assert_eq!(w.body.last_status, crate::world::STATUS_BAD_ARGS);
        assert_eq!(w.body.bullets.iter_alive().count(), 0, "零副作用");
    }

    /// 模板 radius 钳制恰计一次（不逐颗累加）。
    #[test]
    fn batch_clamps_template_radius_once() {
        let mut w = World::new(1);
        let mut init = straight(0, 100, 0, 0, 0xFFFF);
        init.radius = Fx::from_int(5000); // 越 MAX_ENTITY_RADIUS
        let cv0 = w.body.diag.contract_viol;
        let n = w
            .body
            .create_bullets_batch(init, &[], 4, Angle::ZERO, 1000, 1, Fx::ONE, Fx::ZERO);
        assert_eq!(n, 4);
        assert_eq!(w.body.diag.contract_viol, cv0 + 1, "钳制恰计一次");
        for s in 0..4 {
            assert_eq!(w.body.bullets.radius[s], crate::world::MAX_ENTITY_RADIUS);
        }
    }

    /// 池满尽力而为：预占到只剩 3，请求 2×3=6 → 实发 3 + pool_full[BULLET] += 3。
    #[test]
    fn batch_partial_on_pool_full() {
        let mut w = World::new(1);
        for _ in 0..(BulletPool::CAP - 3) {
            w.body.create_bullet(straight(0, 0, 0, 0, 0xFFFF));
        }
        let pf0 = w.body.diag.pool_full[POOL_BULLET];
        let n = w.body.create_bullets_batch(
            straight(0, 100, 0, 0, 0xFFFF),
            &[],
            2,
            Angle::ZERO,
            1000,
            3,
            Fx::from_int(1),
            Fx::from_raw(16384),
        );
        assert_eq!(n, 3, "尽力而为发满剩余额度");
        assert_eq!(
            w.body.diag.pool_full[POOL_BULLET],
            pf0 + 3,
            "剩余 3 颗批量计数"
        );
        assert_eq!(w.body.last_status, crate::world::STATUS_POOL_FULL);
    }

    /// xform 批：每颗自有段（transform_head 互异、段内容 = 序列拷贝+尾零）。
    #[test]
    fn batch_with_xform_gives_each_own_segment() {
        let mut w = World::new(1);
        let seq = [slot(5, crate::xform::OP_SET_SPEED, 131072, 0)];
        let n = w.body.create_bullets_batch(
            straight(0, 100, 0, 0, 0xFFFF),
            &seq,
            2,
            Angle::ZERO,
            16384,
            2,
            Fx::from_int(1),
            Fx::from_raw(32768),
        );
        assert_eq!(n, 4);
        let heads: Vec<u16> = (0..4).map(|s| w.body.bullets.transform_head[s]).collect();
        assert_eq!(heads, vec![0, 1, 2, 3], "每颗自有段、分配序 = 槽序");
        for &seg in &heads {
            assert_eq!(w.body.xforms.seg_slots(seg)[0], seq[0], "段内容 = 拷贝");
            assert_eq!(w.body.xforms.seg_slots(seg)[1], Default::default(), "尾零");
        }
    }

    /// xform 坏序列：开跑前整体拒（实发 0、无弹无段泄漏）。
    #[test]
    fn batch_bad_xform_rejected_upfront() {
        let mut w = World::new(1);
        let bad = [slot(0, 99, 0, 0)];
        assert_eq!(
            w.body.create_bullets_batch(
                straight(0, 0, 0, 0, 1),
                &bad,
                4,
                Angle::ZERO,
                0,
                1,
                Fx::from_int(1),
                Fx::ZERO
            ),
            0
        );
        assert_eq!(w.body.last_status, crate::world::STATUS_BAD_ARGS);
        assert_eq!(w.body.bullets.iter_alive().count(), 0);
        assert_eq!(w.body.xforms.alloc().unwrap(), 0, "无段泄漏");
    }

    /// 验证序判别（轴→xform→radius）：同时坏 radius + 坏 xform 时 xform 拒先短路，
    /// contract_viol 恰 +1——若有人把 radius 钳挪到 xform 检查前（对齐单发 API）会变 +2 变红。
    #[test]
    fn batch_bad_radius_plus_bad_xform_counts_once() {
        let mut w = World::new(1);
        let mut init = straight(0, 100, 0, 0, 0xFFFF);
        init.radius = Fx::from_int(5000); // 越 MAX_ENTITY_RADIUS
        let bad = [slot(0, 99, 0, 0)];
        let cv0 = w.body.diag.contract_viol;
        assert_eq!(
            w.body.create_bullets_batch(
                init,
                &bad,
                2,
                Angle::ZERO,
                0,
                2,
                Fx::from_int(1),
                Fx::ZERO
            ),
            0
        );
        assert_eq!(
            w.body.diag.contract_viol,
            cv0 + 1,
            "xform 拒先短路，radius 钳不再计"
        );
        assert_eq!(w.body.last_status, crate::world::STATUS_BAD_ARGS);
    }

    /// 段满短路（xform 批）：段池只剩 2，请求 2×2 → 实发 2 + pool_full[XFORM] += 2。
    #[test]
    fn batch_partial_on_segpool_full() {
        let mut w = World::new(1);
        for _ in 0..(crate::xform::SEG_CAP - 2) {
            w.body.xforms.alloc().unwrap();
        }
        let seq = [slot(0, crate::xform::OP_SET_SPRITE, 1, 0)];
        let n = w.body.create_bullets_batch(
            straight(0, 100, 0, 0, 0xFFFF),
            &seq,
            2,
            Angle::ZERO,
            1000,
            2,
            Fx::from_int(1),
            Fx::ZERO,
        );
        assert_eq!(n, 2);
        assert_eq!(
            w.body.diag.pool_full[crate::world::POOL_XFORM],
            2,
            "剩余 2 颗计入段池计数"
        );
    }

    #[test]
    fn new_is_deterministic_and_seed_matters() {
        assert_eq!(World::new(42).checksum(), World::new(42).checksum());
        assert_ne!(World::new(1).checksum(), World::new(2).checksum()); // 种子入 rng 入校验和
    }

    #[test]
    fn new_spawns_player0_alive() {
        use crate::player::{LIFE_ABSENT, LIFE_ALIVE};
        let w = World::new(1);
        assert_eq!(w.body.players[0].life_state, LIFE_ALIVE);
        assert_eq!(w.body.players[0].lives, 3);
        assert_eq!(w.body.players[0].y, Fx::from_int(384));
        assert_eq!(w.body.players[1].life_state, LIFE_ABSENT); // 第 2 人不在场
    }

    #[test]
    fn player_moves_right_with_input() {
        use crate::input::BTN_RIGHT;
        let mut w = World::new(1);
        let x0 = w.body.players[0].x.raw();
        let mut f = InputFrame::empty(0);
        f.actions[0].buttons = BTN_RIGHT;
        crate::world::test_support::step_t(&mut w, &f);
        assert!(w.body.players[0].x.raw() > x0); // 右移
    }

    #[test]
    fn player_shot_fires_on_button() {
        use crate::input::BTN_SHOT;
        let mut w = World::new(1);
        let mut f = InputFrame::empty(0);
        f.actions[0].buttons = BTN_SHOT;
        crate::world::test_support::step_t(&mut w, &f);
        assert_eq!(w.body.shots.iter_alive().count(), 1); // shot_cd 从 0 → 发 1 发
    }

    #[test]
    fn player_clamped_to_left_edge() {
        use crate::input::BTN_LEFT;
        let mut w = World::new(1);
        let mut f = InputFrame::empty(0);
        f.actions[0].buttons = BTN_LEFT;
        for _ in 0..200 {
            crate::world::test_support::step_t(&mut w, &f); // 一直左移
        }
        assert_eq!(w.body.players[0].x, Fx::from_int(-192)); // 钳到左边界
    }

    #[test]
    fn player_deterministic_with_scripted_input() {
        use crate::input::{BTN_RIGHT, BTN_SHOT};
        let run = || {
            let mut w = World::new(9);
            let mut cks = Vec::new();
            for frame in 0..60u32 {
                let mut f = InputFrame::empty(frame);
                f.actions[0].buttons = BTN_RIGHT | BTN_SHOT;
                crate::world::test_support::step_t(&mut w, &f);
                cks.push(w.checksum());
            }
            cks
        };
        assert_eq!(run(), run()); // 玩家 + 自机弹演化确定
    }

    #[test]
    fn integrate_moves_and_advances_frame() {
        let mut w = World::new(1);
        let h = w.body.create_bullet(straight(0, 0, 1, 2, 0xFFFF));
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(0));
        let i = w.body.bullets.get(h).unwrap();
        assert_eq!(w.body.bullets.x[i], Fx::from_int(1));
        assert_eq!(w.body.bullets.y[i], Fx::from_int(2));
        assert_eq!(w.body.frame, 1);
    }

    #[test]
    fn cleanup_frees_out_of_bounds_and_expired() {
        let mut w = World::new(1);
        let h_far = w.body.create_bullet(straight(1000, 0, 0, 0, 0xFFFF)); // 越界
        let h_life = w.body.create_bullet(straight(0, 0, 0, 0, 1)); // 寿命 1
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(0)); // life:1→0(integrate)，cleanup 释放两者
        assert_eq!(w.body.bullets.get(h_far), None);
        assert_eq!(w.body.bullets.get(h_life), None);
    }

    #[test]
    fn deterministic_replay() {
        let run = || {
            let mut w = World::new(7);
            let mut cks = Vec::new();
            for _ in 0..50u32 {
                step_with_director(
                    &mut w,
                    &crate::tables::TABLES_V0,
                    &EclImage::empty(),
                    &InputFrame::empty(0),
                    |b| {
                        let vx = b.rng.rand_range(5) as i32 - 2;
                        b.create_bullet(straight(0, 0, vx, 3, 100));
                    },
                );
                cks.push(w.checksum());
            }
            cks
        };
        assert_eq!(run(), run()); // 同种子+同导演 → 逐帧 checksum 全等
    }

    #[test]
    fn snapshot_restore_roundtrip() {
        let mut w = World::new(3);
        for _ in 0..10 {
            step_with_director(
                &mut w,
                &crate::tables::TABLES_V0,
                &EclImage::empty(),
                &InputFrame::empty(0),
                |b| {
                    b.create_bullet(straight(0, 0, 1, 1, 200));
                },
            );
        }
        let snap_ck = w.checksum();
        let mut snap = World::new(3);
        w.copy_into(&mut snap);
        assert_eq!(snap.checksum(), snap_ck);
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(0));
        assert_ne!(w.checksum(), snap_ck);
        snap.copy_into(&mut w); // 恢复
        assert_eq!(w.checksum(), snap_ck);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "相位乱序")]
    fn phase_guard_catches_out_of_order() {
        let mut w = World::new(1);
        w.body.advance(); // 不经 begin 直接 advance → 护栏 panic
    }

    #[test]
    fn snapshot_roundtrip_covers_xform_pool() {
        let mut w = World::new(3);
        let seg = w.body.xforms.alloc().unwrap();
        w.body.xforms.seg_slots_mut(seg)[0].args[0] = 42;
        let ck = w.checksum();
        let mut snap = World::new(3);
        w.copy_into(&mut snap);
        assert_eq!(snap.checksum(), ck); // 段池随快照
        snap.body.xforms.seg_slots_mut(seg)[0].args[0] = 43;
        assert_ne!(snap.checksum(), ck); // 且真的在参与指纹
    }

    #[test]
    fn snapshot_covers_item_pool() {
        let mut w = World::new(3);
        w.body
            .items
            .alloc(crate::items::ItemInit {
                x: Fx::from_int(5),
                y: Fx::from_int(6),
                vx: Fx::ZERO,
                vy: Fx::ZERO,
                item_type: crate::items::ITEM_POINT,
                magnet_to: crate::items::MAGNET_NONE,
                timer: 0,
            })
            .unwrap();
        let ck = w.checksum();
        let mut snap = World::new(3);
        w.copy_into(&mut snap);
        assert_eq!(snap.checksum(), ck);
        let i = 0;
        snap.body.items.item_type[i] ^= 1;
        assert_ne!(snap.checksum(), ck, "items 必须真的参与校验和");
    }

    /// M1 T1：`World.tasks` 快照往返 + 校验和敏感（M0-15 同款判别）——写一个从未 `spawn`
    /// 过的槽字段，checksum 必须变（P6 哈希全槽不看存活位）；`copy_into` 漏拷 `tasks` 即红。
    #[test]
    fn snapshot_covers_tasks_pool() {
        let mut w = World::new(3);
        let ck0 = w.checksum();
        w.tasks.slots[5].pc = 42; // 槽 5 从未 spawn 过——专挑"只哈希占用槽"的变异体
        let ck1 = w.checksum();
        assert_ne!(ck1, ck0, "tasks 池必须入校验和（哈希全槽不看 alive）");

        let mut snap = World::new(3);
        w.copy_into(&mut snap);
        assert_eq!(snap.checksum(), ck1, "快照必须带 tasks 池全部字节");
        w.tasks.slots[5].pc = 0;
        snap.copy_into(&mut w); // 恢复
        assert_eq!(
            w.checksum(),
            ck1,
            "restore 必须还原 tasks 池（copy_into 漏拷即红）"
        );
    }

    /// M1 T2：空镜像穿线不 panic、零任务零行为（`EclImage::empty()` 是金向量一号的常态输入）。
    #[test]
    fn empty_image_step_runs_without_panic_and_zero_tasks() {
        let mut w = World::new(1);
        let ecl = EclImage::empty();
        for f in 0..10u32 {
            step(
                &mut w,
                &crate::tables::TABLES_V0,
                &ecl,
                &InputFrame::empty(f),
            );
        }
        assert_eq!(w.tasks.iter_alive().count(), 0);
        assert_eq!(w.body.diag.task_faults, 0);
    }

    /// M1 T2：`spawn_task` 次帧首跑——出生帧（`born_frame == frame`）门禁挡在 owner/wait 门禁
    /// 之外（本任务 owner=STAGE 恒过、wait 恰为 0，唯一能挡它的只有 born_frame 门禁）：
    /// 出生当帧 locals 原封不动，次帧起首次执行才写入。
    #[test]
    fn spawn_task_next_frame_first_run() {
        let ecl = root_image(vec![
            OP_PUSHI as u32,
            1,
            OP_POPL as u32,
            0,
            OP_PUSHI as u32,
            5,
            OP_WAIT as u32,
        ]);
        let mut w = World::new(1);
        let idx = spawn_test(&mut w, &ecl, 0, (OWNER_STAGE, 0, 0)).unwrap();

        step(
            &mut w,
            &crate::tables::TABLES_V0,
            &ecl,
            &InputFrame::empty(0),
        );
        assert_eq!(
            w.tasks.slots[idx as usize].locals[0], 0,
            "born_frame 门禁：出生当帧不跑"
        );
        assert!(w.tasks.is_alive(idx as usize));

        step(
            &mut w,
            &crate::tables::TABLES_V0,
            &ecl,
            &InputFrame::empty(1),
        );
        assert_eq!(
            w.tasks.slots[idx as usize].locals[0], 1,
            "次帧首跑：locals 写入生效"
        );
        assert_eq!(w.tasks.slots[idx as usize].wait, 5);
    }

    /// M1.9 T3 Commit A：`OP_SPAWN` 带参端到端——**次帧首跑语义不受带参扩展影响**（子任务
    /// 出生帧不跑，locals 在出生帧就已经落好实参、只是脚本还没读到它们），且实参在次帧首跑
    /// 时确实可用（子脚本对 `locals[0]+locals[1]` 求和写回 globals，观测真实经过 VM 执行）。
    /// root 正序压栈 `11, 22` 后 `SPAWN` script1 argc=2；`vm.rs` 单测已钉死"落位即声明序"，
    /// 本测试钉的是跨帧调度门禁与真实执行链路的组合，属于纯 `exec` 单测覆盖不到的一层。
    #[test]
    fn opspawn_with_args_next_frame_first_run_and_args_usable() {
        use crate::ecl::syscall::SYS_SET_VAR;
        const SLOT: u16 = 20; // 自由段（≥ GLOBALS_SYS_SEGMENT）

        let root_code = vec![
            OP_PUSHI as u32,
            11, // 0,1
            OP_PUSHI as u32,
            22, // 2,3
            OP_SPAWN as u32,
            1,               // 4,5：script1
            2,               // 6：argc=2
            OP_POP as u32,   // 7：丢弃子句柄
            OP_PUSHI as u32, // 8
            1000,            // 9
            OP_WAIT as u32,  // 10：root 存活，槽号不被复用
        ];
        let child_entry = root_code.len() as u32;
        let child_code = vec![
            OP_PUSHI as u32,
            SLOT as u32, // +0,+1：待写槽号
            OP_PUSHL as u32,
            0, // +2,+3：locals[0]
            OP_PUSHL as u32,
            1,             // +4,+5：locals[1]
            OP_ADD as u32, // +6：栈 = [SLOT, locals[0]+locals[1]]
            OP_SYS as u32,
            SYS_SET_VAR as u32, // +7,+8
            OP_END as u32,      // +9
        ];
        let mut code = root_code.clone();
        code.extend_from_slice(&child_code);
        let ecl = multi_image(
            code,
            &[(0, SubKind::Root, 0), (child_entry, SubKind::Async, 2)],
        );

        let mut w = World::new(1);
        let root = spawn_test(&mut w, &ecl, 0, (OWNER_STAGE, 0, 0)).unwrap();

        step(
            &mut w,
            &crate::tables::TABLES_V0,
            &ecl,
            &InputFrame::empty(0),
        ); // root 出生帧：不跑
        step(
            &mut w,
            &crate::tables::TABLES_V0,
            &ecl,
            &InputFrame::empty(1),
        ); // root 首跑：SPAWN child（born_frame=当前帧）

        let child = (0..TASK_CAP)
            .find(|&i| w.tasks.is_alive(i) && w.tasks.slots[i].parent == root + 1)
            .expect("child 应已生成");
        assert_eq!(
            &w.tasks.slots[child].locals[0..2],
            &[11, 22],
            "child 出生当帧 locals 已落好实参（声明序）——只是脚本还没跑到读它们"
        );
        assert_eq!(
            w.body.globals[SLOT as usize], 0,
            "child 出生当帧不跑（次帧首跑门禁不受带参扩展影响）：globals 尚未被写"
        );

        step(
            &mut w,
            &crate::tables::TABLES_V0,
            &ecl,
            &InputFrame::empty(2),
        ); // child 次帧首跑
        assert_eq!(
            w.body.globals[SLOT as usize], 33,
            "次帧首跑：11+22=33 经真实 VM 执行写入 globals"
        );
        assert!(!w.tasks.is_alive(child), "child 执行到 END 自灭");
        assert_eq!(w.body.diag.task_faults, 0, "全程不应产生 Fault");
    }

    /// M1.5：`SYS_SELF_AGE` 端到端 off-by 语义——任务出生帧 F（`spawn_task` 时 `body.frame`），
    /// 出生当帧不跑（born_frame 门禁），**次帧首跑** `ctx.frame=F+1`，此时 `self_age = 1`（不是
    /// 0）。脚本每帧把 `self_age` 写回一个自由段全局槽（`WAIT(0)+JMP` 回环，每帧恰写一次）——
    /// 跑 N 次 `step` 后，槽值应精确等于 `N-1`（第 0 次 step 撞 born 门禁不写，随后 N-1 次各写
    /// 一次，各次覆盖，终值 = 最后一次的 age = N-1）。此 off-by 是本刀刻意钉死的契约。
    #[test]
    fn sys_self_age_ticks_task_age_each_frame_via_globals_relay() {
        use crate::ecl::syscall::{SYS_SELF_AGE, SYS_SET_VAR};
        const SLOT: u16 = 20; // 自由段（≥ GLOBALS_SYS_SEGMENT）

        let ecl = root_image(vec![
            OP_PUSHI as u32,
            SLOT as u32, // 0,1
            OP_SYS as u32,
            SYS_SELF_AGE as u32, // 2,3
            OP_SYS as u32,
            SYS_SET_VAR as u32, // 4,5
            OP_PUSHI as u32,
            0,              // 6,7：wait(0) 帧数
            OP_WAIT as u32, // 8
            OP_JMP as u32,
            0, // 9,10：回环顶部
        ]);
        let mut w = World::new(1);
        spawn_test(&mut w, &ecl, 0, (OWNER_STAGE, 0, 0)).unwrap();

        const N: u32 = 5;
        for f in 0..N {
            step(
                &mut w,
                &crate::tables::TABLES_V0,
                &ecl,
                &InputFrame::empty(f),
            );
        }
        assert_eq!(
            w.body.globals[SLOT as usize],
            (N - 1) as i32,
            "born 帧不跑（第 0 次 step 撞门禁），随后每次 step 各写一次 age，\
             终值 = 最后一次 step 时的 age = N-1"
        );
        assert_eq!(w.body.diag.task_faults, 0, "全程不应 Fault");
    }

    /// M1.5：`SYS_SELF_HP_MAX` 端到端（经 globals relay 观测）——owner=ENEMY 读到
    /// `enemies.hp_max`（与该敌 `hp` 不同值，逐位命中排除读混字段）；owner=STAGE 恒 0
    /// （同 `SYS_SELF_HP` 误用策略：静默降级不 Fault）。
    #[test]
    fn sys_self_hp_max_relays_via_globals_for_enemy_and_stage_owner() {
        use crate::ecl::syscall::{SYS_SELF_HP_MAX, SYS_SET_VAR};
        const SLOT: u16 = 21;

        let ecl = root_image(vec![
            OP_PUSHI as u32,
            SLOT as u32, // 0,1
            OP_SYS as u32,
            SYS_SELF_HP_MAX as u32, // 2,3
            OP_SYS as u32,
            SYS_SET_VAR as u32, // 4,5
            OP_END as u32,      // 6
        ]);

        // owner=ENEMY：hp=5（存活门禁用）、hp_max 另设 9999——两字段判别式取值。
        let mut w_enemy = World::new(1);
        let h = crate::world::test_support::spawn_enemy(&mut w_enemy, 0, 80, 5);
        w_enemy.body.enemies.hp_max[h.index as usize] = 9999;
        spawn_test(&mut w_enemy, &ecl, 0, (OWNER_ENEMY, h.index, h.generation)).unwrap();
        step(
            &mut w_enemy,
            &crate::tables::TABLES_V0,
            &ecl,
            &InputFrame::empty(0),
        );
        step(
            &mut w_enemy,
            &crate::tables::TABLES_V0,
            &ecl,
            &InputFrame::empty(1),
        );
        assert_eq!(w_enemy.body.globals[SLOT as usize], 9999);
        assert_eq!(w_enemy.body.diag.task_faults, 0);

        // owner=STAGE：恒 0。
        let mut w_stage = World::new(1);
        spawn_test(&mut w_stage, &ecl, 0, (OWNER_STAGE, 0, 0)).unwrap();
        step(
            &mut w_stage,
            &crate::tables::TABLES_V0,
            &ecl,
            &InputFrame::empty(0),
        );
        step(
            &mut w_stage,
            &crate::tables::TABLES_V0,
            &ecl,
            &InputFrame::empty(1),
        );
        assert_eq!(w_stage.body.globals[SLOT as usize], 0, "非敌 owner 恒 0");
        assert_eq!(w_stage.body.diag.task_faults, 0);
    }

    /// M1.5：globals 系统段脚本写保护——端到端穿过真实调度器（`run_tasks`）钉死"no-op
    /// 不是 Fault"：脚本 `set_var(GVAR_RANK, 999)` 撞系统段 guard 后**继续执行**（不被杀、
    /// 不发 `EVT_TASK_FAULT`），紧随其后对自由段槽的写照常生效（用它反证任务没被腰斩）；
    /// 世界 API `set_var` 直写系统段全程不受本 guard 影响（不同门，调用方是可信的 game 层）。
    #[test]
    fn sys_set_var_system_segment_guard_no_op_task_survives_and_world_api_unrestricted() {
        use crate::ecl::syscall::SYS_SET_VAR;
        use crate::world::GVAR_RANK;
        const FREE_SLOT: u16 = 20;

        let ecl = root_image(vec![
            OP_PUSHI as u32,
            GVAR_RANK as u32, // 0,1：系统段槽
            OP_PUSHI as u32,
            999, // 2,3
            OP_SYS as u32,
            SYS_SET_VAR as u32, // 4,5：应 no-op，不 Fault
            OP_PUSHI as u32,
            FREE_SLOT as u32, // 6,7：自由段槽
            OP_PUSHI as u32,
            555, // 8,9
            OP_SYS as u32,
            SYS_SET_VAR as u32, // 10,11：应正常写入——证明任务未被腰斩
            OP_END as u32,      // 12
        ]);
        let mut w = World::new(1);
        w.body.set_var(GVAR_RANK, 111); // game 层建场惯例：世界 API 先写系统段已知基线值
        spawn_test(&mut w, &ecl, 0, (OWNER_STAGE, 0, 0)).unwrap();

        step(
            &mut w,
            &crate::tables::TABLES_V0,
            &ecl,
            &InputFrame::empty(0),
        );
        step(
            &mut w,
            &crate::tables::TABLES_V0,
            &ecl,
            &InputFrame::empty(1),
        );

        assert_eq!(
            w.body.globals[GVAR_RANK as usize], 111,
            "系统段 guard no-op：脚本 999 未落地，真槽值仍是建场基线"
        );
        assert_eq!(w.body.diag.contract_viol, 1);
        assert_eq!(
            w.body.globals[FREE_SLOT as usize], 555,
            "guard 触发后任务继续执行，自由段的后续写正常生效"
        );
        assert_eq!(
            w.body.diag.task_faults, 0,
            "guard 是 no-op 不是 Fault，任务应正常跑完 END 自灭"
        );

        // 世界 API 直写系统段不受本 guard 影响。
        w.body.set_var(GVAR_RANK, 777);
        assert_eq!(w.body.get_var(GVAR_RANK), 777, "世界 API 写系统段仍畅通");
    }

    /// M1 T2：owner=ENEMY 的敌人死亡（池释放）→ 任务次帧被静默回收——不发 `EVT_TASK_FAULT`、
    /// 不计 `task_faults`（owner 死是常态非错误，物理区分于确定性报错杀）。
    #[test]
    fn owner_enemy_death_kills_task_silently_next_frame() {
        let mut w = World::new(1);
        let h = crate::world::test_support::spawn_enemy(&mut w, 0, 80, 5);
        let ecl = root_image(vec![OP_END as u32]);
        let idx = spawn_test(&mut w, &ecl, 0, (OWNER_ENEMY, h.index, h.generation)).unwrap();

        step(
            &mut w,
            &crate::tables::TABLES_V0,
            &ecl,
            &InputFrame::empty(0),
        ); // 出生帧：born_frame 门禁跳过
        assert!(w.tasks.is_alive(idx as usize));
        w.body.enemies.free(h); // 模拟 owner 死亡（脱离正常 settle/cleanup 链路，直测门禁）

        step(
            &mut w,
            &crate::tables::TABLES_V0,
            &ecl,
            &InputFrame::empty(1),
        );
        assert!(!w.tasks.is_alive(idx as usize), "owner 死后任务应被回收");
        assert_eq!(w.body.diag.task_faults, 0, "owner 死不是 Fault");
        assert_eq!(w.body.events_len, 0, "owner 死静默——不发 EVT_TASK_FAULT");
    }

    /// M1 T2：owner=BULLET 同款门禁（与 ENEMY 分支镜像，独立判别覆盖）。
    #[test]
    fn owner_bullet_death_kills_task_silently_next_frame() {
        let mut w = World::new(1);
        let h = crate::world::test_support::bullet_at(&mut w, 0, 0);
        let ecl = root_image(vec![OP_END as u32]);
        let idx = spawn_test(&mut w, &ecl, 0, (OWNER_BULLET, h.index, h.generation)).unwrap();

        step(
            &mut w,
            &crate::tables::TABLES_V0,
            &ecl,
            &InputFrame::empty(0),
        );
        assert!(w.tasks.is_alive(idx as usize));
        w.body.bullets.free(h);

        step(
            &mut w,
            &crate::tables::TABLES_V0,
            &ecl,
            &InputFrame::empty(1),
        );
        assert!(!w.tasks.is_alive(idx as usize), "owner 死后任务应被回收");
        assert_eq!(w.body.diag.task_faults, 0);
        assert_eq!(w.body.events_len, 0);
    }

    /// M1 T2：`wait n` 恰 n 帧后恢复——`WAIT` 执行帧不计入空转，随后 n 帧调度层只递减
    /// 不执行，第 n+1 帧起恢复。
    #[test]
    fn wait_n_idles_exactly_n_frames_then_resumes() {
        const N: u16 = 3;
        let ecl = root_image(vec![
            OP_PUSHI as u32,
            N as u32,
            OP_WAIT as u32,
            OP_PUSHI as u32,
            7,
            OP_POPL as u32,
            0,
            OP_END as u32,
        ]);
        let mut w = World::new(1);
        let idx = spawn_test(&mut w, &ecl, 0, (OWNER_STAGE, 0, 0)).unwrap();

        step(
            &mut w,
            &crate::tables::TABLES_V0,
            &ecl,
            &InputFrame::empty(0),
        ); // 出生帧：跳过
        step(
            &mut w,
            &crate::tables::TABLES_V0,
            &ecl,
            &InputFrame::empty(1),
        ); // 首跑：执行到 WAIT(N)
        assert_eq!(
            w.tasks.slots[idx as usize].wait, N,
            "WAIT 执行帧本身即置 wait=N"
        );
        assert!(w.tasks.is_alive(idx as usize));

        for k in 0..N {
            step(
                &mut w,
                &crate::tables::TABLES_V0,
                &ecl,
                &InputFrame::empty(2 + k as u32),
            );
            assert!(
                w.tasks.is_alive(idx as usize),
                "空转第 {k} 帧仍不该恢复执行/死亡"
            );
            assert_eq!(w.tasks.slots[idx as usize].locals[0], 0, "空转帧不得执行");
        }

        // 第 N+2 帧（出生后第 N+1 个 step 调用）：wait 已递减到 0，恢复执行至 END。
        step(
            &mut w,
            &crate::tables::TABLES_V0,
            &ecl,
            &InputFrame::empty(2 + N as u32),
        );
        assert_eq!(
            w.tasks.slots[idx as usize].locals[0], 7,
            "恰 N 帧后恢复执行"
        );
        assert!(!w.tasks.is_alive(idx as usize), "恢复后跑到 END 被回收");
    }

    /// M1 T2：调度升序（I4）——两个 STAGE 任务各自 `SPAWN` 一个子任务；池分配走最低空位，
    /// 若调度确实按池索引升序执行，先注册的任务（低索引）先跑、先抢到更低的子任务槽位。
    #[test]
    fn scheduler_executes_in_ascending_pool_index_order() {
        // 父模板（script0，入口 idx0）：SPAWN script1（argc=0） → POP 丢弃句柄 → PUSHI 1000 → WAIT
        // （**故意不让父在本帧 END**——若父当帧死亡，它的槽会被同帧后续任务的 SPAWN 复用，
        // 破坏槽号与"谁先跑"的对应关系；WAIT 让父存活，子任务的槽号才干净地反映执行序）。
        // 子模板（script1，入口 idx7）：纯 END（次帧首跑门禁下本帧不会被调度到，无所谓）。
        let ecl = multi_image(
            vec![
                OP_SPAWN as u32,
                1,               // 0,1: SPAWN script1
                0,               // 2: argc=0（M1.9 T3 起 SPAWN 元数 2）
                OP_POP as u32,   // 3
                OP_PUSHI as u32, // 4
                1000,            // 5
                OP_WAIT as u32,  // 6
                OP_END as u32,   // 7: script1 入口
            ],
            &[(0, SubKind::Root, 0), (7, SubKind::Async, 0)],
        );
        let mut w = World::new(1);
        let a = spawn_test(&mut w, &ecl, 0, (OWNER_STAGE, 0, 0)).unwrap(); // 期望池索引 0
        let b = spawn_test(&mut w, &ecl, 0, (OWNER_STAGE, 0, 0)).unwrap(); // 期望池索引 1
        assert_eq!((a, b), (0, 1), "前置：两父任务确定性占据 0/1 号槽");

        step(
            &mut w,
            &crate::tables::TABLES_V0,
            &ecl,
            &InputFrame::empty(0),
        ); // 出生帧：都不跑
        step(
            &mut w,
            &crate::tables::TABLES_V0,
            &ecl,
            &InputFrame::empty(1),
        ); // 两父任务各自 SPAWN 一次

        // 若 A(0) 先跑，其子占最低空位 2；若 B(1) 先跑，其子会先占 2——用 parent 字段反查
        // 谁先谁后：A 的 parent 戳 = a+1 = 1，B 的 parent 戳 = b+1 = 2。
        let child_of_a = w
            .tasks
            .slots
            .iter()
            .enumerate()
            .find(|(i, t)| *i >= 2 && w.tasks.is_alive(*i) && t.parent == a + 1)
            .map(|(i, _)| i)
            .expect("A 的子任务应已生成");
        let child_of_b = w
            .tasks
            .slots
            .iter()
            .enumerate()
            .find(|(i, t)| *i >= 2 && w.tasks.is_alive(*i) && t.parent == b + 1)
            .map(|(i, _)| i)
            .expect("B 的子任务应已生成");
        assert!(
            child_of_a < child_of_b,
            "升序调度：A（低索引）先跑，其子应先抢到更低槽位（A={child_of_a}, B={child_of_b}）"
        );
    }

    /// M1 T2：全局预算横跨多任务升序饿死判别——`GLOBAL_BUDGET`(65536) 恰是
    /// `TASK_BUDGET`(1024) 的 64 倍：64 个自跳转死循环任务各耗尽自己的 1024 上限、
    /// 恰好吃满全局预算；第 65 个任务（canary）本帧连一条指令都不会执行——
    /// **没轮到不是它的错**：不 Fault、状态原封不动，次帧满血重跑正常完成。
    #[test]
    fn global_budget_starves_across_tasks_same_frame_then_resumes_next() {
        let ecl = multi_image(
            vec![
                OP_JMP as u32,
                0, // script 0：自跳转死循环（永不 END）
                OP_PUSHI as u32,
                9,
                OP_POPL as u32,
                0,
                OP_END as u32, // script 1（入口字 2）：canary，写 locals[0]=9 后正常结束
            ],
            &[(0, SubKind::Root, 0), (2, SubKind::Async, 0)],
        );
        let mut w = World::new(1);
        let mut looper_indices = Vec::new();
        for _ in 0..64 {
            looper_indices.push(spawn_test(&mut w, &ecl, 0, (OWNER_STAGE, 0, 0)).unwrap());
        }
        let canary = spawn_test(&mut w, &ecl, 1, (OWNER_STAGE, 0, 0)).unwrap();
        assert_eq!(canary as usize, 64, "canary 应落在第 65 号槽（升序分配）");

        step(
            &mut w,
            &crate::tables::TABLES_V0,
            &ecl,
            &InputFrame::empty(0),
        ); // 出生帧：全部跳过

        step(
            &mut w,
            &crate::tables::TABLES_V0,
            &ecl,
            &InputFrame::empty(1),
        ); // 64 个死循环各耗尽 1024，恰吃满 65536
        for &i in &looper_indices {
            assert!(
                !w.tasks.is_alive(i as usize),
                "死循环任务应在自己的 1024 上限处 Fault 被杀"
            );
        }
        assert_eq!(w.body.diag.task_faults, 64, "64 个死循环各计一次 Fault");
        assert!(
            w.tasks.is_alive(canary as usize),
            "canary 本帧没轮到，不该被杀"
        );
        assert_eq!(
            w.tasks.slots[canary as usize].locals[0], 0,
            "canary 本帧零执行（全局预算已耗尽，静默跳过不 Fault）"
        );

        step(
            &mut w,
            &crate::tables::TABLES_V0,
            &ecl,
            &InputFrame::empty(2),
        ); // 次帧：预算满血重置，死循环任务已死，canary 独享
        assert_eq!(
            w.tasks.slots[canary as usize].locals[0], 9,
            "次帧满血重跑，canary 正常执行完成"
        );
        assert!(
            !w.tasks.is_alive(canary as usize),
            "canary 执行到 END 被回收"
        );
    }

    /// M1 T2：`KILL_CHILDREN` 只杀直系子，孙辈存活（detached、不递归）——真实多帧调度链路：
    /// root spawn mid → mid spawn grandchild → root 稍后 `KILL_CHILDREN`（只碰 mid，不碰
    /// grandchild）。
    #[test]
    fn kill_children_through_real_frames_kills_only_direct_child() {
        // root: SPAWN mid(script1,入口4) ; POP(丢弃句柄) ; PUSHI 5 ; WAIT ; KILL_CHILDREN ; END
        // mid : SPAWN grandchild(script2,入口12) ; POP ; PUSHI 200 ; WAIT ; END
        // grandchild: PUSHI 200 ; WAIT ; END
        let root_code = [
            OP_SPAWN as u32,
            1,                       // 0,1: SPAWN script1(mid)
            0,                       // 2: argc=0
            OP_POP as u32,           // 3
            OP_PUSHI as u32,         // 4
            5,                       // 5
            OP_WAIT as u32,          // 6
            OP_KILL_CHILDREN as u32, // 7
            OP_END as u32,           // 8
        ];
        let mid_code_at = root_code.len() as u32; // 9
        let mid_code = [
            OP_SPAWN as u32,
            2,               // +0,+1: SPAWN script2(grandchild)
            0,               // +2: argc=0
            OP_POP as u32,   // +3
            OP_PUSHI as u32, // +4
            200,             // +5
            OP_WAIT as u32,  // +6
            OP_END as u32,   // +7
        ];
        let grandchild_code_at = mid_code_at + mid_code.len() as u32;
        let grandchild_code = [
            OP_PUSHI as u32,
            200,            // +0,+1
            OP_WAIT as u32, // +2
            OP_END as u32,  // +3
        ];

        let mut code = Vec::new();
        code.extend_from_slice(&root_code);
        code.extend_from_slice(&mid_code);
        code.extend_from_slice(&grandchild_code);

        let ecl = multi_image(
            code,
            &[
                (0, SubKind::Root, 0),
                (mid_code_at, SubKind::Async, 0),
                (grandchild_code_at, SubKind::Async, 0),
            ],
        );

        let mut w = World::new(1);
        let root = spawn_test(&mut w, &ecl, 0, (OWNER_STAGE, 0, 0)).unwrap();

        // 出生帧：跳过。
        step(
            &mut w,
            &crate::tables::TABLES_V0,
            &ecl,
            &InputFrame::empty(0),
        );
        // 帧1：root 首跑——SPAWN mid（born_frame=1）、POP、PUSHI 5、WAIT(5)。
        step(
            &mut w,
            &crate::tables::TABLES_V0,
            &ecl,
            &InputFrame::empty(1),
        );
        let mid = (0..TASK_CAP)
            .find(|&i| {
                i != root as usize && w.tasks.is_alive(i) && w.tasks.slots[i].parent == root + 1
            })
            .expect("mid 应已生成") as u16;

        // 帧2：mid 首跑（born_frame=1 != 2）——SPAWN grandchild（born_frame=2）、POP、PUSHI 200、WAIT(200)。
        // root 本帧 wait 5>0 递减，不跑。
        step(
            &mut w,
            &crate::tables::TABLES_V0,
            &ecl,
            &InputFrame::empty(2),
        );
        let grandchild = (0..TASK_CAP)
            .find(|&i| {
                i != root as usize
                    && i != mid as usize
                    && w.tasks.is_alive(i)
                    && w.tasks.slots[i].parent == mid + 1
            })
            .expect("grandchild 应已生成") as u16;
        assert!(w.tasks.is_alive(mid as usize));
        assert!(w.tasks.is_alive(grandchild as usize));

        // 再跑足够多帧，让 root 的 wait(5) 耗尽并执行 KILL_CHILDREN + END。
        // root 于帧1设 wait=5；帧2..6 各递减一次（5→4→3→2→1→0，5 次递减）；帧7 起 wait==0 恢复执行。
        for f in 3..8u32 {
            step(
                &mut w,
                &crate::tables::TABLES_V0,
                &ecl,
                &InputFrame::empty(f),
            );
        }

        assert!(!w.tasks.is_alive(root as usize), "root 执行到 END 应已回收");
        assert!(
            !w.tasks.is_alive(mid as usize),
            "mid 是 root 的直系子，应被 KILL_CHILDREN 杀"
        );
        assert!(
            w.tasks.is_alive(grandchild as usize),
            "grandchild 是 mid 的子、不是 root 的直系子，不应被杀（不递归）"
        );
    }

    /// M1 T2：`EVT_TASK_FAULT` 事件形状——`a_index` = 任务池索引，`data = [fault_code, script]`。
    #[test]
    fn task_fault_event_has_right_kind_index_and_data() {
        let ecl = root_image(vec![
            OP_PUSHI as u32,
            5,
            OP_PUSHI as u32,
            0,
            OP_DIV as u32, // 除零 → Fault(FAULT_DIV_ZERO=4)
        ]);
        let mut w = World::new(1);
        let idx = spawn_test(&mut w, &ecl, 0, (OWNER_STAGE, 0, 0)).unwrap();
        step(
            &mut w,
            &crate::tables::TABLES_V0,
            &ecl,
            &InputFrame::empty(0),
        );
        step(
            &mut w,
            &crate::tables::TABLES_V0,
            &ecl,
            &InputFrame::empty(1),
        );

        assert!(!w.tasks.is_alive(idx as usize));
        assert_eq!(w.body.diag.task_faults, 1);
        assert_eq!(w.body.events_len, 1);
        let ev = w.body.events[0];
        assert_eq!(ev.kind, crate::events::EVT_TASK_FAULT);
        assert_eq!(ev.a_index, idx);
        assert_eq!(ev.data, [4, 0], "data = [fault_code, script]");
    }

    /// 无效 raw id 无法越过 image 绑定门，因而不能传给 typed 过渡启动入口。
    #[test]
    fn spawn_task_bad_script_id_counts_contract_viol() {
        let ecl = EclImage::empty(); // subs 空
        assert_eq!(ecl.sub_id(9), None);
    }

    /// M1 T2：`spawn_task` 任务池满 → `None` + `pool_full[POOL_TASK]` 计数（P4-a）。
    #[test]
    fn spawn_task_pool_full_counts_pool_full() {
        let mut w = World::new(1);
        let ecl = root_image(vec![OP_END as u32]);
        for _ in 0..TASK_CAP {
            assert!(spawn_test(&mut w, &ecl, 0, (OWNER_STAGE, 0, 0)).is_some());
        }
        assert_eq!(spawn_test(&mut w, &ecl, 0, (OWNER_STAGE, 0, 0)), None);
        assert_eq!(w.body.diag.pool_full[POOL_TASK], 1);
        assert_eq!(w.body.last_status, STATUS_POOL_FULL);
    }

    #[test]
    fn snapshot_covers_signals() {
        let mut w = World::new(3);
        w.body.pulse_signal(5);
        let ck = w.checksum();
        let mut snap = World::new(3);
        w.copy_into(&mut snap);
        assert_eq!(snap.checksum(), ck, "signals 随快照且入校验和");
        snap.body.signals[5] ^= 1;
        assert_ne!(
            snap.checksum(),
            ck,
            "signals 必须真的参与校验和（防未来误加 skip）"
        );
    }

    /// 掉落散布消耗世界 RNG：同种子两跑逐位一致；不同颗速度不同（散布真的在动）。
    #[test]
    fn drop_item_scatter_deterministic() {
        let run = || {
            let mut w = World::new(11);
            let a = w.body.drop_item(
                Fx::ZERO,
                Fx::from_int(100),
                crate::items::ITEM_POWER,
                &crate::tables::TABLES_V0,
            );
            let b = w.body.drop_item(
                Fx::ZERO,
                Fx::from_int(100),
                crate::items::ITEM_POWER,
                &crate::tables::TABLES_V0,
            );
            let (ia, ib) = (w.body.items.get(a).unwrap(), w.body.items.get(b).unwrap());
            (
                w.body.items.vx[ia],
                w.body.items.vy[ia],
                w.body.items.vx[ib],
                w.body.items.vy[ib],
            )
        };
        let (ax1, ay1, bx1, by1) = run();
        let (ax2, ay2, bx2, by2) = run();
        assert_eq!((ax1, ay1, bx1, by1), (ax2, ay2, bx2, by2), "同种子逐位一致");
        assert!(ay1.raw() < 0, "弹出初速向上");
        assert!((ax1, ay1) != (bx1, by1), "两颗散布不同（RNG 真的被消耗）");
    }

    /// P4：坏类型 → NULL + BAD_ARGS；池满 → NULL + pool_full[ITEM]（B1 道具池份额）。
    #[test]
    fn drop_item_bad_type_and_pool_full() {
        let mut w = World::new(1);
        assert_eq!(
            w.body
                .drop_item(Fx::ZERO, Fx::ZERO, 9, &crate::tables::TABLES_V0),
            crate::items::ItemHandle::NULL
        );
        assert_eq!(w.body.last_status, crate::world::STATUS_BAD_ARGS);
        for _ in 0..crate::items::ItemPool::CAP {
            w.body.spawn_drop(
                Fx::ZERO,
                Fx::from_int(50),
                crate::items::ITEM_POINT,
                &crate::tables::TABLES_V0,
            );
        }
        assert_eq!(
            w.body.drop_item(
                Fx::ZERO,
                Fx::ZERO,
                crate::items::ITEM_POINT,
                &crate::tables::TABLES_V0
            ),
            crate::items::ItemHandle::NULL
        );
        assert_eq!(w.body.diag.pool_full[crate::world::POOL_ITEM], 1);
    }

    /// globals 往返 + 坏槽 P4-b + 入校验和 + 快照往返（copy_into 漏拷即红）。
    #[test]
    fn globals_set_get_badslot_checksum_snapshot() {
        let mut w = World::new(1);
        let c0 = w.checksum();
        w.body.set_var(0, 42);
        w.body.set_var(1023, -7);
        assert_eq!(w.body.get_var(0), 42);
        assert_eq!(w.body.get_var(1023), -7);
        assert_ne!(w.checksum(), c0, "globals 必须入校验和");
        let cv0 = w.body.diag.contract_viol;
        w.body.set_var(1024, 1); // 坏槽
        assert_eq!(w.body.get_var(1024), 0); // 坏槽读 0
        assert_eq!(w.body.diag.contract_viol, cv0 + 2, "set/get 各计一次");
        assert_eq!(w.body.last_status, crate::world::STATUS_BAD_ARGS);
        assert_eq!(w.body.globals[1023], -7, "坏槽写不得触碰任何真槽");
        // 快照往返：copy_into 漏拷 globals 即红
        let mut snap = World::new(2);
        w.copy_into(&mut snap);
        assert_eq!(snap.body.get_var(0), 42);
        w.body.set_var(0, 99);
        snap.copy_into(&mut w);
        assert_eq!(w.body.get_var(0), 42, "restore 必须还原 globals");
    }

    /// boss_ui 整槽写读 + 坏槽 P4-b + 入校验和 + 快照往返（copy_into 漏拷即红）。
    #[test]
    fn boss_set_roundtrip_badslot_checksum_snapshot() {
        use crate::boss::{BossUiSlot, MAX_BOSSES};
        let mut w = World::new(1);
        let c0 = w.checksum();
        let ui = BossUiSlot {
            enemy: crate::enemy::EnemyHandle::NULL,
            hp_ratio: Fx::from_raw(32768),
            spell_id: 7,
            timer_frames: 3600,
            phase_left: 2,
            active: 1,
        };
        w.body.boss_set(0, ui);
        assert_eq!(w.body.boss_ui[0], ui, "整槽写入可整槽读回");
        assert_ne!(w.checksum(), c0, "boss_ui 必须入校验和");
        let cv0 = w.body.diag.contract_viol;
        w.body.boss_set(MAX_BOSSES as u8, ui); // 坏槽
        assert_eq!(w.body.diag.contract_viol, cv0 + 1);
        assert_eq!(w.body.last_status, crate::world::STATUS_BAD_ARGS);
        assert_eq!(w.body.boss_ui[1].active, 0, "坏槽写不得溢到别槽");
        // 快照往返：copy_into 漏拷 boss_ui 即红
        let mut snap = World::new(2);
        w.copy_into(&mut snap);
        assert_eq!(snap.body.boss_ui[0], ui, "快照必须带 boss_ui");
        w.body.boss_set(0, BossUiSlot::default());
        snap.copy_into(&mut w);
        assert_eq!(w.body.boss_ui[0], ui, "restore 必须还原 boss_ui");
    }

    // ── 快照防漏(2026-07-23 审阅 §1):此前无判别式拷贝覆盖的字段,逐一 mutate 非默认值
    //    → copy_into → 命中;从 copy_into 删对应行即红。 ────────────────────────────

    #[test]
    fn snapshot_covers_frame() {
        let mut w = World::new(3);
        w.body.frame = 777;
        let mut snap = World::new(3);
        w.copy_into(&mut snap);
        assert_eq!(snap.body.frame, 777, "copy_into 漏拷 frame 即红");
    }

    #[test]
    fn snapshot_covers_rng_state() {
        let mut w = World::new(3);
        let _ = w.body.rng.next_u32(); // 状态偏离种子初值
        let ck = w.checksum();
        let mut snap = World::new(3);
        w.copy_into(&mut snap);
        assert_eq!(snap.checksum(), ck, "rng 状态必须随快照(I3;漏拷即红)");
    }

    #[test]
    fn snapshot_covers_players() {
        let mut w = World::new(3);
        w.body.set_player_power(0, 123);
        let mut snap = World::new(3);
        w.copy_into(&mut snap);
        assert_eq!(snap.body.players()[0].power, 123, "players 数组漏拷即红");
    }

    /// ABA 修复（复审 Task 2）判别腿：`spells[].epoch`（非零，一次真 begin 产出）与
    /// `spell_seq`（持久代际计数器）都必须随 `copy_into` 快照往返——这正是 Critical 修复
    /// 依赖的地基：若快照恢复后 epoch 语义丢失，rollback 场景下"槽复用 ABA"防线会重新破防。
    #[test]
    fn snapshot_covers_spells_and_spell_seq() {
        let mut w = World::new(3);
        let boss = crate::world::test_support::spawn_enemy(&mut w, 0, 80, 1000);
        assert!(w.body.spell_begin_internal(0, boss, 5, 100, 1000, 0, 0));
        let epoch0 = w.body.spells[0].epoch;
        assert_ne!(epoch0, 0, "前置：真 begin 应已铸出非零 epoch");

        let mut snap = World::new(3);
        w.copy_into(&mut snap);
        assert_eq!(snap.body.spells[0].epoch, epoch0, "spells[].epoch 漏拷即红");
        assert_eq!(
            snap.body.spell_seq[0], w.body.spell_seq[0],
            "spell_seq 持久计数器漏拷即红"
        );
        assert_eq!(snap.checksum(), w.checksum(), "两者校验和应完全一致");

        // 结算清槽后再 begin 一次（同槽复用），代际应继续往前走且同样能快照往返。
        w.body.spell_end_by_owner(boss);
        assert!(w.body.spell_begin_internal(0, boss, 6, 100, 1000, 0, 0));
        let epoch1 = w.body.spells[0].epoch;
        assert_ne!(epoch1, epoch0, "同槽复用应换代");
        let mut snap2 = World::new(3);
        w.copy_into(&mut snap2);
        assert_eq!(
            snap2.body.spells[0].epoch, epoch1,
            "复用后的新 epoch 同样漏拷即红"
        );
        assert_eq!(snap2.checksum(), w.checksum());
    }

    #[test]
    fn snapshot_covers_diag_and_last_status() {
        let mut w = World::new(3);
        w.body.set_player_power(9, 0); // OOB → contract_viol+1 + BAD_ARGS
        assert!(w.body.diag.contract_viol > 0, "前置:计数已非零");
        let mut snap = World::new(3);
        w.copy_into(&mut snap);
        assert_eq!(snap.body.diag.contract_viol, w.body.diag.contract_viol);
        assert_eq!(snap.body.last_status, w.body.last_status);
    }

    #[test]
    fn snapshot_covers_ecl_main_started_and_tables_hash() {
        let mut w = World::new(3);
        w.ecl_main_started = 1;
        w.tables_hash = 0xDEAD_BEEF;
        let mut snap = World::new(3);
        w.copy_into(&mut snap);
        assert_eq!(snap.ecl_main_started, 1, "ecl_main_started 漏拷即红");
        assert_eq!(snap.tables_hash, 0xDEAD_BEEF, "tables_hash 漏拷即红");
    }

    // ── 表现锚点四字段判别式（整局流程刀 Task 2；spec §10-8 拍板形态）──────────────

    /// 判别式：锚字段必须"仅 builtin 写、仅表现读"——任何相位一旦读了它们参与决策，
    /// 两个仅锚字段不同的 world 就会在别处（rng/players/bullets…）分叉。手法：每帧把
    /// wa 的四锚字段临时抄给 wb 后比对整体校验和（含锚字段自身，此刻应相等），再把
    /// wb 的抄回 0（否则下一帧 wa 的"新"锚值会被 wb 的陈旧值错误掩盖）。红 = 有相位
    /// 读了锚字段。
    #[test]
    fn anchor_fields_are_write_only_for_simulation() {
        let mut wa = World::new(1);
        let mut wb = World::new(1);
        wa.body.bgm_id = 7;
        wa.body.bg_id = 8;
        wa.body.bg_phase = 9;
        wa.body.bg_phase_frame = 100;
        for f in 0..120u32 {
            let input = InputFrame::empty(f);
            crate::world::test_support::step_t(&mut wa, &input);
            crate::world::test_support::step_t(&mut wb, &input);
            wb.body.bgm_id = wa.body.bgm_id;
            wb.body.bg_id = wa.body.bg_id;
            wb.body.bg_phase = wa.body.bg_phase;
            wb.body.bg_phase_frame = wa.body.bg_phase_frame;
            assert_eq!(
                wa.checksum(),
                wb.checksum(),
                "frame {f}：抄平锚字段后仍分叉 ⇒ 某相位读了锚字段"
            );
            wb.body.bgm_id = 0;
            wb.body.bg_id = 0;
            wb.body.bg_phase = 0;
            wb.body.bg_phase_frame = 0;
        }
    }

    /// 新 World 四锚字段全 0；无脚本跑 60 帧后仍全 0（无相位写它们，金向量不会因本刀
    /// 平移——本任务实际会因字段本身入校验和而平移，本测试只钉"没有相位额外写它们"）。
    #[test]
    fn anchor_fields_default_zero_keeps_golden_quiet() {
        let mut w = World::new(1);
        for f in 0..60u32 {
            crate::world::test_support::step_t(&mut w, &InputFrame::empty(f));
        }
        assert_eq!(w.body.bgm_id, 0);
        assert_eq!(w.body.bg_id, 0);
        assert_eq!(w.body.bg_phase, 0);
        assert_eq!(w.body.bg_phase_frame, 0);
    }

    /// 快照防漏哨兵(checksum-mechanism.md 承诺的二线防护,实现形态=本测试):
    /// **本断言红了 ⇒ 你增/删/改了 World 字段** ⇒ 依次核对
    /// ① `copy_into` 逐字段清单(手写,漏拷编译不报错——这正是本哨兵存在的原因)
    /// ② checksum(新字段默认入;skip 须理由) ③ D10 容量预算
    /// ④ SaveBytes 存档(derive 类型自动;**手写 impl 如 XformSegPool 须两侧同步**——
    ///    这是唯一 checksum 与深等价测试都盲的缝,全靠本哨兵逼人来读此清单),
    /// 然后才允许更新下方数字。
    /// `phase_guard` 仅 debug 存在 ⇒ 双值。
    #[test]
    fn world_size_sentinel_guards_copy_into_field_list() {
        let sizes = (
            core::mem::size_of::<crate::world::WorldBody>(),
            core::mem::size_of::<World>(),
        );
        // 2026-07-25（整局流程刀 Task 2）：`WorldBody` 新增表现锚点四字段
        // `bgm_id/bg_id/bg_phase:u16`×3 + `bg_phase_frame:u32`（逻辑 10B，紧邻 `reqs`
        // 数组前，对齐吸收后 WorldBody 实测 +16：969360→969376；`World` 同步 +16：
        // 1084104→1084120（无新池/无 Task 字段改动，增量 1:1 对应，非新增分摊乘数）。
        // 不是新池（③ D10 容量预算不适用——D10 只管池容量常数，本刀四字段是标量）；
        // ② checksum 走 derive 默认全量入（未加 skip）；④ SaveBytes 走 derive 自动
        // （WorldBody 用 `#[derive(... SaveBytes)]`，非手写 impl，无需两侧同步）。
        // 以下两值均为 `cargo test -p stg-core world_size_sentinel` 实测输出，非手算。
        #[cfg(debug_assertions)]
        const EXPECTED: (usize, usize) = (969376, 1084120);
        #[cfg(not(debug_assertions))]
        const EXPECTED: (usize, usize) = (969376, 1084120);
        assert_eq!(sizes, EXPECTED, "先按测试文档注释核对三件套,再更新哨兵数字");
    }

    /// D6 封口后的三只读口(2026-07-23 审阅 §2):帧号推进可见 / events 切片界=len /
    /// tasks 只读借用。
    #[test]
    fn frame_and_frame_events_and_tasks_read_accessors() {
        let mut w = World::new(1);
        assert_eq!(w.frame(), 0);
        crate::world::test_support::step_t(&mut w, &crate::input::InputFrame::empty(0));
        assert_eq!(w.frame(), 1, "advance 后帧号经读口可见");
        assert_eq!(w.tasks().iter_alive().count(), 0, "&TaskPool 只读口");
        assert!(w.frame_events().is_empty());
        w.body.push_event(crate::events::Event {
            kind: crate::events::EVT_PLAYER_DIED,
            ..Default::default()
        });
        assert_eq!(
            w.frame_events().len(),
            1,
            "切片界 = events_len(不吐陈旧尾槽)"
        );
    }

    #[test]
    fn engine_ver_anchored() {
        assert_eq!(
            crate::ENGINE_VER,
            1,
            "bump 必须是有意识决定(评审 + 改本测试)"
        );
    }

    // ── 刀 2/3：save_bytes/load_bytes 判别测试 ─────────────────────────────

    /// 判别测试脚手架("表绑定 + 有任务"的最小世界)：`stg_ecl_compiler` 依赖方向不可达
    /// （P1：core 不能依赖上层编译器），故用 `EclImage::try_from_parts` 手拼一枚
    /// root-only 小镜像，`content_hash` 对齐 `TABLES_V0.content_hash` 使 `start_main`
    /// 的 coherence 守卫放行（真实非零哈希，不是"任一侧 0"逃逸路径）。根任务
    /// `push 5 → wait 5 → jmp 0` 死循环，240 帧后仍存活、`pc`/`wait` 非零；外加一颗
    /// "boss"敌人 + 一颗弹，令弹/敌/任务/rng 全非默认值——深等价测试
    /// （save→load→checksum 相等）的判别力全靠这个"非平凡"世界撑着（全零世界的往返
    /// 测试对漏撒 derive 是瞎的）。
    fn rainbow_for_test() -> (Box<World>, EclImage, crate::enemy::EnemyHandle) {
        use crate::ecl::image::ImageParts;

        let image = EclImage::try_from_parts(ImageParts {
            code: vec![OP_PUSHI as u32, 5, OP_WAIT as u32, OP_JMP as u32, 0],
            subs: vec![SubInit::new(0, SubKind::Root, vec![])],
            entries: vec![],
            root: Some(0),
            marks: vec![],
            content_hash: crate::tables::TABLES_V0.content_hash,
        })
        .expect("root-only 镜像必须满足运行期镜像契约");

        let mut w = World::new(0xC0FF_EE42_1357_9BDF);
        w.start_main(&image).expect("coherence 应放行(哈希对齐)");

        let boss = w.body.create_enemy(crate::enemy::EnemyInit {
            x: Fx::from_int(30),
            y: Fx::from_int(60),
            vx: Fx::from_int(1),
            vy: Fx::from_int(-1),
            mv_from_x: Fx::ZERO,
            mv_from_y: Fx::ZERO,
            mv_to_x: Fx::ZERO,
            mv_to_y: Fx::ZERO,
            mv_t: 0,
            mv_dur: 0,
            mv_easing: 0,
            mv_active: 0,
            hp: 500,
            hp_max: 500,
            radius: Fx::from_int(12),
            hurtbox: Fx::from_int(16),
            invuln: 0,
            hit_flash: 0,
            flags: 0,
            sprite: 3,
            anm_state: 0,
            main_task: 0,
            death_script: 0,
            drop_table: 0,
            score: 500,
        });
        assert_ne!(boss, crate::enemy::EnemyHandle::NULL, "boss 敌人必须建成");

        w.body.create_bullet(straight(10, 100, 1, 2, 600));
        // 带 xform 段的第二颗弹——让 `XformSegPool`(手写 SaveBytes 镜像)也捎带非零内容,
        // 深等价测试才对"手写字段序错位"这一类 bug 有判别力(全零段的往返对此维度是瞎的)。
        w.body.create_bullet_with_xform(
            straight(-10, 120, 0, 1, 600),
            &[slot(
                3,
                crate::xform::OP_SET_SPEED,
                Fx::from_int(2).raw(),
                0,
            )],
        );

        (w, image, boss)
    }

    /// 深等价:跑一段有弹/敌/任务/道具的世界 → save → load → checksum 相等
    /// (校验和即全字段深比较,P6 白拿);外加二次 save 字节全等(规范自洽)。
    #[test]
    fn save_load_roundtrip_deep_equal_by_checksum() {
        let (mut w, image, _boss) = rainbow_for_test();
        for f in 0..240u32 {
            crate::step(
                &mut w,
                &crate::tables::TABLES_V0,
                &image,
                &InputFrame::empty(f),
            );
        }
        let bytes = w.save_bytes(&image);
        let w2 = World::load_bytes(&bytes, &crate::tables::TABLES_V0, &image).unwrap();
        assert_eq!(w2.checksum(), w.checksum(), "载入 == 从未离开");
        assert_eq!(w2.save_bytes(&image), bytes, "save→load→save 字节全等");
    }

    /// skip 字段不入档:save 前预污染源世界的三条输出缓冲 → load 后全空。
    #[test]
    fn save_omits_pure_output_buffers() {
        let (mut w, image, _boss) = rainbow_for_test();
        w.body.emit_req(7, [1; 6]);
        w.body.push_event(crate::events::Event {
            kind: 1,
            ..Default::default()
        });
        let bytes = w.save_bytes(&image);
        let w2 = World::load_bytes(&bytes, &crate::tables::TABLES_V0, &image).unwrap();
        assert!(w2.take_requests().is_empty(), "reqs 不入档");
        assert!(w2.frame_events().is_empty(), "events 不入档");
    }

    /// 头长钉死(复审 Task 2 修):`SAVE_HEADER_LEN` 必须等于 `save_bytes` 实际写出的
    /// 头字节数,不能是分解和算错的常量(曾误为 45,真实/求和皆 49)——总字节数减去
    /// 载荷长(从头里 len 字段读回,而非从常量反推)必须精确等于它,漂移不可能蒙混过关。
    #[test]
    fn header_len_matches_actual_wire() {
        let (w, image, _boss) = rainbow_for_test();
        let bytes = w.save_bytes(&image);
        // len 字段紧随 magic4+ver1+engine_ver4+tables_hash8+image_hash8+seed8+frame4,
        // 即偏移 37..41(与上面 corruption 测试里 b[9]/b[17] 等硬编码偏移同一套写出序)。
        let plen = u32::from_le_bytes(bytes[37..41].try_into().unwrap()) as usize;
        assert_eq!(
            bytes.len() - plen,
            crate::save::SAVE_HEADER_LEN,
            "头长必须等于 save_bytes 实际写出的头字节数"
        );
    }

    /// 头/载荷错误路径逐一判别(八条各得其 LoadError 变体)。
    #[test]
    fn load_rejects_each_corruption_distinctly() {
        use crate::save::LoadError;
        let (w, image, _boss) = rainbow_for_test();
        let good = w.save_bytes(&image);
        let t = &crate::tables::TABLES_V0;
        let mut b;
        b = good.clone();
        b[0] ^= 0xFF;
        assert_eq!(
            World::load_bytes(&b, t, &image).unwrap_err(),
            LoadError::BadMagic
        );
        b = good.clone();
        b[4] = 99;
        assert_eq!(
            World::load_bytes(&b, t, &image).unwrap_err(),
            LoadError::BadFileVer { got: 99 }
        );
        b = good.clone();
        b[5] ^= 0xFF; // ENGINE_VER 首字节
        assert!(matches!(
            World::load_bytes(&b, t, &image).unwrap_err(),
            LoadError::EngineVerMismatch { .. }
        ));
        b = good.clone();
        let last = b.len() - 1;
        b[last] ^= 0x01; // 载荷尾翻一位
        assert!(matches!(
            World::load_bytes(&b, t, &image).unwrap_err(),
            LoadError::HashMismatch { .. }
        ));
        b = good.clone();
        b.truncate(good.len() - 8);
        assert_eq!(
            World::load_bytes(&b, t, &image).unwrap_err(),
            LoadError::Truncated
        );
        b = good.clone();
        b.push(0);
        assert!(matches!(
            World::load_bytes(&b, t, &image).unwrap_err(),
            LoadError::TrailingBytes { .. }
        ));
        b = good.clone();
        b[9] ^= 0xFF; // tables_hash 首字节
        b[10] ^= 0xFF; // 多翻一字节，防"首字节巧合同值"假绿(brief 注记)
        assert!(matches!(
            World::load_bytes(&b, t, &image).unwrap_err(),
            LoadError::TablesMismatch { .. }
        ));
        b = good.clone();
        b[17] ^= 0xFF; // image_hash 首字节
        b[18] ^= 0xFF; // 多翻一字节，同上理由
        assert!(matches!(
            World::load_bytes(&b, t, &image).unwrap_err(),
            LoadError::ImageMismatch { .. }
        ));
    }

    // ── Task 6：Loadout + new_game_at(核心开机面)───────────────────────────

    /// 委托改造零行为差(整局流程刀 spec §2.2):`new_game` 与
    /// `new_game_at(.., 0, Loadout::default(), ..)` 同 seed/rank/image → 逐位一致
    /// （校验和相等,P6 白拿的全字段深比较）。
    #[test]
    fn new_game_delegates_bitwise_to_default_path() {
        let image = root_image(vec![OP_END as u32]);
        let wa = World::new_game(7, 2, &image).expect("new_game");
        let wb = World::new_game_at(7, 2, 0, crate::player::Loadout::default(), &image)
            .expect("new_game_at 默认路径");
        assert_eq!(wa.checksum(), wb.checksum(), "委托改造零行为差");
    }

    /// B17 销账(2026-07-25 文档整理):`new_game` spec 四断言的最后两项——①开局帧号
    /// 恒 0(堆零构造隐式保证,此处显式钉住);②`TaskStartError` 经委托链原样透传
    /// (空镜像无 root → `NoRoot`,`new_game` 不吞不换)。
    #[test]
    fn new_game_frame_zero_and_error_passthrough() {
        let image = root_image(vec![OP_END as u32]);
        let w = World::new_game(7, 2, &image).expect("new_game");
        assert_eq!(w.frame(), 0, "开局帧号 0");
        assert_eq!(
            World::new_game(7, 2, &crate::ecl::image::EclImage::empty()).unwrap_err(),
            crate::ecl::binding::TaskStartError::NoRoot,
            "start_main 错误经 new_game 委托链原样透传"
        );
    }

    /// 装备钳位:power 越 `POWER_MAX` 钳、lives/bombs 全域直收(u8 无上限常量);
    /// score/graze 仍出场默认 0(装备面不碰这两个字段)。
    #[test]
    fn new_game_at_applies_loadout_with_clamp() {
        let image = root_image(vec![OP_END as u32]);
        let loadout = crate::player::Loadout {
            character: 0,
            power: 9999,
            lives: 8,
            bombs: 1,
        };
        let w = World::new_game_at(7, 2, 0, loadout, &image).expect("new_game_at");
        let p = &w.body.players[0];
        assert_eq!(p.power, crate::items::POWER_MAX, "power 钳到 POWER_MAX");
        assert_eq!(p.lives, 8, "lives 全域直收");
        assert_eq!(p.bombs, 1, "bombs 全域直收");
        assert_eq!(p.score, 0);
        assert_eq!(p.graze, 0);
    }

    /// 两条宿主期响亮错(P4-a):`start` 无此标记 → `UnknownMark`;`character` 越
    /// `tables.characters.len()` → `InvalidCharacter`。两者必须发生在任何任务落池
    /// 之前——本测试只断言返回值,不去戳"世界半初始化"(设计上 `Err` 分支下调用方
    /// 手里的 `Box<World>` 半成品被直接丢弃,无从观测)。
    #[test]
    fn new_game_at_unknown_mark_and_bad_character_fail_loud() {
        use crate::ecl::binding::TaskStartError;
        let image = root_image(vec![OP_END as u32]);

        let err = World::new_game_at(7, 2, 42, crate::player::Loadout::default(), &image)
            .expect_err("start=42 无此标记");
        assert_eq!(err, TaskStartError::UnknownMark(42));

        let bad_character = crate::player::Loadout {
            character: 9,
            ..crate::player::Loadout::default()
        };
        let err = World::new_game_at(7, 2, 0, bad_character, &image)
            .expect_err("character=9 越 TABLES_V0.characters.len()==1");
        assert_eq!(err, TaskStartError::InvalidCharacter(9));
    }

    /// 中段启动:根任务(tasks 池 0 号,首次分配必落最低空位——I4)pc 直接搁到标记
    /// 落点 L,而非 `code_entry` E。手工镜像(P1:core 不依赖编译器)——`marks: [(7, 2)]`,
    /// E=0(root sub 入口),L=2(< code.len()==3,满足 `try_from_parts` 落点边界契约)。
    #[test]
    fn new_game_at_moves_root_pc_to_mark_landing() {
        use crate::ecl::image::ImageParts;
        let image = EclImage::try_from_parts(ImageParts {
            code: vec![OP_END as u32, OP_END as u32, OP_END as u32],
            subs: vec![SubInit::new(0, SubKind::Root, vec![])],
            entries: vec![],
            root: Some(0),
            marks: vec![(7, 2)],
            content_hash: 0,
        })
        .expect("root+marks 镜像必须满足运行期镜像契约");
        let w = World::new_game_at(7, 2, 7, crate::player::Loadout::default(), &image)
            .expect("new_game_at start=7");
        assert_eq!(
            w.tasks.slots[0].pc, 2,
            "根任务(tasks 池 0 号)pc 应搁到标记落点 L=2,而非 code_entry E=0"
        );
    }
}
