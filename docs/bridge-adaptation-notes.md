# 外接适配注意事项(坑记录)

> **这是什么**:每次给 stg-core 接一个外部消费者(查看器/godot/py/net)踩到的坑与适配经验,
> 逐条严肃记录——**面向下一个接入者**(首要读者:M2 WorldBridge 的作者)。每条:坑 → 现场 →
> 对下一次接入的含义。持续追加,条目过时就删(同 follow-ups 无墓碑纪律)。
>
> 首批条目来自 **WS 查看器刀**(2026-07-23,通道 A/B 的第一个真实交互消费者,harness `serve`)。
> 第二批条目来自 **M2 stg-godot 桥刀**(2026-07-24,gdext 宿主 + Rust↔ECL↔GDScript 三层
> 对接的第一个真实消费者)。第三批条目来自 **Godot 场景刀**(2026-07-26,`godot/` 真工程——
> 场景树/渲染链/请求分发/HUD/demo 局,第一个跑完整可玩闭环的消费者)。

## 网络/传输层

### N1. 这台开发机的回环路径上有透明代理,会改 HTTP 包

`curl localhost` 的冒烟测试实测被注入 `Connection: keep-alive`/`Proxy-Connection` 头——
**本机 loopback 流量不是你发什么就到什么**。含义:
- 一切本地 HTTP/WS 调试工具(将来的编辑器桥、调试面板)都要按"包可能被改写/重分段"设防;
- 协议嗅探(HTTP vs WS 升级)不能信单次 `peek`——见 N2;
- 冒烟测试断言别写死头集合,只断言自己关心的行。

### N2. 协议嗅探必须读齐请求头再判(单发 peek 分包误判)

一次 `TcpStream::peek(1KB)` 判 `Upgrade: websocket` 在分包到达(代理重分段/慢客户端)时会
把 WS 握手误判成普通 HTTP,浏览器侧表现为 ws onerror,**且无任何服务端报错**——极难排查。
正解:有界重试 peek 直到看见 `\r\n\r\n`(头结束)或超时,再判。`serve` 的 `is_ws_upgrade`
即现成参考实现(含分包判别测试)。

### N3. `Content-Length` 是字节长不是字符长

内嵌中文 HTML 时 `str::len()`(字节)与字符数差出上百——用错浏览器会截断页面。
写 `as_bytes().len()`,冒烟断言用 `wc -c` 对账。

## 线格式/序列化

### S1. "判别值"纪律同样适用于序列化测试(圆心重合的新变种)

boss 段测试对**开局默认全零**的 `boss_ui` 逐字段断言——四个字段互换写序测试照绿
(全零怎么排列都是零),变异检验实锤。这是 CLAUDE.md M0-7"圆心重合"教训在**线格式测试**
里的重演:**编码测试的每个被断言字段必须先灌互异非零判别值**。凡新增 wire 段/新增字段,
测试先问一句:此刻这些字段的取值能区分错位吗?

### S2. 双端解码器同构靠纪律 + version 字节,别指望测试全兜

Rust `encode_frame` ↔ JS `parse()` 是手工同步的两份实现;审查手段 = 逐字段并排比对
(段序/宽度/符号性/端序:`getInt32` vs `getUint32` 之类)。version 字节把关大版本错配,
**同 commit 内同步改两端**是唯一纪律(测试只能盖 Rust 侧,JS 侧无 harness)。M2 gdext 是
同进程读切片、无线格式,此坑主要属查看器/py 远程形态。

## 接口消费体验(通道 A/B 排练情报,M2 直接受益)

### C1. 开箱好用、无需胶水的部分(实测确认)

- `view()` 裸切片 + `alive_words()` 位扫(`trailing_zeros` + `bits &= bits-1`)——批量
  消费一遍写成,与 `iter_alive()` 语义一致;A9"过滤是消费者义务"实践无摩擦。
- `take_requests()` 幂等非消费——编码器和测试可重复调,无"谁先取谁清空"陷阱。
- 输入零翻译:浏览器键盘 → u32 掩码**原位**就是 `BTN_*` 位布局 → `actions[0].buttons`
  直塞,无映射表。输入抽象(动作位非扫描码)的设计红利实锤。
- 定点→浮点只在消费端一处(`raw / 65536`),I1 边界干净。

### C2. 要自己搭、且下个接入者也会要的部分

- **场景搭建**:开局 bootstrap(建 World/装表/编脚本/摆 boss/start_main)原先埋在 golden
  流程里,本刀提成 `build_rainbow_world(seed)`(harness main.rs)。M2 WorldBridge 需要
  同类"开一局"入口——直接参考/复用该形态,别再各写各的。
