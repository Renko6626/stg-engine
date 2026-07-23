# WebSocket 实时查看器(轻量测试接口)设计

> **一句话**:`stg-harness serve` 子命令——起单端口 TCP 服务,HTTP 请求回内嵌 HTML(canvas
> 渲染器),WebSocket 升级则进 60Hz 游戏循环:浏览器键盘 → `InputFrame` → `step` → 通道 A/B
> 二进制快照推流 → canvas 绘制。**stg-core 零改动**;通道 A/B 的第一个真实交互消费者,
> 兼作 M2 WorldBridge 的接口排练。

**目标**:第一次"看见并玩到"引擎(彩虹风铃卡);验证双通道契约在真实消费者手里的姿势;
`ssh -L` 端口转发即用,契合 headless 开发机。**测试性工具,非 M2 交付物**——不追求手感
(WS 往返延迟存在),追求"弹幕形状/机制行为肉眼可验"。

**归属**:M2 前的独立工具刀(用户拍板 2026-07-23);不动 M2 排期。

---

## 1. 架构与划界

- **住 harness**(`serve` 子命令 + `crates/stg-harness/viewer/index.html` 内嵌),**不建新
  crate**(宪法:Phase 1 只建三 crate + derive)。断层线以上,允许 std::time/线程/浮点。
- **新依赖仅 `tungstenite`**(同步 WS,纯 Rust)——加在 stg-harness(断层线上);
  `cargo tree -p stg-core` 防火墙不受影响;Cargo.lock 照常 commit。
- **stg-core 零改动**:数据全部经既有出口——`view()`(五池+players)、`take_requests()`、
  `frame()`、`boss_ui`(pub 字段)。金向量不相关(golden 路径仅做纯提取重构,见 §4)。
- 单客户端、串行服务:accept 循环一次伺候一个连接,**连接断开/刷新页面 = 下一局新 World**
  (重开即重置,天然 restart)。

## 2. 服务与协议

### 2.1 单端口双协议

`serve [--port 8611] [--seed 1]`。accept 后 `TcpStream::peek` 读请求头(≤1KB):
含 `Upgrade: websocket`(不区分大小写)→ `tungstenite::accept(stream)` 进游戏循环;
否则手写 `HTTP/1.1 200 OK` + `Content-Type: text/html` 回内嵌页面
(`include_str!("../viewer/index.html")`)后关连接。用法:
`ssh -L 8611:localhost:8611 <box>` → 浏览器开 `http://localhost:8611`。

### 2.2 游戏循环(60Hz)

- 场景 = 彩虹风铃卡:从 `cmd_golden` 场景 2 **提取** `build_rainbow_world(seed) ->
  (Box<World>, EclImage)` 复用(boss 敌 + `start_main_with_owner` + RANK global 同款);
  golden 调同一 helper(纯提取重构,golden 逐位不变为证)。
- 每 tick:排空 WS 待读消息取**最新**输入掩码(底层 stream 设 1ms 读超时,`WouldBlock`
  即止;文本/ping 忽略,4 字节二进制 = u32 LE 掩码)→ 构造 `InputFrame`(玩家 0,掩码
  直接就是 `BTN_*` 位布局,见 §2.4)→ `step` → 编码快照(§2.3)→ 二进制 WS 消息发出。
- 节拍:`Instant` 累积器,60Hz 固定步;落后则连补(上限 3 步/tick 防螺旋),超前则 sleep。
- 客户端断开(发送/读取 Err)→ 回 accept 循环。

### 2.3 快照线格式(v1,小端,无对齐填充,偏移即字节序拼接)

```
u8  version = 1
u32 frame
u8  player_count            # = MAX_PLAYERS 实际在场数,v1 恒 1 个记录:玩家 0
  per player: i32 x, i32 y, u8 life, u16 invuln
u8  boss_slots = 2
  per slot: u8 active, i32 hp_ratio_raw(Fx), u16 spell_id, u16 timer_frames
u16 bullet_count   → per: i32 x, i32 y, u16 sprite          (10 B)
u16 shot_count     → per: i32 x, i32 y, u16 sprite          (10 B)
u16 enemy_count    → per: i32 x, i32 y, u16 sprite, u8 hp_pct(0-255)  (11 B)
u16 item_count     → per: i32 x, i32 y, u8 item_type        (9 B)
u16 req_count      → per: u16 id, u16 seq, i32 args[6]      (28 B)
```

- 各池**只发存活槽**(扫 `alive_words()` 位字——批量消费姿势,A9"过滤是消费者义务"的
  第一次真实践行);计数天然 ≤ cap ≤ 8192 < u16::MAX。
