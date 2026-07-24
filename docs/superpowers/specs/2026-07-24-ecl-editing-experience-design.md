# .ecl 编辑体验升级——VS Code 扩展 + 生成式规范文档 + check 诊断环

> **状态**:brainstorm 拍板(2026-07-24)。**实施挂起待令**("到时候做",spec 先存档)。
> 痛点拍板:①满屏一色(高亮)②盲写 builtin(补全/签名/hover)③**agent 可读规范**
> (合作者开 Claude Code 自助写/debug .ecl)。**不做**(本轮未选):存盘即诊断(LSP)、
> 热重载预览环——见 §6 升级位。目标编辑器:**VS Code 系**独占。

## 0. 方案裁决记录

- **方案 A(采纳)**:静态数据驱动扩展 + 生成式文档 + `check` 子命令。零常驻服务,
  单一真相源纪律贯穿,LSP 留平滑升级位。
- 方案 B(LSP 直上)否决:痛点未选"错误反馈慢",tower-lsp+客户端维护成本这轮 YAGNI;
- 方案 C(只文档+高亮)否决:盲写 builtin 只解一半,差一口气。

## 1. VS Code 扩展 `editors/vscode/stg-ecl/`

仓库内目录装载(不上市场;README 写 `ln -s` 进 `~/.vscode/extensions/` 或
`code --install-extension` 目录两法)。

```
editors/vscode/stg-ecl/
  package.json                   语言注册(.ecl,注释配置/括号对)+ 三 provider 贡献点
  syntaxes/ecl.tmLanguage.json   高亮文法
  ecl-meta.json                  【生成物,§2】builtin 元数据
  extension.js                   vanilla VS Code API,~100 行,零依赖
  README.md                      装载方式 + 再生成方式
```

**高亮范围**(tmLanguage):
- 关键字:以 `crates/stg-ecl-compiler/src/lang/lex.rs` 关键字实表为准(plan 期钉,
  §7.1)——`sub/async/spawn/const/loop/while/if/else/return/global` 一类语句词 +
  保留糖名(`wait_spell` 等,RESERVED_SUGAR_NAMES 同步);
- builtin 名:从 `ecl-meta.json` 生成进 pattern(§2 的生成器同时产这段);
- **数字字面量四后缀 `fx/px/deg/bam` 独立着色**——本语言第一易错点,视觉突出;
- 注释/字符串/运算符/const 名(大写惯例可用 pattern 近似)。

**extension.js 三 provider**,全部读 `ecl-meta.json` 零硬编码:
- CompletionProvider:builtin 名 + 参数占位 snippet(`fire(${1:appearance}, ...)`);
- SignatureHelpProvider:参名+类型渲染
  (如 `fire(appearance: int, x: fx, y: fx, speed: fx, angle: angle, xf: xform|none, task: sub|none) -> int`);
- HoverProvider:一句语义 + 坑注(如 emit_req:"void,只能裸语句,不可 `_ =`")。

## 2. 生成协议(单一真相源,烘焙表同款纪律)

- `Builtin` 结构(builtins.rs,编译器 crate,断层线之上)加 `doc: &'static str`
  字段:一句语义 + 坑注,与签名同源同处维护;
- harness 新子命令 **`gen-ecl-meta`**:从 builtins 表(name/params/ret/is_op/doc)产
  `editors/vscode/stg-ecl/ecl-meta.json`(含渲染好的签名字符串)+ 同步刷新
  `docs/ecl-lang.md` 的生成段(§3)与 tmLanguage 的 builtin pattern 段;
- **防漂移**:CI/测试断言"现生成 == commit 字节"(verify-tables 同款);生成段在文件内
  以显式标记包围(`<!-- gen:begin -->/<!-- gen:end -->` 类),手改必被抓。

## 3. `docs/ecl-lang.md` 重构(agent 优先)

结构(给 Claude Code 一次读全的密度):
1. **文法速查**:紧凑 EBNF 式全语法(语句/表达式/字面量后缀/三型);
2. **builtin 签名表**:生成段(§2 同一 JSON 渲染);
3. **坑清单**(全部实证过的):`fx/px` 后缀必写、`deg/bam` 角度后缀、有返回值必
   `_ =` 弃/void 必裸语句、`fire` 的 `none` 占位、**born_frame 出生帧不跑**(实体
   第 2 步才存在)、`global(GVAR_RANK)` 读难度、`spell_begin`+`wait_spell` 全套、
   drop_item 有随机喷发速度;
4. 最小可跑示例(自举 boss + 发环 + 符卡);
5. **debug 循环**:"改 → `cargo run -p stg-harness -- check f.ecl` → 行列错误 → 再改";
   合作者体验 = clone 仓库开 Claude Code,agent 读本文档即能写对/修对。
- `CLAUDE.md` 的 ecl-lang.md 入口行加"**agent 必读**"标注。

## 4. `stg-harness check <file.ecl>`

只编译不跑:`lang::compile(读盘文本, 文件名)` → 成功打 `OK`(退 0);失败打编译器
行列渲染错误原文(退 1);文件读不到退 2。人 / 合作者的 agent / 将来 CI 三方共用的
最短诊断环。

## 5. 测试与 DoD

- `gen-ecl-meta` 防漂移断言(现生成==commit);
- `check`:成功/编译错/文件缺失三路退码测试;
- tmLanguage/extension.js 无自动化(玩具级人肉视检;README 附一张判别用例清单:
  四后缀着色/builtin 补全弹出/hover 内容);
- **金向量逐字节不变**(全部改动在 harness 子命令/编译器 doc 字段/编辑器目录/文档,
  零 core 改动——doc 字段不进 EclImage 序列化,镜像 hash 不变,plan 期以
  compile 输出字节断言钉死);
- fmt/clippy/workspace 全绿。

## 6. 明确不做与升级位

- **LSP**(存盘即诊断/语义补全/跳转):升级位已留——ecl-meta.json 与扩展结构不变,
  将来把 provider 换 LSP client、加 tower-lsp 二进制即可;触发条件 = 用户/合作者
  实际抱怨错误反馈慢;
- **热重载预览环**(watch → 重编 → serve/Godot 重演):架构白送(`new_game` 吃源码
  文本),独立小刀随时可加;触发条件 = 弹型调参进入高频期(Godot 场景刀后大概率触发);
- JetBrains/Vim 封装:无需求不做。

## 7. plan 期钉死清单

1. lex.rs 关键字实表 + RESERVED_SUGAR_NAMES 全量(高亮词表);
2. `Builtin.doc` 字段落点(结构体加字段 vs 伴生表——以 builtins.rs 现有结构侵入最小者);
3. `ecl-meta.json` schema(字段名/签名渲染格式);
4. 生成段标记格式与 gen-ecl-meta 的幂等写入方式;
5. 镜像 hash 不受 doc 字段影响的断言写法(compile 前后字节对比);
6. package.json 的语言配置细节(注释符/括号对/自动闭合,对 lex.rs 实况)。