- **节拍器**:核不管时间(I6),60Hz 定拍是消费者的活。参考实现(`serve`):首步即刻、
  落后连补 ≤3 步防死亡螺旋、超限重锚弃补。gdext 有引擎自己的帧回调,但"补帧上限+重锚"
  策略同样适用(显示帧率 ≠ 逻辑帧率时)。
- **表现层自有动画状态**:通道 B 请求是一次性事实,爆圈淡出等演出计时全在消费端
  (JS `effects` 列表);"核出请求,壳做演出"分工在真实消费者里成立,M2 分发器同构。

### C3. 已知限制(玩具级明知不修,真桥接时要认真对待)

- `serve` 的 `ws.send` 无写超时:客户端停止收流(后台标签页)会塞满发送缓冲、卡死节拍环。
  单客户端测试工具 = 杀进程重启;**真流式桥(py 远程/观战)必须有背压策略**。
- 单客户端串行、无 TLS、无断线续联——测试工具本分,不修。

## Godot 宿主运行时(headless bootstrap,M2 桥刀)

### G1. gdext 扩展存在时,冷缓存 `--import` 首跑可能在编辑器收尾阶段 SIGABRT——与扩展加载/注册无关

`godot --headless --path . --import` 首次冷启动(清空 `.godot/`)稳定退出 134(SIGABRT/core
dumped);担心的问题是"扩展是否根本没被正确加载注册",若是则后续 `--script` 冒烟判定也可能
假阳性。三重实验定位:①对照组(裸工程、无任何 `.gdextension`)跑同一条 `--import` 干净退出
0——排除"Godot 4.6.3 headless `--import` 通病",崩溃确定与本扩展被加载有关;②`gdb -batch
-ex run -ex bt` 抓栈,崩溃点在 `[ DONE ] loading_editor_layout` **之后**——即
`.gdextension` 解析/`.so` 加载/`entry_symbol` 调用/类注册早已顺利跑完并打出
`Initialize godot-rust (...)`,是在收尾阶段才炸;③冷/热缓存对比,连续跑三次 `--import`:
第 1 次(冷缓存)稳定崩溃,第 2、3 次(`.godot/` 已由第 1 次崩溃前的工作写盘)稳定退出
0——即扩展清单/类缓存等落盘工作在崩溃发生前已完成,崩溃是编辑器 bootstrap 收尾阶段自身的
既有脆弱点,与"扩展是否被正确加载"完全正交。**对下一次接入的含义**:CI/自动化脚本比照
`crates/stg-godot/smoke/run-smoke.sh` 把首跑 `--import` 处理为非致命步骤(`|| true`
兜底、退出码不参与判定),真判定点是随后的 `--script res://smoke.gd`;顺手一提,核验
`entry_symbol` 是否与 `.gdextension` 里写的名字一致,直接 `nm -D target/.../lib*.so |
grep -i init` 读产物真实导出符号,比对着简报字面猜可靠。

### G2. `.uid` sidecar 文件是 Godot 4.4+ 的资源引用机制,应随源码一并入库,不当缓存清

`smoke/smoke.gd.uid`、`smoke/stg_godot.gdextension.uid` 由 `--import` 自然产生,简报未提及。
这是 Godot 4.4+ 起为每个被扫描到的资源(含 `.gd` 脚本、`.gdextension` 文件)自动生成的 UID
旁车文件(`uid://...` 一行,极小),官方约定要提交进版本库以稳定资源引用——类比 `.import`
文件的角色,不属于可重新生成的临时缓存(`.godot/` 才是,已 `.gitignore`)。**对下一次接入的
含义**:目录级 `git add` 会自然带上这些 `.uid` 文件,判为预期内、应保留,不必额外清理;真
要清理的只有 `.godot/`。

## gdext 0.5.4 API 适配点(实测调整,非简报预判)

### G3. `Dictionary` 已泛型化——裸 `Dictionary`/`Array<Dictionary>` 编译不过

0.5.4 把 `Dictionary` 改成了泛型 `Dictionary<K: Element, V: Element>`(4.4+ 起支持编辑器
可见的强类型字典),旧版(0.4.x)风格未带类型参数的裸 `Dictionary::new()`/`-> Dictionary`/
`Array<Dictionary>` 编译不过。**对下一次接入的含义**:换成 crate 自带的未类型化别名
**`VarDictionary`**(`= Dictionary<Variant, Variant>`,`godot::prelude::*` 已重导出,无需
额外 `use`);GDScript 侧观感不变(`VarDictionary` 编译期擦除后仍是 GDScript 看到的普通
`Dictionary`)。

