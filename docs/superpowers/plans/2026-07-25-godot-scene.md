# Godot 场景刀 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 真 Godot 工程竖切——A5 乙案解锁纯 .ecl boss,四层 MultiMesh + 分发器 + HUD,demo 局(杂兵段+风铃卡)在 Godot 里可玩可打,双 headless 冒烟守链路。

**Architecture:** spec `docs/superpowers/specs/2026-07-25-godot-scene-design.md`(已批准)。壳全在 GDScript(桥面冻结零新增);断层线下唯一改动 = A5 乙案(T1);场景树代码构建(编辑器化留给真美术期);渲染契约收口成 `docs/render-contract.md`。

**Tech Stack:** Rust(stg-core/stg-ecl-compiler,断层线纪律)+ gdext 0.5.4 cdylib(已有,不动)+ Godot 4.6.3 GDScript + .ecl 表层语言。

## Global Constraints

- **断层线**:T1 是唯一 Rust 任务;stg-core 内无浮点/时钟/宿主 RNG/无序容器;TDD(先红后绿)。
- **桥面冻结**:`crates/stg-godot/src/bridge.rs` 本刀**零新增** `#[func]`/`#[constant]`(spec §2)。GDScript 侧需要的池容量/REQ id 一律本地常量镜像 + 注释指权威源。
- **金向量判据(T1)**:金向量两场景(诊断/rainbow.ecl)**都不调 `spawn_enemy`**,故 golden 输出应**逐位不变(空 diff)**;非空 = 停手排查,不许"顺手承认平移"。
- **池容量镜像**(GDScript 硬编码值,源 `define_pool!` 声明):bullets **8192** / shots **1024** / enemies **256** / items **512**;`register_layer` 返 false 必须 `push_error` 且冒烟失败(漂移即炸)。
- **实例缓冲 stride 冻结**:12 float/实例 `[cos,-sin,0,x, sin,cos,0,y, sprite,0,0,0]`;`INSTANCE_CUSTOM.y/z/w` 保留空位。
- **坐标映射**:SubViewport 384×448 挂在容器 (32,16);世界内容根 Node2D 摆 (192,0),世界坐标即本地坐标(x∈[-192,192], y∈[0,448],1px=1unit)。
- **Godot 宿主**:`GODOT_BIN=/data/sunyunbo/playground/godot/Godot_v4.6.3-stable_linux.x86_64`;`--import` 首跑非致命(`|| true`,坑档 G1);`.uid` sidecar 随源入库(G2);gdext 0.5.4 用 `VarDictionary`/AsArg 按引用(G3/G4)。
- **冒烟脚本纪律**:不许 `set -e` + 命令替换吞诊断(follow-ups B22);断言字段先灌互异非零判别值(S1)。
- **demo 参数**:seed=1, rank=2, loadout=(character 0, power 0, lives 3, bombs 3);REQ id 1..7 值源 `crates/stg-core/src/reqs.rs`。
- **commit 结尾**:`Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`。

---

### Task 1: A5 乙案——`spawn_enemy` 长 sprite+task 参 + `enemy_hp` 读口

> **spec 补遗(执行前已向用户点名)**:①内建表无任何"查敌"读口,stage 主任务无法等 boss 死、
> spec §8 的"脚本 emit STAGE_CLEAR"写不出来 → 追 `enemy_hp(handle)`;②`sys_spawn_enemy`
> sprite 恒 0,杂兵/boss 无法区分外观 → 签名一并放开(append-only,一次迁移)。两者都属
> A5"语言面缺口"本义。

**Files:**
- Modify: `crates/stg-ecl-compiler/src/lang/builtins.rs`(spawn_enemy 条目 + 新 enemy_hp 条目)
- Modify: `crates/stg-core/src/ecl/syscall.rs`(`SYS_ENEMY_HP=12` 常量 + dispatch + `sys_spawn_enemy` 重写 + `sys_enemy_hp` + 测试)
- Modify(机械迁移): `crates/stg-godot/smoke/godot_smoke.ecl`、`crates/stg-godot/src/frame.rs`(测试 SRC)、其余 `grep -rn "spawn_enemy(" --include="*.ecl" --include="*.rs" --include="*.md"` 命中的调用点/文档例子
- Regenerate: `cargo run -p stg-harness -- gen-ecl-meta`(ecl-meta.json / VS Code 扩展数据 / ecl-lang.md 生成段)

**Interfaces:**
- Produces(T6 依赖): `.ecl` 侧 `spawn_enemy(x:fx, y:fx, hp:int, drop_table:int, score:int, sprite:int, task) -> int`(task = async 无参 sub 名或 `none`);`enemy_hp(handle:int) -> int`(活敌返当前 hp,死/悬垂/越界返 -1,P4-b;句柄是池 index,复用不可辨——demo 惯例:boss 段后不再造敌)。
- Produces: 敌死 → 其 task 被 owner-liveness gate 下相位杀(既有机制,零改动);`EnemyPool.main_task = 任务槽号+1`(0=无,纯记账)。

- [ ] **Step 1: 基线留证**

```bash
mkdir -p .superpowers/godot-scene
cargo run -p stg-harness -- golden --out .superpowers/godot-scene/golden-base.txt
```

- [ ] **Step 2: 写失败测试(compiler 侧)**——`crates/stg-ecl-compiler/src/lang/codegen.rs` 测试区(仿既有 `spell_begin` e2e 测试形态,该文件 ~1225 行处有同款先例):

```rust
#[test]
fn spawn_enemy_task_param_lowers_like_fire() {
    // 7 参:sprite 位求值参,task 位 SubRef(标识符/none 编译期解析,同 fire 第 7 参通道)
    let src = r#"
async sub boss_main() { loop { wait(60); } }
sub main() {
    _ = spawn_enemy(0.0fx, 96.0fx, 100, 1, 500, 3, boss_main);
    _ = spawn_enemy(1.0fx, 2.0fx, 10, 0, 0, 0, none);
}
"#;
    let image = crate::lang::compile(src).expect("7 参 spawn_enemy 应编译通过");
    let _ = image; // 语义断言在 core 侧 e2e;此处钉编译面:标识符/none 两形态都收
}

#[test]
fn spawn_enemy_task_param_rejects_value_expr() {
    // Sub 位不收求值表达式(同 fire:标识符或 none)
    let src = r#"
sub main() { _ = spawn_enemy(0.0fx, 0.0fx, 1, 0, 0, 0, 1 + 2); }
"#;
    assert!(crate::lang::compile(src).is_err());
}
```

- [ ] **Step 3: 跑测试确认红**

```bash
cargo test -p stg-ecl-compiler spawn_enemy_task -- --nocapture
```
Expected: FAIL(现签名 5 参,7 参报参数数量/类型错)。

- [ ] **Step 4: 改 builtins.rs 两条目**

`spawn_enemy` 条目(append-only:旧 5 参前缀不动,尾追 sprite、task):

```rust
Builtin {
    name: "spawn_enemy",
    syscall: syscall::SYS_SPAWN_ENEMY,
    is_op: false,
    params: &[Val(Fx), Val(Fx), Val(Int), Val(Int), Val(Int), Val(Int), Sub],
    ret: Some(Int),
    doc: "造敌;判定 12/16 默认;task 为敌主任务 async sub 名或 none(owner=新敌,敌死任务亡);返敌句柄,失败 -1",
    param_names: &["x", "y", "hp", "drop_table", "score", "sprite", "task"],
},
```

紧随其后新增 `enemy_hp`(读族,放 `global` 附近的读口区亦可,表序即源码序):

```rust
Builtin {
    name: "enemy_hp",
    syscall: syscall::SYS_ENEMY_HP,
    is_op: false,
    params: &[Val(Int)],
    ret: Some(Int),
    doc: "查敌当前 hp;死/悬垂/越界句柄返 -1(P4-b;句柄是池 index,槽复用不可辨)——stage 编排等 boss 死用",
    param_names: &["handle"],
},
```

并把 `lookup_finds_every_documented_builtin_by_name` 测试的名字数组补 `"enemy_hp"`。

- [ ] **Step 5: 写失败测试(core 侧)**——`crates/stg-core/src/ecl/syscall.rs` 测试区(用既有 `call`/`test_support` 助手,形态仿 `sys_spawn_enemy_creates_with_fields`;**机械调整许可**:助手签名以文件现状为准):