- 坐标 Q16.16 raw 原样过线(I1 纪律:定点→浮点转换发生在 JS,`raw / 65536`——恰是
  design_doc §6.4 说的"转换只发生在这条边界上")。
- `hp_pct = (hp.max(0) * 255 / hp_max.max(1)).min(255) as u8`(表现层色调用)。
- **协议版本纪律**:version 字节把关;改布局 = version+1 + 同步 HTML 解码器(同仓同 commit,
  无兼容包袱——这是测试工具不是外部契约,**不入半冻结面**)。

### 2.4 输入(浏览器 → 服务)

4 字节 LE u32,位布局**直接采用** `stg_core::input` 的 `BTN_*`:UP=bit0 / DOWN=1 / LEFT=2
/ RIGHT=3 / SHOT=4 / BOMB=5 / SLOW=6。键映:方向键 + Z=SHOT + X=BOMB + Shift=SLOW
(ZUN 惯例)。keydown/keyup 维护掩码,变化即发 + 1s 心跳重发;服务端 tick 取最新值填
`InputFrame`(BOMB 是 Edge 语义,由引擎侧 `EDGE_MASK` 自理,查看器只送电平)。

## 3. 页面(canvas 渲染器,单文件内嵌)

`crates/stg-harness/viewer/index.html`,零外部资源(CSP 无关,纯本地):

- **场地**:canvas 逻辑 384×448(D7 中轴系:`px = x/65536 + 192`,`py = y/65536`),
  整数倍缩放适配窗口,场外留 HUD 侧栏。
- **绘制**(每 WS 消息一帧,`requestAnimationFrame` 节流):弹=小圆(色相 = sprite 值
  哈希到色轮);自机弹=细长白点;敌=大圆(HP 色调 `hp_pct` 红→绿);道具=小方块(色 =
  item_type 查小色表);自机=白心三角 + SLOW 按下时显示 hit_radius 判定点(东方惯例)。
- **通道 B 消费**:`REQ_ENEMY_DEATH`(id=1)→ 在 args[0..1] 位置起一圈扩散淡出圆环
  (寿命 ~20 帧,表现层自己的动画状态——"核出请求,壳做演出"第一次真跑通);其余 id
  console.log(留给未来 id 词汇)。
- **HUD**:帧号、各池活跃计数、boss 血条(hp_ratio_raw/65536)+ spell/timer、WS 往返
  估计(输入回显延迟)、连接状态;断线显示"刷新重开一局"。
- **无外部字体/库**;`<title>stg-engine viewer</title>`。

## 4. 确定性与金向量论证

- stg-core 零改动;harness 的 golden 路径只做 `build_rainbow_world` **纯提取重构**
  (代码搬家,行为零变)⇒ **金向量与基线逐位全等**(回归闸)。
- serve 模式本身是交互式、非确定性的(取决于人手)——这是查看器的本分,不进任何对拍。
- 新依赖只落 harness;`cargo tree -p stg-core` 防火墙照旧。

## 5. 测试策略

1. **编码器判别式单测**(harness):搭已知世界(定点坐标/若干弹/敌/道具/req)→
   `encode_frame` → Rust 侧按 §2.3 手工解码,逐字段断言(x/y raw 值、计数、hp_pct、
   req args 逐位;判别值防错位)。
2. **输入映射单测**:掩码 → `InputFrame` → 断言 `actions[0].buttons` 原样;超界位不清洗
   (引擎侧 `decode_input` 自有纪律)。
3. **金向量逐位不变**(提取重构的回归闸)。
4. **HTTP/WS 分流冒烟**:单测起端口 → 裸 TCP 发 `GET /` 断言回 200+HTML 片段;
   (WS 全链路留人工验收——浏览器开局玩一把,标准见 §6)。
5. 全绿 + fmt + clippy + 防火墙。

## 6. 人工验收标准(合并前用户过目)

`cargo run -p stg-harness -- serve` + 本地浏览器:能看到风铃卡开打(弹环扩散、boss 血条
走、timer 跳),方向键移动、Z 射击杀敌见爆圈、Shift 显判定点,刷新重开一局。

## 7. 收尾

- `README.md`/`CLAUDE.md` 常用命令区加一行 `serve`;`docs/architecture.md` harness 行提一句。
- `PROGRESS.md` 史加一行 + 「现在」段。
- follow-ups:记一条「查看器泛化(--ecl 任意脚本预览 / 回放文件播放)」的将来扩展点
  (触发点 = .ecl 创作流真开动 / M3 回放调试)。