### G4. 非 `Copy` 的 builtin 容器类型走 `AsArg` 是按引用传参的调用约定

`d.set("args", args)` 里 `args: Array<i64>` 传给 `Dictionary::set` 的 `impl AsArg<V>` 时,
编译期报 `<Array<i64> as ToGodot>::Pass == ByValue` 不满足(期望 `ByRef`)。**对下一次接入
的含义**:`Array`/`Dictionary`/`GString` 等非 `Copy` builtin 容器类型传给 `AsArg` 形参一律
按引用传(`d.set("args", &args)`),别假设值语义;`i32`/`f32`/`bool` 等 `Copy` 标量不受影响。

## 桥两侧类型/接口对接(Rust 桥层 ↔ ECL 脚本 ↔ GDScript)

### G5. 动作位常量以 `stg-core` 代码实名为准,别照设计文档口头名字猜

`stg_core::input::BTN_FOCUS` 不存在——`crates/stg-core/src/input.rs` 的 `define_actions!`
词表里第七个动作位(低速)实名 `BTN_SLOW`(位 6),没有 `BTN_FOCUS` 这个名字,常量本身的值/
位号未变。**对下一次接入的含义**:桥层暴露给 GDScript 的每个 `#[constant]` 常量名,写代码
前先 grep `stg-core` 的实际定义处,别按设计文档/简报口头叫法直接编,同类坑此前已见于
`PlayerState.life_state`(见"杂项"节)。

### G6. ECL 内建函数的 void 返回值不能 `_ =` 丢弃,只能当裸语句

`emit_req` 等 `ret: None` 的内建(`builtins.rs` 表项)编译期强制"无值可丢弃"
(`typeck` 断言 `_ = pulse_signal(0)` 这类是编译错误),`docs/ecl-lang.md` 也明写"只能做
语句"。**对下一次接入的含义**:给桥写胶水 `.ecl` 脚本时,调用通道 B/void 类内建一律裸语句
(`emit_req(64, 1, 2, 3, 4, 5, 6);`),别按"每次调用都赋值/弃值"的惯性加 `_ =` 前缀。

### G7. ECL `fire` 是 7 参、角度参必须走单位字面量后缀,不接受裸整数/裸数量

`fire` 签名是 `(appearance:Int, x:Fx, y:Fx, speed:Fx, angle:Angle, xf:XformRef,
task:SubRef)`——常见的踩坑设想是 8 参(把 `xf`/`task` 拆成"偏移+计数"两个整数)且角度位传
裸整数;实际 `xf`/`task` 是编译期解析的标识符(xformdef 名/async sub 名,或字面量
`none`),`angle` 形参类型是 `Ty::Angle`,裸整数字面量(`Ty::Int`)编译期类型不匹配。
**对下一次接入的含义**:桥用的胶水脚本里 `fire(...)` 传参数固定 7 个,角度位一律带单位后缀
(`0deg`/`16384bam`),不挂变换/子任务时两个标识符位填字面量 `none, none`——生成/审阅胶水
`.ecl` 时对照 `crates/stg-ecl-compiler/src/lang/builtins.rs` 的真实签名表,比凭空写参数列表
可靠。

## 真 Godot 工程(场景刀,2026-07-26;第三批消费者——`godot/` 真工程 + demo 局)

### G8. `physics_frame` 信号在 `_physics_process` 之前触发——数信号次数≠数 step 次数

冒烟等帧最初设想"数 `await get_tree().physics_frame` 触发几次就等于 `step_frame` 调了几次"
——实测(Godot 4.6.3)`physics_frame` 信号在每 tick 的 `_physics_process`(`step_frame` 调用点)
**之前**触发,数信号次数会比真实 step 次数差一帧,不代表"每两 tick 一 step"这类回归会被
正确捕捉。**对下一次接入的含义**:等帧断言一律轮询实际读口(本仓是 `bridge.frame()`)而非
数信号触发次数;轮次上限给目标值的合理余量防死等,同时另加一条"实际轮次 ≤ 目标+1"断言,
防止宽松的上限把"每两 tick 一 step"这类节奏回归悄悄放过(`main.gd::_wait_frame` 现成参考)。

### G9. `MultiMesh.visible_instance_count`(资源字段)与 `RenderingServer.multimesh_get_visible_instances`(服务端真值)双双不可 headless 断言