```rust
#[test]
fn spawn_enemy_with_task_binds_owner_and_main_task() {
    // args 正序 x,y,hp,drop,score,sprite,task_script(压栈序);task_script 用一个在册 async 无参 sub 号
    // 期望:敌建成 + 任务 spawn,owner=(OWNER_ENEMY, 敌 idx, 敌 gen),main_task = 槽号+1
    // 判别值:sprite 传 5(非 0),断言 enemies.sprite[idx]==5(S1:非默认判别)
}

#[test]
fn spawn_enemy_task_none_leaves_main_task_zero() {
    // task_script = -1:敌建成、无任务、main_task==0
}

#[test]
fn spawn_enemy_bad_task_script_faults_without_enemy() {
    // task_script = 9999(不在册):Err(FAULT_BAD_OP) 且敌池活数不变(先验后建,镜像 sys_fire)
}

#[test]
fn spawn_enemy_task_pool_full_degrades() {
    // 先灌满任务池(循环 tasks.spawn),再 spawn_enemy 带合法 task:
    // 敌建成、diag.pool_full[POOL_TASK] 计数 +1、main_task==0(P4-a)
}

#[test]
fn enemy_owned_task_dies_with_enemy() {
    // e2e:.ecl 源 "async sub t(){loop{wait(1);}} sub main(){_=spawn_enemy(...,t);}"
    // 正典 boot 跑两帧(出生当帧不跑门禁)→ 断言任务活;free 该敌 → 再 step 一帧 →
    // 断言该任务被 owner-liveness gate 收走(活任务数判别)
}

#[test]
fn enemy_hp_reads_alive_and_rejects_dead() {
    // 造敌 hp=77 → call SYS_ENEMY_HP(handle) 返 77(判别值,非默认);
    // free 敌 → 返 -1;handle=9999 → 返 -1(P4-b 不 Fault)
}
```

- [ ] **Step 6: 跑测试确认红**

```bash
cargo test -p stg-core enemy_hp spawn_enemy -- --nocapture
```
Expected: FAIL(SYS_ENEMY_HP 未定义/7 参不符)。

- [ ] **Step 7: 实现 syscall 侧**

`SYS_ENEMY_HP` 进读族空号(0-11 已用,取 12):

```rust
/// 查敌读口(A5 补遗):活敌返 hp,其余 -1。P4-b:句柄是池 index,悬垂/复用不可辨,
/// 越界/死槽一律 -1 不 Fault——stage 编排等 boss 死的轮询原语。
pub const SYS_ENEMY_HP: u16 = 12;
```

dispatch match 加 `SYS_ENEMY_HP => sys_enemy_hp(task, ctx),`;实现:

```rust
fn sys_enemy_hp(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let handle = pop(task)?;
    let idx = handle as usize;
    let alive = handle >= 0
        && idx < crate::enemy::EnemyPool::CAP
        && ctx.body.enemies.is_alive(idx);
    push(task, if alive { ctx.body.enemies.hp[idx] } else { -1 })
}
```

`sys_spawn_enemy` 重写(弹出序 = 压栈逆序:task_script → sprite → score → drop_table → hp → y → x;先验后建镜像 `sys_fire` 的 task 路径,含 `SubKind::Async`+零参校验;`use` 处补 `OWNER_ENEMY`):

```rust
fn sys_spawn_enemy(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let task_script = pop(task)?;
    let sprite = pop(task)?;
    let score = pop(task)?;
    let drop_table = pop(task)?;
    let hp = pop(task)?;
    let y_raw = pop(task)?;
    let x_raw = pop(task)?;

    // task 号先验后建(镜像 sys_fire:坏号 FAULT_BAD_OP,敌未建;Async+零参白名单)
    let task_sub: Option<SubId> = if task_script >= 0 {
        let raw = u16::try_from(task_script).map_err(|_| FAULT_BAD_OP)?;
        let sub = ctx.ecl.sub_id(raw).ok_or(FAULT_BAD_OP)?;
        let meta = ctx.ecl.sub_meta(sub).ok_or(FAULT_BAD_OP)?;
        if meta.kind() != SubKind::Async
            || ctx.ecl.param_types(sub).is_none_or(|p| !p.is_empty())
        {
            return Err(FAULT_BAD_OP);
        }
        Some(sub)
    } else {
        None
    };

    let init = EnemyInit {
        // ……既有字段全数照旧(exhaustive Init),仅两处变:
        sprite: sprite as u16,
        main_task: 0, // 任务 spawn 后回填(敌句柄先于任务存在)
        // ……
    };
    let handle = ctx.body.create_enemy(init);
    if handle == EnemyHandle::NULL {
        return push(task, -1);
    }
    push(task, handle.index as i32)?;

    if let Some(sub) = task_sub {
        let pc0 = ctx.ecl.sub_meta(sub).expect("已在上面校验过").code_entry();
        let owner = (OWNER_ENEMY, handle.index, handle.generation);
        let parent = ctx.self_index + 1;
        match ctx.tasks.spawn(sub, pc0, owner, parent, ctx.frame) {
            Some(slot) => {
                ctx.body.enemies.main_task[handle.index as usize] = slot as u32 + 1;
            }
            None => {
                ctx.body.diag.pool_full[crate::world::POOL_TASK] =
                    ctx.body.diag.pool_full[crate::world::POOL_TASK].wrapping_add(1);
            }
        }
    }
    Ok(())
}
```

同步更新 `sys_spawn_enemy` 头注释(删"main_task/death_script 恒 0…A5 缺口"段,death_script 仍恒 0)。

- [ ] **Step 8: 机械迁移全部调用点**

```bash
grep -rn "spawn_enemy(" --include="*.ecl" --include="*.rs" --include="*.md" .
```
每处旧 5 参调用尾追 `, 0, none`(.ecl)/等价 args(.rs 测试直接给 7 个栈值,task 位 -1);
`crates/stg-godot/smoke/godot_smoke.ecl` 与 `crates/stg-godot/src/frame.rs` SRC 必中。
文档手写例子同改(ecl-lang.md 生成段交给 Step 10 的 gen-ecl-meta,手写段人工改)。

- [ ] **Step 9: 全绿 + 金向量逐位不变**

```bash
cargo test --workspace
cargo run -p stg-harness -- golden --out .superpowers/godot-scene/golden-a5.txt
diff .superpowers/godot-scene/golden-base.txt .superpowers/godot-scene/golden-a5.txt
```
Expected: 测试全 PASS;diff **空输出**(两场景不调 spawn_enemy)。非空 = 停手排查,不许承认平移。

- [ ] **Step 10: 元数据同步 + 桥冒烟**

```bash
cargo run -p stg-harness -- gen-ecl-meta
bash crates/stg-godot/smoke/run-smoke.sh
```
Expected: 生成 sink 更新(diff 里只见 spawn_enemy/enemy_hp 两条目);SMOKE OK。

- [ ] **Step 11: fmt/clippy + commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add -A && git commit -m "feat(ecl): A5 乙案——spawn_enemy 长 sprite+task 参(owner=新敌,main_task 通电)+enemy_hp 读口;金向量逐位不变实证"
```

---

### Task 2: 渲染契约文档 + 占位图集管线

**Files:**
- Create: `docs/render-contract.md`
- Create: `godot/tools/gen_atlas.gd`
- Create(产物 commit): `godot/assets/{bullets,shots,enemies,items,player,hitbox}.png`

**Interfaces:**
- Produces(T4 依赖): 图集网格契约——每层独立 PNG、独立 id 空间、**sprite 号=格号(行优先)**;cell 尺寸 bullets/shots/items **32×32**、enemies **64×64**;网格 bullets 8×1 / shots 4×1 / enemies 4×1 / items 8×1;越界号 shader `mod` 回卷(表现层容错)。
- Produces: `docs/render-contract.md` = 表现层契约唯一权威(美术+壳作者读)。

- [ ] **Step 1: 写 `docs/render-contract.md`**(全文结构,逐节实写,不留 TBD):

```markdown
# 渲染契约(表现层权威;首要读者:美术 + Godot 壳作者)

## 1. 两条通道(定位)
通道 A 状态视图 → 四层实例缓冲(本文 §2-3);通道 B 离散请求 → 分发器(§4)。
权威上游:crates/stg-godot/src/frame.rs(编码器)/ crates/stg-core/src/reqs.rs(请求 id)。

## 2. 实例缓冲布局(冻结,stride 12)
[cos,-sin,0,x, sin,cos,0,y, sprite,0,0,0] —— 前 8 = MULTIMESH_TRANSFORM_2D,
后 4 = INSTANCE_CUSTOM;custom.x=sprite 号,y/z/w 保留(将来 scale/alpha/调色,stride 不变)。
bullets 层带旋转(basis=角度),其余层单位 basis。压实前缀 + set_visible_instances。

## 3. 图集契约(每层独立 PNG + 独立 id 空间)
| 层 | 文件 | cell | 网格 | id 源 |
|---|---|---|---|---|
| bullets | assets/bullets.png | 32×32 | 8×1 | tables appearances[].sprite(现 0..3) |
| shots   | assets/shots.png   | 32×32 | 4×1 | shottype 表 sprite |
| enemies | assets/enemies.png | 64×64 | 4×1 | spawn_enemy sprite 参(A5 起脚本自给) |
| items   | assets/items.png   | 32×32 | 8×1 | tables item_cfg[].sprite |
sprite 号 = 格号(行优先);越界号 mod 回卷。QuadMesh 尺寸 = cell 尺寸(1px=1unit)。
自机 assets/player.png(32×32 单图)/判定点 assets/hitbox.png(16×16)。
换真美术:只换 PNG(同网格),契约与代码零改动;要变网格,改本表 + playfield.gd 常量即可。

