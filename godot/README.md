# 真 Godot 工程 —— clone 下来怎么跑

> 这是 stg-engine 的表现层工程(场景刀,2026-07-26)。**仓库里没有编译产物**——`target/` 全被
> `.gitignore` 挡着,所以 clone 完必须先在本机编一次 Rust 动态库,Godot 才认得 `WorldBridge`。
> 渲染契约(图集网格/实例布局/请求分发/坐标系)见 [`../docs/render-contract.md`](../docs/render-contract.md)。

## 前置

| 需要 | 版本 | 说明 |
|---|---|---|
| Godot | **≥ 4.6**(开发用 4.6.3) | `stg_godot.gdextension` 写死 `compatibility_minimum = 4.6`,gdext 编译时也绑 `api-4-6`;4.5 装了也加载不了 |
| rustup | 任意 | 工具链版本由仓库根 `rust-toolchain.toml` 钉死(1.94.0),首次 `cargo` 命令自动拉,不用手动切 |
| C 链接器 | 见下 | Windows 要 MSVC(VS Build Tools 的 "Desktop development with C++",提供 `link.exe`);Linux 用系统 `cc` |

**不需要**装 LLVM/clang——gdext 用预生成绑定(锁文件里没有 `bindgen`/`clang-sys`),构建期也不需要
Godot 二进制在场。

## 三步跑起来

### Windows

```powershell
git clone <repo> && cd stg-engine
cargo build -p stg-godot                  # 产物 target\debug\stg_godot.dll
godot --path godot                        # 或:Godot 编辑器里 Import → 选 godot\project.godot
```

第 2 步产出的 DLL 必须落在 `target\debug\stg_godot.dll`——这正是 `stg_godot.gdextension` 里写的
路径。**别加 `--target x86_64-pc-windows-msvc`**:带了三元组,cargo 会把产物挪进
`target\x86_64-pc-windows-msvc\debug\`,Godot 就找不着了(那条路径只服务于 Linux 上的交叉编译验证)。

### Linux

```bash
git clone <repo> && cd stg-engine
cargo build -p stg-godot                  # 产物 target/debug/libstg_godot.so
godot --path godot                        # 无 X 环境见下面的 headless 冒烟
```

## 验证装配是否正确(headless,不需要显示器)

```bash
godot --headless --path godot -- --smoke      # Windows 同样可用(PowerShell/CMD 直接跑)
```

打出 `SMOKE OK` 且退出码 0 = 扩展加载成功 + 三层 MultiMesh 注册通过 + 敌人木偶喂料接上 + demo 局两次开机(正常/中段)
断言全绿。Linux 上另有一键脚本 `bash godot/smoke/run-smoke.sh`(自带 `cargo build`、首跑
`--import`、Godot 版本闸与超时;Windows 上用 Git Bash/WSL 才有 bash,或者直接跑上面那条原始命令)。

> **装了多个 Godot 时留神版本**:低于 4.6 的 Godot 不会加载扩展,而 `main.gd` 解析期就会因
> `WorldBridge` 不存在而报错——此时 `--headless` 进程**不会自己退出**,看起来像卡死而不是失败
> (坑档 G14)。脚本侧已用 `scripts/find-godot.sh` 加了 ≥4.6 版本闸 + `timeout` 兜底;手工跑
> 原始命令时请自己确认 `godot --version` ≥ 4.6。

首次运行会看到 Godot 重建导入缓存(`godot/.godot/`,已 gitignore);冷缓存下 `--import` 偶发
SIGABRT 是 Godot 自身的 bootstrap 脆弱点、与本扩展无关,不影响随后的真运行(详见
[`../docs/bridge-adaptation-notes.md`](../docs/bridge-adaptation-notes.md) G1)。

## 玩什么 / 操作

跑起来即进 demo 局:杂兵段 → 风铃卡 boss 战 → 挂牌结算。

| 键 | 动作 |
|---|---|
| 方向键 | 移动 |
| Z | 射击 |
| X | Bomb |
| Shift | 低速(同时显示判定点) |
| C | **観測**:按一下叠出「从此刻起什么都不做,30 帧后弹幕在哪」的暗色影子;60 tick 内再按一下 = **跳躍**(缺席跳过那 30 帧,瞬时) |
| V | **遡行**:被弹后的决死窗口(8 帧)内按,画面逐帧倒退回被弹前 30 帧,落地带 30 帧无敌 |
| D | 时间停止(临时键位;停止合并那刀再定) |
| Esc | 暂停(宿主停拍,世界时间线零帧) |
| Z(结算/GAME OVER 时) | 重开 |

关卡内容是 `ecl/demo/*.ecl` 三个文件(目录整取、按文件名排序编译);改完不用重编 Rust,重开
Godot 即可。语法手册见 [`../docs/ecl-lang.md`](../docs/ecl-lang.md),只编译不跑的快速检查:

```bash
cargo run -p stg-harness -- check godot/ecl/demo
```

## 扩展没加载怎么判断

症状是 `main.gd` 报 `WorldBridge` 不存在 / 开局失败。排查顺序:

1. 产物在不在:`target/debug/stg_godot.dll`(Windows)或 `target/debug/libstg_godot.so`(Linux);
2. 位置对不对:与 `stg_godot.gdextension` 的 `[libraries]` 逐字比对(路径相对 `godot/`,即
   `res://../target/...`);
3. Godot 版本 ≥ 4.6;
4. 删掉 `godot/.godot/` 让它重扫一遍。

## 有头目验:`--shots` 截图模式(2026-09-07 起)

没有显示器也能"看":`main.gd` 的 `--shots` 模式用脚本化输入跑 demo(常按射击、60–75 帧向左、
200 帧放 bomb),在若干帧把弹幕域存成 PNG(目录 `$STG_SHOTS_DIR`,默认 `user://shots`),首次敌死
后第 4 帧补一张,最后一张后自退;同时打印各层 `visible_instances` 真值。需要真渲染器——本开发机
走 VNC 桌面 + llvmpipe:

```bash
DISPLAY=:2 LIBGL_ALWAYS_SOFTWARE=1 MESA_LOADER_DRIVER_OVERRIDE=llvmpipe STG_SHOTS_DIR=/tmp/shots \
  godot --rendering-driver opengl3 --path godot -- --shots
```

有显示器的机器直接 `godot --path godot -- --shots` 即可。判读清单见
[`../docs/render-contract.md`](../docs/render-contract.md) §7/§8。