桥面固定走 `RenderingServer.multimesh_set_visible_instances(rid, n)` 直写服务端,从不经
`MultiMesh` 资源对象自身的 setter——资源侧 `visible_instance_count` 字段因此永远停在
`playfield.gd::_make_layer` 播种的初值(0),这是纯粹的客户端/服务端字段分裂,与渲染后端
无关,真机同样成立。退一步改走服务端真值 `RenderingServer.multimesh_get_visible_instances`,
headless dummy renderer 下**同样实测恒 0**(与 G1/register_layer 那条"仅 `set_buffer` 后
`get_buffer` 完整往返"是两回事,可见数这条指标 headless 下彻底不可读)。**对下一次接入的
含义**:可见数判据只能靠有 GPU/有头环境验证;headless 冒烟改走 `multimesh_get_buffer` 回读
实际写入的实例数据(位置/旋转/自定义位)做判别,不断言可见数,见 `docs/follow-ups.md` B18。

### G10. headless dummy renderer 只跑完整 shader 前端——编译错抓得到,数据通路问题抓不到

`layer.gdshader`(`canvas_item`,读 `INSTANCE_CUSTOM.x` 选图集格)在 dummy renderer 下三轮
冒烟稳定零 shader 编译错误/`push_error`——但这只证明**语法**过了 GLSL 前端(词法/类型检查/
uniform 声明等),不证明**运行期数据通路**(`INSTANCE_CUSTOM` 实际取值→UV 采样→像素输出)
正确,因为 dummy renderer 根本不做光栅化。`docs/follow-ups.md` B23(UV 垂直朝向镜像嫌疑)
正是这一类问题的实例——CPU 侧 `QuadMesh.get_mesh_arrays()` 能读顶点/UV 静态配对,但配对
在真管线里是否如实生效(NDC/视口变换等中间环节可能已抵消)必须真渲染器出图才能判。
**对下一次接入的含义**:shader 冒烟绿只代表"没写错语法",数据通路/视觉正确性类问题一律
留给首个有 GPU/X 环境判决,别把"编译期零报错"读成"运行期零问题"。

### G11. `DirAccess.open` 对不存在的目录静默返回 `null`,不打任何 stderr

`main.gd::_boot` 用 `DirAccess.open("res://ecl/demo")` 读关卡目录——指向不存在路径时该调用
**不产生任何引擎侧警告/错误输出**,只是返回值为 `null`,与部分资源加载 API(如
`ResourceLoader.load` 失败会自己打 error)的行为不对称。**对下一次接入的含义**:任何用
`DirAccess`/`FileAccess` 读外部内容(关卡包/mod/存档目录)的宿主代码,必须自己判 `null` 并
主动 `push_error`,不能指望引擎替你发现路径写错这类低级问题——静默失败会一路传导到更远
处才炸(比如"目录读到但文件列表为空"这类更难定位的次生故障)。

### G12. `SubViewportContainer.stretch=true` 若被放进布局容器,会连带改写内部 `SubViewport` 的尺寸

`Playfield`(`extends SubViewportContainer`,`stretch=true`)内部 `SubViewport` 固定
384×448、世界根 `Node2D@(192,0)`——这组尺寸/偏移是渲染契约的坐标系基准(§6)。若这个容器
被塞进任何会重新分配子节点尺寸的布局容器(`VBoxContainer`/`HBoxContainer`/带
`size_flags_*` 拉伸的 `Container` 等),布局系统会覆写 `SubViewportContainer` 自身尺寸,
`stretch=true` 又会把这个被覆写的尺寸继续传给内部 `SubViewport`——最终 384×448 这个基准
悄悄跑掉,`world_root` 的坐标系跟着整体错位,而这类问题在编辑器里往往不报错,只是画面对不上。
本工程现状**未踩中**(`Playfield` 直接 `add_child` 在 `Main`——一个裸 `Node`,不是布局
`Container`;右栏 HUD 走独立的 `Hud extends CanvasLayer`,`CanvasLayer` 子节点用绝对定位、
天然不参与任何父级的 `Container` 布局流程,两边都绕开了这个坑)。**对下一次接入的含义**:
以后若要加菜单/选关等需要把 `Playfield` 摆进某个自适应布局的场景,想清楚这条尺寸传导链;
纯 HUD 类叠加层继续走 `CanvasLayer` 绝对定位是更省心的默认选择。

### G13. `.gdextension` 的库路径必须匹配 **cargo 原生产物布局**——带 `--target` 三元组编译会把产物挪走

cargo 的产物落点有两套:`cargo build` 落 `target/{debug,release}/`,而 `cargo build --target
<三元组>` 落 `target/<三元组>/{debug,release}/`。`.gdextension` 的 `[libraries]` 每个平台键
只能写**一条**路径(feature-tag 组合不可重复,没有回退列表),所以两套布局只能认一套。本工程
(2026-07-26 工具链刀)统一认**原生布局**:Windows 上 `cargo build -p stg-godot` →
`target\debug\stg_godot.dll`,Linux 上 → `target/debug/libstg_godot.so`。**对下一次接入的含义**:
①异机 clone 后按 `godot/README.md` 走,Windows 上**别**加 `--target x86_64-pc-windows-msvc`
——加了产物进三元组目录,Godot 静默找不到库,表现是 `WorldBridge` 类不存在(而不是"库加载失败"
这种指向明确的报错),排查成本远高于起因;②Linux 上用 cargo-xwin 交叉出 Windows DLL 那条路
(见用户全局笔记)只用于**验证能编过**,产物在三元组目录、不被本表认领,要真让 Godot 加载须自行
拷进 `target/debug/`;③交叉编译验证仍是划算的——本刀实测 `cargo-xwin build -p stg-godot
--target x86_64-pc-windows-msvc` 在 Linux 上产出 PE32+ DLL 且 `llvm-objdump -p` 导出表里有
`gdext_rust_init`(与 `entry_symbol` 一致),等于在没有 Windows 机器的情况下证明了整条
core+compiler+gdext 链在 MSVC 目标下编得过、符号对得上。注意钉死的工具链(`rust-toolchain.toml`
1.94.0)需要 `rustup target add x86_64-pc-windows-msvc` 单独装标准库,否则报 `can't find crate
for std`——这不是代码问题。

### G14. 扩展加载失败时 headless Godot **永不退出**——冒烟表现为挂死而非报错,故必须有版本闸 + 超时

`.gdextension` 的 `compatibility_minimum = 4.6`:低于该版本的 Godot **不加载**本扩展,于是
`WorldBridge` 类不存在,`main.gd`/`smoke.gd` 在解析期就报 `Identifier "WorldBridge" not declared`
——脚本根本没跑到那句 `quit()`,而 `--headless` 又没有窗口可关,进程就**一直挂着**。实测
(2026-07-26 工具链刀):本机 PATH 上的 `godot` 是 4.5.1、开发钉的是 4.6.3;冒烟脚本一度改成
"优先取 PATH 里的 godot",当场挂死 18 分钟、输出为空,从现象上完全看不出是版本问题。
**对下一次接入的含义**:①任何"自动发现 Godot 二进制"的逻辑都要带**版本闸**,别只判存在
(本仓的闸在 `scripts/find-godot.sh`,自动候选须 ≥4.6,显式 `GODOT_BIN` 只警告不否决);
②每条 godot 调用都要 `timeout` 兜底,把"无限等"降级成"可诊断的失败"——挂死比失败难查得多,
因为它连一行错误都不给;③反过来,这也是一条**极好的负控**:临时喂个旧版 Godot 就能验证你的
冒烟脚本在扩展失效时是否真的会红(本刀实测:退出码 1 + 打印超时诊断,而非静默绿或永远转)。

## 杂项

- `PlayerState` 生死字段实名 `life_state`(非设计文档口头的 life);写外接层前先 grep 实名。
- tungstenite 0.30 与 0.24 教程级 API 兼容,`Message::Binary` 载荷 `Vec<u8>/Bytes` 用
  `.into()` 兜。

## G15. 冒烟里"自机打不中靶"的两连坑（命中事件刀，2026-07-27）

给 `frame_events` 写非空断言时连红两次，两次都不是读口的问题：

1. **靶敌在自机弹的越界回收线之外**。`godot_smoke.ecl` 原有两只敌在 `y=-160`/`-120`，
   而自机弹的回收判据是 `y ∈ [-64, 512]`（`cleanup.rs::out_of_bounds`，`OOB_MARGIN=64`）
   ——弹飞到 -64 就被收了，永远够不着。补了第三只在 `y=200`（场内）当靶子。
2. **自机不在靶敌那一列**。冒烟在存档/重演段按了 30 帧 `BTN_LEFT`，自机漂到 `x≈-135`，
   而靶敌在 `x=0`。补 30 帧 `BTN_RIGHT` 送回（左右速度与钳制对称，正好回 `x≈0`）。

**教训**：桥级冒烟里"驱动输入 → 期待世界产生某事实"这类断言，得同时算准**空间可达性**
（回收线）与**自机当时在哪**（前面的输入是有累积效果的）。定位方式是先在 `stg-core` 写一条
等价的纯 Rust 端到端测试（`settle.rs::player_holding_shot_hits_enemy_in_its_column`），
它一过就说明模拟层没问题、锅在冒烟。