## 4. 请求分发(引擎段 id 1..7)
表:reqs.rs 模块文档为准(id/args 逐位);GDScript 侧 dispatcher.gd 本地常量镜像。
64+ 脚本段:内容包经 dispatcher.register(id, callable) 自注册。

## 5. 锚点双表示规矩(硬规矩)
事件(REQ_BGM/BG/BG_PHASE)= 边沿;anchors() 四字段 = 电平。宿主在 new_game_at/
load_state 成功后必须一次性读 anchors() 对表;游玩期只走请求增量。
bg 段内局部时间 = frame - bg_phase_frame(A4 mini-VM 的 seek 契约,本刀 phase 硬编码)。

## 6. 坐标与画面
场界 x∈[-192,192], y∈[0,448](中轴原点);SubViewport 384×448 @容器(32,16),
世界根 Node2D@(192,0);640×480 窗口,canvas_items 拉伸。定点→浮点仅 raw/65536 一处。
```

- [ ] **Step 2: 写 `godot/tools/gen_atlas.gd`**(SceneTree 脚本,确定性像素绘制,无随机/无时钟):

```gdscript
extends SceneTree
# 占位图集一次性生成(产物 commit,同烘焙表纪律:重跑逐位一致)。
# 用法: $GODOT_BIN --headless --path godot --script res://tools/gen_atlas.gd
const PALETTE := [
	Color(0.95, 0.30, 0.30), Color(0.30, 0.55, 0.95), Color(0.35, 0.85, 0.40),
	Color(0.95, 0.80, 0.25), Color(0.80, 0.40, 0.90), Color(0.30, 0.85, 0.85),
	Color(0.95, 0.55, 0.25), Color(0.75, 0.75, 0.80),
]

func _init() -> void:
	_atlas("res://assets/bullets.png", 8, 32, _cell_bullet)
	_atlas("res://assets/shots.png", 4, 32, _cell_shot)
	_atlas("res://assets/enemies.png", 4, 64, _cell_enemy)
	_atlas("res://assets/items.png", 8, 32, _cell_item)
	_atlas("res://assets/player.png", 1, 32, _cell_player)
	_atlas("res://assets/hitbox.png", 1, 16, _cell_hitbox)
	print("ATLAS OK")
	quit(0)

func _atlas(path: String, cols: int, cell: int, painter: Callable) -> void:
	var img := Image.create(cols * cell, cell, false, Image.FORMAT_RGBA8)
	img.fill(Color(0, 0, 0, 0))
	for c in cols:
		painter.call(img, c * cell, cell, c)
	var err := img.save_png(path)
	assert(err == OK, "save_png 失败: " + path)

# 距离场画圆:核白心 + 色环(占位弹的通用观感)
func _disc(img: Image, ox: int, cell: int, r: float, col: Color) -> void:
	var cx := cell / 2.0
	for y in cell:
		for x in cell:
			var d := Vector2(x + 0.5 - cx, y + 0.5 - cx).length()
			if d < r * 0.55:
				img.set_pixel(ox + x, y, Color(1, 1, 1, 1))
			elif d < r:
				img.set_pixel(ox + x, y, col)
			elif d < r + 1.5:
				var a := clampf(r + 1.5 - d, 0.0, 1.0)
				img.set_pixel(ox + x, y, Color(col.r, col.g, col.b, a))

func _cell_bullet(img: Image, ox: int, cell: int, i: int) -> void:
	# 0=小圆 1=中圆 2=大圆 3=菱形星 4..7=色变圆(号→形/色都判别,美术期整格替换)
	var col: Color = PALETTE[i % PALETTE.size()]
	if i == 3:
		var cx := cell / 2.0
		for y in cell:
			for x in cell:
				var d: float = abs(x + 0.5 - cx) + abs(y + 0.5 - cx) # 菱形度量
				if d < 6.0: img.set_pixel(ox + x, y, Color(1, 1, 1, 1))
				elif d < 11.0: img.set_pixel(ox + x, y, col)
	else:
		_disc(img, ox, cell, [6.0, 9.0, 13.0, 0.0, 7.0, 8.0, 10.0, 11.0][i], col)

func _cell_shot(img: Image, ox: int, cell: int, i: int) -> void:
	# 自机弹:竖长针(椭圆度量),号变色
	var col: Color = PALETTE[(i + 1) % PALETTE.size()]
	var cx := cell / 2.0
	for y in cell:
		for x in cell:
			var d := Vector2((x + 0.5 - cx) / 0.35, (y + 0.5 - cx)).length()
			if d < 10.0: img.set_pixel(ox + x, y, Color(col.r, col.g, col.b, 0.9))

func _cell_enemy(img: Image, ox: int, cell: int, i: int) -> void:
	# 0=杂兵 1=boss 2/3=备用;大圆身 + 深色描边由 _disc 环体现
	_disc(img, ox, cell, [18.0, 26.0, 20.0, 22.0][i], PALETTE[(i + 4) % PALETTE.size()])

func _cell_item(img: Image, ox: int, cell: int, i: int) -> void:
	# 方块图标,号变色(P 点/分点等观感区分交真美术)
	var col: Color = PALETTE[i % PALETTE.size()]
	for y in range(8, cell - 8):
		for x in range(8, cell - 8):
			img.set_pixel(ox + x, y, col)

func _cell_player(img: Image, ox: int, cell: int, _i: int) -> void:
	# 上尖三角(自机朝上)
	var cx := cell / 2.0
	for y in cell:
		for x in cell:
			if absf(x + 0.5 - cx) < float(y) * 0.45 and y > 4 and y < cell - 4:
				img.set_pixel(ox + x, y, Color(0.9, 0.9, 1.0, 1.0))

func _cell_hitbox(img: Image, ox: int, cell: int, _i: int) -> void:
	_disc(img, ox, cell, 5.0, Color(1.0, 0.2, 0.2))
```

- [ ] **Step 3: 生成 + 确定性验证**(注意:此时 `godot/project.godot` 未建——先建一个только含 `config_version=5` 的最小占位文件让 `--path godot` 可用,T3 会覆写完整版):

```bash
mkdir -p godot/assets godot/tools
printf 'config_version=5\n' > godot/project.godot
GODOT_BIN=/data/sunyunbo/playground/godot/Godot_v4.6.3-stable_linux.x86_64
"$GODOT_BIN" --headless --path godot --script res://tools/gen_atlas.gd
md5sum godot/assets/*.png > /tmp/atlas1.md5
"$GODOT_BIN" --headless --path godot --script res://tools/gen_atlas.gd
md5sum -c /tmp/atlas1.md5
```
Expected: 两次 `ATLAS OK`;md5 全 OK(逐位一致)。

- [ ] **Step 4: commit**

```bash
git add docs/render-contract.md godot/
git commit -m "feat(godot): 渲染契约文档收口 + 占位图集管线(确定性生成,产物 commit,美术只换 PNG)"
```

---

### Task 3: Godot 工程骨架(project/输入/状态机/冒烟通路)

**Files:**
- Modify: `godot/project.godot`(T2 占位 → 完整)
- Create: `godot/stg_godot.gdextension`、`godot/scenes/main.tscn`、`godot/scripts/main.gd`、`godot/scripts/input.gd`、`godot/smoke/run-smoke.sh`
- 入库: `--import` 产生的 `.uid` sidecar

**Interfaces:**
- Consumes: WorldBridge 既有 15 口(`new_game_at`/`step_frame`/`take_requests`/`register_layer`/`anchors`/`hud_*`/`player_pos`/`frame`/`checksum` …)+ `BTN_*` 常量。
- Produces(T4/T5 依赖): `main.gd` 成员 `bridge: WorldBridge`、状态机 `enum S { PLAYING, PAUSED, STAGE_CLEAR }`、钩子函数 `func _after_step() -> void`(T4/T5 填内容);`input.gd` 类 `StgInput.mask() -> int`;`main.gd` 的 `_boot(start: int) -> bool` 读 `res://ecl/demo/*.ecl` 按名排序喂 `new_game_at`(seed=1, rank=2, loadout 0,0,3,3)。
- Produces: `--smoke` 模式(`OS.get_cmdline_user_args`)v0:T3 阶段 demo .ecl 未建,冒烟用内置最小源 `"sub main() { bgm(3); loop { wait(60); } }"` 单单元开机,60 帧后断言 `frame()>=60`、`checksum()!=0`、`anchors().bgm==3`,打印 `SMOKE OK` 后 `quit(0)`(断言败 `quit(1)`)。T6 换成 demo 两次开机。

- [ ] **Step 1: 写 `godot/project.godot`**

```ini
; Godot 4.6 工程文件(场景刀 T3;经典 640×480,弹幕域 384×448 @ (32,16))
config_version=5

[application]
config/name="stg-engine demo"
run/main_scene="res://scenes/main.tscn"
config/features=PackedStringArray("4.6")

[display]
window/size/viewport_width=640
window/size/viewport_height=480
window/stretch/mode="canvas_items"
window/stretch/aspect="keep"

[physics]
common/physics_ticks_per_second=60

[rendering]
textures/canvas_textures/default_texture_filter=0
```

- [ ] **Step 2: 写 `godot/stg_godot.gdextension`**(路径比桥冒烟浅两级):

```ini
[configuration]
entry_symbol = "gdext_rust_init"
compatibility_minimum = 4.6
reloadable = false

[libraries]
linux.debug.x86_64 = "res://../target/debug/libstg_godot.so"
linux.release.x86_64 = "res://../target/release/libstg_godot.so"
windows.debug.x86_64 = "res://../target/x86_64-pc-windows-msvc/debug/stg_godot.dll"
windows.release.x86_64 = "res://../target/x86_64-pc-windows-msvc/release/stg_godot.dll"
```

- [ ] **Step 3: 写 `godot/scenes/main.tscn`**(最小壳,树在代码里长——编辑器化留美术期):

```
[gd_scene load_steps=2 format=3]

[ext_resource type="Script" path="res://scripts/main.gd" id="1"]

[node name="Main" type="Node"]
script = ExtResource("1")
```

- [ ] **Step 4: 写 `godot/scripts/input.gd`**

```gdscript
class_name StgInput
extends Node
## InputMap 代码注册(免手写序列化;物理键:方向键 + Z 射 X bomb Shift 低速)。
## 位掩码零翻译:WorldBridge.BTN_* 即 stg-core 动作位(坑档 C1 红利)。

const KEYS := {
	"stg_up": KEY_UP, "stg_down": KEY_DOWN,
	"stg_left": KEY_LEFT, "stg_right": KEY_RIGHT,
	"stg_shot": KEY_Z, "stg_bomb": KEY_X, "stg_slow": KEY_SHIFT,
}

func _ready() -> void:
	for a in KEYS:
		if InputMap.has_action(a):
			continue
		InputMap.add_action(a)
		var ev := InputEventKey.new()
		ev.physical_keycode = KEYS[a]
		InputMap.action_add_event(a, ev)

func mask() -> int:
	var m := 0
	if Input.is_action_pressed("stg_up"): m |= WorldBridge.BTN_UP
	if Input.is_action_pressed("stg_down"): m |= WorldBridge.BTN_DOWN
	if Input.is_action_pressed("stg_left"): m |= WorldBridge.BTN_LEFT
	if Input.is_action_pressed("stg_right"): m |= WorldBridge.BTN_RIGHT
	if Input.is_action_pressed("stg_shot"): m |= WorldBridge.BTN_SHOT
	if Input.is_action_pressed("stg_bomb"): m |= WorldBridge.BTN_BOMB
	if Input.is_action_pressed("stg_slow"): m |= WorldBridge.BTN_SLOW
	return m
```

- [ ] **Step 5: 写 `godot/scripts/main.gd`**(状态机 + 回路 + 冒烟 v0;`_after_step()` 是 T4/T5 的挂点,本任务空实现):

```gdscript
extends Node
## 状态机 + 每帧回路(spec §6)。宿主暂停 = 不调 step_frame(世界时间线零帧)。

enum S { PLAYING, PAUSED, STAGE_CLEAR }

var state: int = S.PLAYING
var bridge: WorldBridge
var stg_input: StgInput
var smoke := false

const SMOKE_SRC := "sub main() { bgm(3); loop { wait(60); } }"

func _ready() -> void:
	smoke = "--smoke" in OS.get_cmdline_user_args()
	bridge = WorldBridge.new()
	add_child(bridge)
	stg_input = StgInput.new()
	add_child(stg_input)
	if smoke:
		_run_smoke() # async,自行 quit
	else:
		if not _boot(0):
			push_error("[stg] 开局失败")
			get_tree().quit(1)

## 读 res://ecl/demo/*.ecl(按名排序)开局;T3 期目录还没有内容 → 回退内置最小源。
func _boot(start: int) -> bool:
	var names := PackedStringArray()
	var sources := PackedStringArray()
	var dir := DirAccess.open("res://ecl/demo")
	if dir != null:
		var files: Array[String] = []
		for f in dir.get_files():
			if f.ends_with(".ecl"):
				files.append(f)
		files.sort()
		for f in files:
			names.append(f)
			sources.append(FileAccess.get_file_as_string("res://ecl/demo/" + f))
	if names.is_empty():
		names.append("inline.ecl")
		sources.append(SMOKE_SRC)
	var ok := bridge.new_game_at(names, sources, 1, 2, start, 0, 0, 3, 3)
	if ok:
		_sync_anchors() # 双表示规矩:开机后一次性对电平(T5 实装演出)
	return ok

func _sync_anchors() -> void:
	var a := bridge.anchors() # T5 起喂给 hud/bg;T3 只留读口热身
	if a.is_empty():
		push_error("[stg] anchors 空(未开局?)")

func _physics_process(_dt: float) -> void:
	if state != S.PLAYING:
		return
	bridge.step_frame(stg_input.mask())
	_after_step()

## T4(渲染)/T5(分发器 HUD)在此挂逐帧消费;T3 空置。
func _after_step() -> void:
	pass

func _unhandled_input(ev: InputEvent) -> void:
	if ev.is_action_pressed("ui_cancel"):
		if state == S.PLAYING:
			state = S.PAUSED
		elif state == S.PAUSED:
			state = S.PLAYING

## ── 冒烟(v0:内置源;T6 换 demo 两次开机)────────────────────────────
func _run_smoke() -> void:
	var fails := 0
	if not _boot(0):
		print("SMOKE FAIL: boot")
		get_tree().quit(1)
		return
	for i in 60:
		await get_tree().physics_frame
	fails += _chk(bridge.frame() >= 60, "frame>=60, got %d" % bridge.frame())
	fails += _chk(bridge.checksum() != 0, "checksum!=0")
	fails += _chk(int(bridge.anchors().get("bgm", -1)) == 3, "anchors.bgm==3")
	if fails == 0:
		print("SMOKE OK")
	get_tree().quit(0 if fails == 0 else 1)

func _chk(cond: bool, msg: String) -> int:
	if not cond:
		print("SMOKE FAIL: ", msg)
		return 1
	return 0
```

- [ ] **Step 6: 写 `godot/smoke/run-smoke.sh`**(B22 免疫:诊断永远先落地再判定):

```bash
#!/usr/bin/env bash
# 真工程 headless 冒烟。诊断纪律:不许 set -e + 命令替换吞输出(follow-ups B22)。
set -uo pipefail
cd "$(dirname "$0")/.."
GODOT_BIN="${GODOT_BIN:-/data/sunyunbo/playground/godot/Godot_v4.6.3-stable_linux.x86_64}"
( cd .. && cargo build -p stg-godot ) || exit 1
# 首跑 --import 生成 .godot/ 缓存;冷缓存可能 SIGABRT(坑档 G1),非致命
"$GODOT_BIN" --headless --path . --import >/dev/null 2>&1 || true
out=$("$GODOT_BIN" --headless --path . -- --smoke 2>&1)
st=$?
echo "$out"
[ "$st" -eq 0 ] && grep -q "SMOKE OK" <<<"$out"
```

- [ ] **Step 7: 跑通冒烟 + `.uid` 入库**

```bash
chmod +x godot/smoke/run-smoke.sh
bash godot/smoke/run-smoke.sh
git status --short godot/   # 应见 *.uid 新文件(G2:随源入库)
```
Expected: `SMOKE OK`,退出码 0。

- [ ] **Step 8: commit**

```bash
git add godot/
git commit -m "feat(godot): 真工程骨架——project/gdextension/主场景/输入映射/状态机/--smoke 通路(内置源 v0)"
```

---

### Task 4: 渲染链——四层 MultiMesh + shader + 自机/背景

**Files:**
- Create: `godot/shaders/layer.gdshader`、`godot/scripts/playfield.gd`、`godot/scripts/bg.gd`
- Modify: `godot/scripts/main.gd`(建 Playfield 子树 + `_after_step` 挂渲染消费 + 冒烟补层注册断言)

**Interfaces:**
- Consumes: T2 图集契约(cell/网格/文件名);T3 `main.gd` 骨架、`bridge.register_layer(kind, rid)`(判据 buffer 尺寸==cap×12,headless 须先 `set_buffer` 播种——bridge.rs 注释即契约)、`bridge.player_pos()`、`WorldBridge.LAYER_*` 常量。
- Produces(T5/T6 依赖): `Playfield` 类(`class_name Playfield extends SubViewportContainer`),成员 `world_root: Node2D`(Effects 挂点,世界坐标系)、`bg: Bg`;方法 `setup(bridge) -> bool`(建层+注册,全成才 true)、`update_view(bridge) -> void`(自机位置/判定点显隐);`Bg` 类方法 `set_bg(id: int)`、`set_phase(phase: int)`。

- [ ] **Step 1: 写 `godot/shaders/layer.gdshader`**

```glsl
shader_type canvas_item;
// 图集选格:INSTANCE_CUSTOM.x = sprite 号 = 格号(行优先;render-contract §3)。
// custom.y/z/w 保留空位(scale/alpha/调色),stride 12 冻结。
uniform float grid_cols = 8.0;
uniform float grid_rows = 1.0;

varying flat vec2 cell;

void vertex() {
	float s = INSTANCE_CUSTOM.x;
	float total = grid_cols * grid_rows;
	s = mod(s, total); // 越界号回卷(表现层容错,render-contract §3)
	cell = vec2(mod(s, grid_cols), floor(s / grid_cols));
}

void fragment() {
	vec2 uv = (cell + UV) / vec2(grid_cols, grid_rows);
	COLOR = texture(TEXTURE, uv);
}
```

- [ ] **Step 2: 写 `godot/scripts/bg.gd`**

```gdscript
class_name Bg
extends Node2D
## 占位背景:底色 + 滚动网格线。bg 号→底色;bg_phase 硬编码 0=滚 1=停(A4 mini-VM 接管前)。
## 表现层自有时钟(_process float dt)——断层线上,无纪律负担。

const BG_COLORS := { 0: Color(0.06, 0.06, 0.10), 1: Color(0.05, 0.10, 0.08), 2: Color(0.10, 0.05, 0.10) }

var base_color: Color = BG_COLORS[0]
var scrolling := true
var offset := 0.0

func set_bg(id: int) -> void:
	base_color = BG_COLORS.get(id, BG_COLORS[0])
	queue_redraw()

func set_phase(phase: int) -> void:
	scrolling = phase == 0

func _process(dt: float) -> void:
	if scrolling:
		offset = fmod(offset + 40.0 * dt, 32.0)
		queue_redraw()

func _draw() -> void:
	draw_rect(Rect2(0, 0, 384, 448), base_color)
	var line := Color(1, 1, 1, 0.05)
	var y := offset - 32.0
	while y < 448.0:
		draw_line(Vector2(0, y), Vector2(384, y), line)
		y += 32.0
	for x in range(0, 385, 32):
		draw_line(Vector2(x, 0), Vector2(x, 448), line)
```

- [ ] **Step 3: 写 `godot/scripts/playfield.gd`**

```gdscript
class_name Playfield
extends SubViewportContainer
## 弹幕域:SubViewport 384×448;世界根@(192,0)(世界坐标即本地坐标)。
## z 序(节点序,下→上):Bg < shots < enemies < items < Player < bullets < Effects(T5 挂)。

# 池容量镜像(值源 crates/stg-core/src/{bullets,shots,enemy,items}.rs define_pool! 声明;
# 桥面冻结不出容量口——漂移由 register_layer false + 冒烟兜底,见 setup)
const CAPS := { 0: 8192, 1: 1024, 2: 256, 3: 512 } # key = WorldBridge.LAYER_*
const CELLS := { 0: 32, 1: 32, 2: 64, 3: 32 }
const COLS := { 0: 8, 1: 4, 2: 4, 3: 8 }
const TEXTURES := {
	0: "res://assets/bullets.png", 1: "res://assets/shots.png",
	2: "res://assets/enemies.png", 3: "res://assets/items.png",
}
# 节点序 = 绘制序;bullets 最上(东方惯例)
const Z_ORDER := [1, 2, 3, 0] # shots, enemies, items, bullets——player 插在 items 后

var viewport: SubViewport
var world_root: Node2D
var bg: Bg
var player: Sprite2D
var hitbox: Sprite2D
var layer_nodes := {}

func _init() -> void:
	position = Vector2(32, 16)
	stretch = true
	viewport = SubViewport.new()
	viewport.size = Vector2i(384, 448)
	viewport.disable_3d = true
	viewport.render_target_update_mode = SubViewport.UPDATE_ALWAYS
	add_child(viewport)
	custom_minimum_size = Vector2(384, 448)

	bg = Bg.new()
	viewport.add_child(bg)
	world_root = Node2D.new()
	world_root.position = Vector2(192, 0)
	viewport.add_child(world_root)

	for kind in Z_ORDER:
		var mmi := _make_layer(kind)
		layer_nodes[kind] = mmi
		world_root.add_child(mmi)
		if kind == 3: # items 之后插 player(player 在 items 上、bullets 下)
			_make_player()

	var fx_root := Node2D.new() # Effects 挂点(T5 用),压 bullets 之上
	fx_root.name = "FxRoot"
	world_root.add_child(fx_root)

func _make_layer(kind: int) -> MultiMeshInstance2D:
	var mmi := MultiMeshInstance2D.new()
	var mm := MultiMesh.new()
	mm.transform_format = MultiMesh.TRANSFORM_2D
	mm.use_custom_data = true
	mm.instance_count = CAPS[kind]
	var quad := QuadMesh.new()
	quad.size = Vector2(CELLS[kind], CELLS[kind])
	mm.mesh = quad
	mmi.multimesh = mm
	mmi.texture = load(TEXTURES[kind])
	var mat := ShaderMaterial.new()
	mat.shader = load("res://shaders/layer.gdshader")
	mat.set_shader_parameter("grid_cols", float(COLS[kind]))
	mat.set_shader_parameter("grid_rows", 1.0)
	mmi.material = mat
	# 播种定长零缓冲:①headless dummy renderer 下 get_buffer 才可用;②register_layer
	# 的判据就是 buffer 长度==cap×12(bridge.rs 契约注释)
	var buf := PackedFloat32Array()
	buf.resize(CAPS[kind] * 12)
	RenderingServer.multimesh_set_buffer(mm.get_rid(), buf)
	mm.visible_instance_count = 0
	return mmi

func _make_player() -> void:
	player = Sprite2D.new()
	player.texture = load("res://assets/player.png")
	world_root.add_child(player)
	hitbox = Sprite2D.new()
	hitbox.texture = load("res://assets/hitbox.png")
	hitbox.visible = false
	player.add_child(hitbox)

## 四层注册;任何一层失败 → push_error + false(容量镜像漂移在此炸出,冒烟接得住)
func setup(bridge: WorldBridge) -> bool:
	var ok := true
	for kind in layer_nodes:
		var mm: MultiMesh = layer_nodes[kind].multimesh
		if not bridge.register_layer(kind, mm.get_rid()):
			push_error("[stg] register_layer(%d) 被拒(容量镜像漂移?)" % kind)
			ok = false
	return ok

func update_view(bridge: WorldBridge) -> void:
	player.position = bridge.player_pos()
	hitbox.visible = Input.is_action_pressed("stg_slow")
```

- [ ] **Step 4: `main.gd` 接线**(`_ready` 建 Playfield;`_after_step` 填渲染消费;冒烟加层注册断言):

```gdscript
# _ready() 里 add_child(stg_input) 之后:
	playfield = Playfield.new()
	add_child(playfield)
# _boot() 成功分支、_sync_anchors() 之后:
	if not playfield.setup(bridge):
		return false
# _after_step() 改为:
	playfield.update_view(bridge)
```
冒烟真断言:`_boot` 已含 `playfield.setup`(注册失败即 boot 失败),另在 `_run_smoke` 里于 60 帧后追加:

```gdscript
	var mm: MultiMesh = playfield.layer_nodes[WorldBridge.LAYER_BULLETS].multimesh
	var buf := RenderingServer.multimesh_get_buffer(mm.get_rid())
	fails += _chk(buf.size() == 8192 * 12, "bullets 缓冲尺寸")
```
(成员声明 `var playfield: Playfield` 同步补上。)

- [ ] **Step 5: 跑冒烟**

```bash
bash godot/smoke/run-smoke.sh
```
Expected: `SMOKE OK`(层注册全过;内置源无弹,visible 0 合法)。

- [ ] **Step 6: 有头快速目验(可选,本机有 X 则跑)**

```bash
cd godot && /data/sunyunbo/playground/godot/Godot_v4.6.3-stable_linux.x86_64 --path . 2>&1 | head -5
```
Expected: 窗口开、背景网格滚动、自机三角在 (0,384)(屏 224,400 附近)。无 X 就跳过,冒烟已覆盖。

- [ ] **Step 7: commit**

```bash
git add godot/
git commit -m "feat(godot): 渲染链——四层 MultiMesh+图集 shader+自机/判定点/滚动背景,冒烟断言层注册"
```

---

### Task 5: 分发器 + HUD + 演出 + 结算/暂停

**Files:**
- Create: `godot/scripts/dispatcher.gd`、`godot/scripts/hud.gd`、`godot/scripts/effects.gd`、`godot/scripts/content_tables.gd`
- Modify: `godot/scripts/main.gd`(接线 + 状态机补 STAGE_CLEAR/暂停 overlay)

**Interfaces:**
- Consumes: T4 `Playfield.world_root`/`FxRoot`;`bridge.take_requests()`(字典数组:id/seq/frame/args)、`hud_player`/`hud_boss(0)`/`hud_spell(0)`/`anchors()`。
- Produces(T6 依赖): `Dispatcher.register(id, callable)`、`Dispatcher.drain(arr)`;`Hud.refresh(bridge)`、`Hud.show_banner(text, secs)`、`Hud.set_bgm_label(name)`;`Effects.explosion(pos, score)`;main.gd 把 REQ 1..7 全部接妥;结算 overlay 显示 `hud_player.score`,按 Z 重开(`_boot(0)` 重来)。

- [ ] **Step 1: 写 `godot/scripts/content_tables.gd`**

```gdscript
class_name ContentTables
## 演出名表归内容包(id→名是表现层契约,引擎不注册;render-contract §4)。
const BGM_NAMES := { 1: "Stage 1 ~ Placeholder March", 2: "Boss ~ Windchime of Seven Colors" }
const SPELL_NAMES := { 1: "風鈴「Rainbow Wind Chime」" }
```

- [ ] **Step 2: 写 `godot/scripts/dispatcher.gd`**

```gdscript
class_name Dispatcher
extends Node
## 通道 B 路由:id → Callable。"核出请求,壳做演出"的壳侧落点。
## id 值源 crates/stg-core/src/reqs.rs(半冻结契约,1..=63 引擎段;桥面冻结不镜像常量)。

const REQ_ENEMY_DEATH := 1
const REQ_SPELL_DECLARE := 2
const REQ_SPELL_RESULT := 3
const REQ_STAGE_CLEAR := 4
const REQ_BGM := 5
const REQ_BG := 6
const REQ_BG_PHASE := 7

var handlers := {}
var _warned := {}

func register(id: int, fn: Callable) -> void:
	handlers[id] = fn

func drain(arr) -> void:
	for d in arr:
		var id: int = d["id"]
		if handlers.has(id):
			handlers[id].call(d["args"])
		elif not _warned.has(id):
			_warned[id] = true
			push_warning("[stg] 未注册请求 id=%d(同类后续不再报)" % id)
```

- [ ] **Step 3: 写 `godot/scripts/effects.gd`**

```gdscript
class_name Effects
extends Node2D
## 一次性演出(挂 Playfield.world_root/FxRoot 下,世界坐标)。表现层自有计时,无纪律负担。

func explosion(pos: Vector2, score: int) -> void:
	var e := _Ring.new()
	e.position = pos
	add_child(e)
	if score > 0:
		var l := Label.new()
		l.text = str(score)
		l.position = pos + Vector2(-12, -20)
		l.add_theme_font_size_override("font_size", 10)
		add_child(l)
		var tw := l.create_tween()
		tw.tween_property(l, "position:y", l.position.y - 24.0, 0.6)
		tw.parallel().tween_property(l, "modulate:a", 0.0, 0.6)
		tw.tween_callback(l.queue_free)

class _Ring extends Node2D:
	var t := 0.0
	func _process(dt: float) -> void:
		t += dt * 3.0
		if t >= 1.0:
			queue_free()
		queue_redraw()
	func _draw() -> void:
		draw_arc(Vector2.ZERO, 4.0 + t * 28.0, 0, TAU, 24,
			Color(1.0, 0.8, 0.4, 1.0 - t), 2.0)
```

- [ ] **Step 4: 写 `godot/scripts/hud.gd`**(代码建右栏 + boss 条 + 横幅;全 Label/ColorRect,无资产):

```gdscript
class_name Hud
extends CanvasLayer
## 右栏(x≥424):分/残机/bomb/power/graze + 曲名;boss 条覆盖弹幕域顶部;中央横幅。

var score_l: Label
var lives_l: Label
var bombs_l: Label
var power_l: Label
var graze_l: Label
var bgm_l: Label
var boss_bar: ColorRect
var boss_bar_bg: ColorRect
var spell_l: Label
var banner: Label
var _banner_left := 0.0

func _ready() -> void:
	var panel := VBoxContainer.new()
	panel.position = Vector2(424, 24)
	panel.custom_minimum_size = Vector2(200, 0)
	add_child(panel)
	score_l = _row(panel); lives_l = _row(panel); bombs_l = _row(panel)
	power_l = _row(panel); graze_l = _row(panel); bgm_l = _row(panel)

	boss_bar_bg = ColorRect.new()
	boss_bar_bg.position = Vector2(40, 20); boss_bar_bg.size = Vector2(368, 4)
	boss_bar_bg.color = Color(1, 1, 1, 0.15); boss_bar_bg.visible = false
	add_child(boss_bar_bg)
	boss_bar = ColorRect.new()
	boss_bar.position = Vector2(40, 20); boss_bar.size = Vector2(368, 4)
	boss_bar.color = Color(0.9, 0.25, 0.35); boss_bar.visible = false
	add_child(boss_bar)
	spell_l = Label.new()
	spell_l.position = Vector2(40, 26)
	spell_l.add_theme_font_size_override("font_size", 10)
	add_child(spell_l)

	banner = Label.new()
	banner.position = Vector2(120, 200)
	banner.add_theme_font_size_override("font_size", 16)
	banner.visible = false
	add_child(banner)

func _row(p: Container) -> Label:
	var l := Label.new()
	l.add_theme_font_size_override("font_size", 12)
	p.add_child(l)
	return l

func _process(dt: float) -> void:
	if _banner_left > 0.0:
		_banner_left -= dt
		if _banner_left <= 0.0:
			banner.visible = false

func show_banner(text: String, secs: float) -> void:
	banner.text = text
	banner.visible = true
	_banner_left = secs

func set_bgm_label(n: String) -> void:
	bgm_l.text = "♪ " + n

func refresh(bridge: WorldBridge) -> void:
	var p := bridge.hud_player()
	if p.is_empty():
		return
	score_l.text = "Score  %d" % int(p["score"])
	lives_l.text = "Player %d (%d)" % [int(p["lives"]), int(p["life_pieces"])]
	bombs_l.text = "Bomb   %d (%d)" % [int(p["bombs"]), int(p["bomb_pieces"])]
	power_l.text = "Power  %.2f" % (int(p["power"]) / 100.0)
	graze_l.text = "Graze  %d" % int(p["graze"])
	var b := bridge.hud_boss(0)
	var active := not b.is_empty() and int(b["active"]) == 1
	boss_bar.visible = active
	boss_bar_bg.visible = active
	if active:
		boss_bar.size.x = 368.0 * clampf(float(b["hp_ratio"]), 0.0, 1.0)
		var s := bridge.hud_spell(0)
		if not s.is_empty() and int(s["active"]) == 1:
			var sname: String = ContentTables.SPELL_NAMES.get(int(s["spell_id"]), "Spell #%d" % int(s["spell_id"]))
			spell_l.text = "%s  %d" % [sname, int(s["frames_left"]) / 60]
		else:
			spell_l.text = ""
	else:
		spell_l.text = ""
```

- [ ] **Step 5: `main.gd` 接线**(成员 `dispatcher/hud/effects` + `_ready` 建树 + 请求路由 + 状态机补全):

```gdscript
# _ready() 建 playfield 之后:
	effects = Effects.new()
	playfield.world_root.get_node("FxRoot").add_child(effects)
	hud = Hud.new()
	add_child(hud)
	dispatcher = Dispatcher.new()
	add_child(dispatcher)
	_wire_requests()

func _wire_requests() -> void:
	dispatcher.register(Dispatcher.REQ_ENEMY_DEATH, func(a):
		effects.explosion(Vector2(a[0] / 65536.0, a[1] / 65536.0), int(a[3])))
	dispatcher.register(Dispatcher.REQ_SPELL_DECLARE, func(a):
		hud.show_banner(ContentTables.SPELL_NAMES.get(int(a[0]), "Spell #%d" % int(a[0])), 2.5))
	dispatcher.register(Dispatcher.REQ_SPELL_RESULT, func(a):
		hud.show_banner("取得!" if int(a[1]) == 1 else "失敗…", 2.0))
	dispatcher.register(Dispatcher.REQ_STAGE_CLEAR, func(_a): _on_stage_clear())
	dispatcher.register(Dispatcher.REQ_BGM, func(a):
		hud.set_bgm_label(ContentTables.BGM_NAMES.get(int(a[0]), "BGM #%d" % int(a[0]))))
	dispatcher.register(Dispatcher.REQ_BG, func(a): playfield.bg.set_bg(int(a[0])))
	dispatcher.register(Dispatcher.REQ_BG_PHASE, func(a): playfield.bg.set_phase(int(a[0])))

# _sync_anchors() 实装(双表示规矩:电平追平,含中段开机/读档):
	var a := bridge.anchors()
	if a.is_empty():
		return
	hud.set_bgm_label(ContentTables.BGM_NAMES.get(int(a["bgm"]), "BGM #%d" % int(a["bgm"])))
	playfield.bg.set_bg(int(a["bg"]))
	playfield.bg.set_phase(int(a["bg_phase"]))

# _after_step() 补齐:
	dispatcher.drain(bridge.take_requests())
	hud.refresh(bridge)
	playfield.update_view(bridge)

func _on_stage_clear() -> void:
	state = S.STAGE_CLEAR
	var p := bridge.hud_player()
	hud.show_banner("STAGE CLEAR  Score %d  (Z restart)" % int(p.get("score", 0)), 3600.0)

# _unhandled_input 追加:STAGE_CLEAR 态按 stg_shot 重开
	elif state == S.STAGE_CLEAR and ev.is_action_pressed("stg_shot"):
		if _boot(0):
			state = S.PLAYING
```
(暂停 overlay:PAUSED 态 `hud.show_banner("PAUSE", 0.1)` 每帧续——或简单 Label 常显,实施者取顺手者,判据只有一条:PAUSED 不 step。)

- [ ] **Step 6: 冒烟回归**

```bash
bash godot/smoke/run-smoke.sh
```
Expected: `SMOKE OK`(内置源 bgm(3) 走 REQ_BGM → hud 标签路径被真实执行,无脚本错)。

- [ ] **Step 7: commit**

```bash
git add godot/
git commit -m "feat(godot): 分发器+HUD+演出——引擎 7 id 全接线/右栏/boss 条/横幅/爆点/结算重开/锚点对表"
```

---

### Task 6: demo 局内容 + 双冒烟收口(B18/B22 销账)

**Files:**
- Create: `godot/ecl/demo/main.ecl`、`godot/ecl/demo/stage1.ecl`、`godot/ecl/demo/boss_windchime.ecl`
- Modify: `godot/scripts/main.gd`(冒烟 v0 → demo 两次开机)
- Modify: `crates/stg-godot/smoke/godot_smoke.ecl` + `crates/stg-godot/smoke/smoke.gd`(B18 余量两断言)
- Modify: `crates/stg-godot/smoke/run-smoke.sh`(B22 修复)

**Interfaces:**
- Consumes: T1 `spawn_enemy` 7 参 + `enemy_hp`;T5 全部演出链;`mark`/多文件/锚点补偿(整局流程刀既有);引擎常量 `APPEARANCE_*`/`REQ_STAGE_CLEAR`(consts.rs 注入 .ecl 命名空间)。
- Produces: demo 局 = A5+compile_units+mark 的第一个真实消费者;`mark(1)`=杂兵段、`mark(2)`=boss 段(练习位)。

- [ ] **Step 1: 写 `godot/ecl/demo/main.ecl`**

```
// demo 局主编排。多文件单元:目录整取按名排序(boss_windchime < main < stage1),入口 main。
// 转场协议:风铃卡后 emit REQ_STAGE_CLEAR 挂牌,宿主停拍(世界时间线冻结),脚本驻留。
sub main() {
    bgm(1);
    bg(1);
    mark(1); // 杂兵段练习位
    stage1();
    mark(2) { // boss 段练习位。跳入补偿(自动):bg(1);块内手写的 bgm/bg_phase 抑制同类注入
        bgm(2);
        bg_phase(1); // boss 战背景停滚
    }
    boss_battle();
    bg_phase(0);
    add_score(100000); // 关底 bonus 世界内入账(结算数字在挂牌前定格)
    emit_req(REQ_STAGE_CLEAR, 1, 0, 0, 0, 0, 0);
    loop { wait(600); } // 挂牌后驻留
}
```

- [ ] **Step 2: 写 `godot/ecl/demo/stage1.ecl`**

```
// 杂兵段:三波×四机俯冲,瞄准三连发,底部退场(OOB 回收,任务随敌亡)。
async sub zako_dive() {
    move_to(90, $self_x, 140.0fx, 2);
    wait(90);
    for i in 0..3 {
        _ = fire(APPEARANCE_SMALL, $self_x, $self_y, 1.8fx, aim_player(), none, none);
        wait(25);
    }
    move_to(150, $self_x, 560.0fx, 1);
    wait(600);
}

sub stage1() {
    for w in 0..3 {
        for i in 0..4 {
            _ = spawn_enemy((i * 96 - 144) as fx, 2.0fx, 40, 1, 300, 0, zako_dive);
            wait(15);
        }
        wait(150);
    }
    wait(120); // 清场缓冲
}
```

- [ ] **Step 3: 写 `godot/ecl/demo/boss_windchime.ecl`**(风铃卡从 `crates/stg-harness/scenes/rainbow.ecl` 移植——patrol/windchime_pattern/xformdef 原样,boss_main 新增非符段,boss_battle 用 `enemy_hp` 等死):

```
// boss:非符(瞄准三叉)→ 风铃卡(rainbow.ecl 移植)。boss_main 是 enemy-owned 主任务
// (A5 乙案:spawn_enemy 第 7 参),敌死任务亡;boss_battle(STAGE 侧)用 enemy_hp 轮询等死。
const SPELL_WINDCHIME: int = 1;

xformdef WIND_CHIME {
    set_speed(2.0fx);
    @30 turn(90deg);
}

async sub patrol() {
    loop {
        move_to(90, -120fx, 100fx, 2);
        wait(90);
        move_to(90, 120fx, 100fx, 2);
        wait(90);
    }
}

async sub windchime_pattern() {
    var base: angle = 0deg;
    var volley: int = 0;
    loop {
        var ways: int = 28 + global(GVAR_RANK) * 2;
        var step_i: int = 65536 / ways;
        var astep: angle = step_i as angle;
        for i in 0..5 {
            var appearance: int = i % 4;
            var speed: fx = 1.0fx + i as fx * 0.25fx;
            _ = batch(appearance, $self_x, $self_y, ways, base, astep, 1, speed, 0fx);
        }
        if volley % 2 == 0 {
            for k in 0..16 {
                var ka: angle = (k * 4096) as angle;
                _ = fire(APPEARANCE_MEDIUM, $self_x, $self_y, 0fx, ka, WIND_CHIME, none);
            }
        }
        base = base + 7deg;
        volley = volley + 1;
        wait(50);
    }
}

async sub boss_main() {
    spawn patrol();
    // 非符段 10 秒:瞄准三叉,血条手喂(hp 真比值,C13② 同款)
    var t: int = 0;
    while t < 600 {
        boss_set(0, $self_hp as fx / $self_hp_max as fx, 0, 0, 2, 1);
        _ = fire(APPEARANCE_SMALL, $self_x, $self_y, 2.0fx, aim_player(), none, none);
        _ = fire(APPEARANCE_SMALL, $self_x, $self_y, 2.0fx, aim_player() + 12deg, none, none);
        _ = fire(APPEARANCE_SMALL, $self_x, $self_y, 2.0fx, aim_player() - 12deg, none, none);
        wait(20);
        t = t + 20;
    }
    spell_begin(0, SPELL_WINDCHIME, windchime_pattern, 3600, 100000, 0, 0);
    wait_spell();
    // 超时未破:退场(顶部飞出,OOB 回收 → boss_battle 的轮询放行;被击破则本任务已随敌亡)
    move_to(120, 0fx, -600.0fx, 1);
    wait(600);
}

sub boss_battle() {
    var boss: int = spawn_enemy(0.0fx, 96.0fx, 2600, 1, 5000, 1, boss_main);
    // 等 boss 死(enemy_hp<0)——带 75 秒兜底:若敌 OOB 回收纪律不含敌类(敌界放宽,
    // M0-13),超时退场路径下轮询会挂死,兜底保 demo 流程必然推进。
    var t: int = 0;
    var waiting: int = 1;
    while waiting == 1 {
        if enemy_hp(boss) < 0 { waiting = 0; }
        if t > 4500 { waiting = 0; }
        wait(10);
        t = t + 10;
    }
    wait(60); // 死亡演出缓冲
}
```

- [ ] **Step 4: 先拿编译器过一遍语法**

```bash
cargo run -p stg-harness -- check godot/ecl/demo
```
Expected: 三文件整局编译通过,零诊断。红了就修 .ecl(行列号带文件名)。

- [ ] **Step 5: `main.gd` 冒烟 v0 → demo 两次开机**(替换 `_run_smoke`;`SMOKE_SRC` 常量与回退分支删除——demo 目录已实存,读不到就是真错):

```gdscript
func _run_smoke() -> void:
	var fails := 0
	# ① start=0 正常开局:杂兵段跑 240 帧,全链路(编码/分发/HUD)真实走
	if not _boot(0):
		print("SMOKE FAIL: boot(0)")
		get_tree().quit(1)
		return
	var saw_bgm := false
	dispatcher.register(Dispatcher.REQ_BGM, func(_a): saw_bgm = true) # 覆盖注册以侦听
	for i in 240:
		await get_tree().physics_frame
	fails += _chk(bridge.frame() >= 240, "frame>=240")
	fails += _chk(bridge.checksum() != 0, "checksum!=0")
	fails += _chk(saw_bgm, "REQ_BGM 应到达分发器")
	var pp := bridge.player_pos()
	fails += _chk(pp.x >= -192.0 and pp.x <= 192.0 and pp.y >= 0.0 and pp.y <= 448.0, "player 在场界")
	# ② start=2 中段开机:垫片补偿电平追平(bgm 块内手写 2/bg 注入 1/bg_phase 手写 1)
	fails += _chk(_boot(2), "boot(start=2)")
	for i in 10:
		await get_tree().physics_frame
	var a := bridge.anchors()
	fails += _chk(int(a.get("bgm", -1)) == 2, "mid-start bgm==2, got %s" % str(a.get("bgm")))
	fails += _chk(int(a.get("bg", -1)) == 1, "mid-start bg==1")
	fails += _chk(int(a.get("bg_phase", -1)) == 1, "mid-start bg_phase==1")
	if fails == 0:
		print("SMOKE OK")
	get_tree().quit(0 if fails == 0 else 1)
```
注意:侦听用的 lambda 捕获局部 `saw_bgm` 在 GDScript 里对值类型不回写——实施时用成员变量或小数组 `var saw := [false]`;`saw[0] = true`。

- [ ] **Step 6: 跑真工程冒烟**

```bash
bash godot/smoke/run-smoke.sh
```
Expected: `SMOKE OK`。①段杂兵已出弹(240 帧内第一波俯冲+发弹),爆点/HUD 路径被动经过。

- [ ] **Step 7: 桥级冒烟销 B18 余量**——`crates/stg-godot/smoke/godot_smoke.ecl` 追加符卡可达面(T1 已把既有 `spawn_enemy` 迁到 7 参):

```
// B18 销账:enemy-owned 符卡(A5 解锁)→ hud_spell 判别 + fields_info 非空(符卡清弹 field)
async sub smoke_spell_pattern() { loop { wait(60); } }
async sub smoke_boss() {
    spell_begin(0, 7, smoke_spell_pattern, 3600, 50000, 0, 0);
    wait_spell();
}
```
main 里(mark(9) 之前)追加一行:`_ = spawn_enemy(64.0fx, -120.0fx, 8888, 0, 0, 2, smoke_boss);`

`smoke.gd` 断言区追加(S1 纪律:全非默认判别值;帧窗按实测微调——spell_begin 在任务首跑帧生效,清弹 field 生命短,逐帧扫窗口):

```gdscript
# B18 余量:hud_spell 判别断言(spell_id=7/bonus 非零)+ fields_info 非空至少一帧
var spell_seen := false
var field_seen := false
for i in 12:
	bridge.step_frame(0)
	var s := bridge.hud_spell(0)
	if not s.is_empty() and int(s.get("active", 0)) == 1 and int(s.get("spell_id", 0)) == 7 \
			and int(s.get("bonus_now", 0)) > 0 and int(s.get("frames_left", 0)) > 0:
		spell_seen = true
	if bridge.fields_info().size() > 0:
		field_seen = true
ok = ok and _assert(spell_seen, "hud_spell 判别(active/spell_id=7/bonus>0/frames_left>0)")
ok = ok and _assert(field_seen, "fields_info 非空(符卡清弹 field 可达)")
```
(`_assert` 用 smoke.gd 既有断言助手实名——机械调整许可。)

- [ ] **Step 8: 修 B22**——`crates/stg-godot/smoke/run-smoke.sh` 改成与 T3 新脚本同款纪律:

```bash
#!/usr/bin/env bash
# B22 修复:set -e + 命令替换会在 godot 非零退出时吞掉全部诊断——改为先落地再判定。
set -uo pipefail
cd "$(dirname "$0")"
GODOT_BIN="${GODOT_BIN:-/data/sunyunbo/playground/godot/Godot_v4.6.3-stable_linux.x86_64}"
( cd ../../.. && cargo build -p stg-godot ) || exit 1
"$GODOT_BIN" --headless --path . --import >/dev/null 2>&1 || true
out=$("$GODOT_BIN" --headless --path . --script res://smoke.gd 2>&1)
st=$?
echo "$out"
[ "$st" -eq 0 ] && grep -q "SMOKE OK" <<<"$out"
```

- [ ] **Step 9: 双冒烟 + 全测回归**

```bash
bash crates/stg-godot/smoke/run-smoke.sh
bash godot/smoke/run-smoke.sh
cargo test --workspace
```
Expected: 两个 `SMOKE OK` + 测试全 PASS。

- [ ] **Step 10: commit**

```bash
git add godot/ crates/stg-godot/smoke/
git commit -m "feat(godot): demo 局三文件 .ecl(杂兵+风铃卡 boss,A5/compile_units/mark 首个真实消费者)+双冒烟收口(B18 两断言/B22 修复)"
```

---

### Task 7: 文档收口

**Files:**
- Modify: `CLAUDE.md`(结构树 godot/ 行 + render-contract.md 行 + 常用命令补两条冒烟)
- Modify: `PROGRESS.md`(史加一行 + 重写「现在」段:M2 收口,余量与下一步)
- Modify: `docs/follow-ups.md`(A5 销账;B18 销余量;B22 销;B16①③④ 触发点复核追注;新债有则记)
- Modify: `docs/bridge-adaptation-notes.md`(本刀新坑追加,无坑则不动)
- Modify: `docs/ecl-lang.md`(手写节:spawn_enemy task 参/enemy_hp 用法一段——生成段 T1 已同步)

**Interfaces:**
- Consumes: T1-T6 全部落地事实(以 git log 与代码现状为准,不转录本计划)。

- [ ] **Step 1: CLAUDE.md**——仓库结构树加:

```
godot/            真 Godot 工程(场景刀):场景树/四层 MultiMesh/分发器/HUD/demo 局 .ecl
                  (渲染契约见 docs/render-contract.md;冒烟 godot/smoke/run-smoke.sh)
docs/render-contract.md   表现层契约权威(图集网格/stride 12/请求分发/锚点双表示)
```
常用命令区加:

```bash
bash crates/stg-godot/smoke/run-smoke.sh     # 桥级冒烟(桥面回归)
bash godot/smoke/run-smoke.sh                # 真工程冒烟(demo 两次开机)
```
M2 里程碑行改注:桥+真工程都已落地。

- [ ] **Step 2: PROGRESS.md**——史表加一行(一句话:A5 乙案+渲染契约收口+真 Godot 工程竖切+demo 局+双冒烟);「现在」段重写(位置=M2 收口;在飞=无;下一阶段候选=M3 环形快照/RL stg-py/A4 背景刀/内容与美术期;待办指 follow-ups)。

- [ ] **Step 3: follow-ups 对账**——A5 整条销(乙案+enemy_hp 落地,续裁定式留一句指 ecl-lang);B18 销余(hud_spell/fields_info 已断言;visible_instances headless 不可断言的注记保留为 B18 残句或并入 B16 组);B22 销;B16①③④ 逐条复核触发点是否到(到则做/没到就更新措辞);T1-T6 执行中新发现的债逐条新记。

- [ ] **Step 4: 终检 + commit**

```bash
cargo test --workspace && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings
cargo run -p stg-harness -- golden --out /tmp/golden-final.txt && diff .superpowers/godot-scene/golden-base.txt /tmp/golden-final.txt
git add -A && git commit -m "docs: 场景刀收口——CLAUDE.md 树/PROGRESS/follow-ups 对账(销 A5/B18 余/B22)+ecl-lang 手写节"
```
Expected: 全绿;golden diff 仍为空(全刀零平移)。

---

## 执行备注

- **任务序**:T1(纯 Rust)→ T2 → T3 → T4 → T5 → T6 → T7。T1/T2 无相互依赖但都在 T3 之前(T2 产 project.godot 占位与 assets,T3 覆写工程文件)。
- **实施者机械调整许可**:测试助手实名/断言帧窗/EnemyInit 全字段清单/smoke.gd 既有函数名,以文件现状为准,偏离记报告——先例见 frame.rs 头注释。
- **GDScript 无单测框架**:T3-T6 的测试环 = `--smoke` 断言集(每任务只增不减)+ 桥级冒烟回归;红绿循环照走(先加断言看它红,再实现让它绿——冒烟即测试)。
