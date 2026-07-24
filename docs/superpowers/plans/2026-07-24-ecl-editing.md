# .ecl 编辑体验刀 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** VS Code 高亮/补全/签名/hover + `builtins.rs` 单一真相源生成协议 + agent 优先文档 + `check` 诊断子命令。

**Architecture:** `Builtin` 表加 `doc`/`param_names` 成唯一元数据源;harness `gen-ecl-meta` 产 `ecl-meta.json`(手写确定性 JSON,无 serde)并刷新文档生成段,CI 防漂移;扩展是零依赖 vanilla provider;`check` 只编译不跑。

**Tech Stack:** Rust 1.94(现状),VS Code 扩展 API(纯 JS 零 npm 依赖),tmLanguage JSON。

**Spec:** `docs/superpowers/specs/2026-07-24-ecl-editing-experience-design.md`。

## Global Constraints

- **零 core 改动**:动的只有 stg-ecl-compiler(仅 `builtins.rs`)/stg-harness(新子命令+测试)/`editors/`/docs——**金向量逐字节不变**(Task 1 Step 1 抓基线,Task 5 终拍全等)。
- **无新 crate 依赖**(JSON 手写 format!,顺序=表序,确定性字节)。
- 生成物(`ecl-meta.json`、文档生成段)commit 入库 + 防漂移测试(现生成==commit,verify-tables 同款)。
- **事实转录纪律**:凡"从 X 转录"的步骤(关键字表/参数名/param 语义),以命名的源码位置为准逐字核对;plan 给的草稿表若与源码出入,**以源码为准修正并入报告**,不许照抄错的。
- 分支 `feat/ecl-editing`;commit 尾附 `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`。
- 跑绿:`cargo test --workspace` + fmt --check + clippy -D warnings。

---

### Task 1: `stg-harness check <file.ecl>` 子命令

**Files:**
- Modify: `crates/stg-harness/src/main.rs`(dispatch match 加臂 + `cmd_check` + usage 行 + tests)

**Interfaces:**
- Produces: `check` 子命令——成功打 `OK` 退 0;编译错打渲染错误退 1;文件读不到退 2。(Task 5 文档的 debug 循环引用它。)

- [ ] **Step 1: 建分支 + 金向量基线**

```bash
cd /data/sunyunbo/www/stg-engine
git checkout -b feat/ecl-editing
mkdir -p .superpowers/eclediting
cargo run --release -p stg-harness -- golden --out .superpowers/eclediting/golden-base.txt
```

- [ ] **Step 2: 写失败测试**(main.rs tests 模块)

```rust
    /// check 三路退码:OK=0 / 编译错=1(带行列) / 文件缺失=2。
    #[test]
    fn check_exit_codes_three_ways() {
        use std::process::ExitCode;
        let dir = std::env::temp_dir().join("ecl-check-test");
        std::fs::create_dir_all(&dir).unwrap();
        let ok = dir.join("ok.ecl");
        std::fs::write(&ok, "sub main() { loop { wait(60); } }").unwrap();
        let bad = dir.join("bad.ecl");
        std::fs::write(&bad, "sub main() { 这不是脚本 }").unwrap();
        assert_eq!(cmd_check(&[ok.to_string_lossy().into_owned()]), ExitCode::SUCCESS);
        assert_eq!(cmd_check(&[bad.to_string_lossy().into_owned()]), ExitCode::from(1));
        assert_eq!(
            cmd_check(&[dir.join("nope.ecl").to_string_lossy().into_owned()]),
            ExitCode::from(2)
        );
    }
```

- [ ] **Step 3: 红**——`cargo test -p stg-harness check_exit 2>&1 | head -5`,期望 `cmd_check` 未定义编译错。

- [ ] **Step 4: 实现**(依既有 cmd_* 形态;dispatch 加 `Some("check") => cmd_check(&args[2..]),`,usage 字符串加 `check <file.ecl>`)

```rust
/// 只编译不跑(编辑体验刀 spec §4):人 / 合作者 agent / CI 共用的最短诊断环。
fn cmd_check(rest: &[String]) -> ExitCode {
    let Some(path) = rest.first() else {
        eprintln!("usage: stg-harness check <file.ecl>");
        return ExitCode::from(2);
    };
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: 读 {path} 失败: {e}");
            return ExitCode::from(2);
        }
    };
    match stg_ecl_compiler::lang::compile(&src, path) {
        Ok(_) => {
            println!("OK");
            ExitCode::SUCCESS
        }
        Err(errors) => {
            for e in &errors {
                eprintln!("{}", e.render(path));
            }
            ExitCode::from(1)
        }
    }
}
```

- [ ] **Step 5: 绿 + commit**

```bash
cargo test -p stg-harness check_exit && cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add crates/stg-harness/src/main.rs
git commit -m "feat(harness): check 子命令——只编译不跑的最短诊断环(OK/编译错/文件缺失三路退码)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: `Builtin` 加 `doc`/`param_names` + `pub fn all()`

**Files:**
- Modify: `crates/stg-ecl-compiler/src/lang/builtins.rs`

**Interfaces:**
- Produces: `Builtin { name, syscall, is_op, params, ret, doc: &'static str, param_names: &'static [&'static str] }`;`pub fn all() -> &'static [Builtin]`。(Task 3 生成器消费。)

- [ ] **Step 1: 写失败测试**(builtins.rs tests 模块)

```rust
    /// 编辑体验刀:元数据完备——每条 builtin 有非空 doc,param_names 与 params 等长。
    #[test]
    fn all_builtins_have_doc_and_matching_param_names() {
        for b in all() {
            assert!(!b.doc.is_empty(), "{} 缺 doc", b.name);
            assert_eq!(
                b.param_names.len(),
                b.params.len(),
                "{} 参数名/参数型不等长",
                b.name
            );
        }
    }
```

- [ ] **Step 2: 红**——`cargo test -p stg-ecl-compiler all_builtins_have_doc 2>&1 | head -5`。

- [ ] **Step 3: 实现**——struct 加两字段 + `pub fn all() -> &'static [Builtin] { BUILTINS }`;26 条目逐条填。**doc/param_names 草稿表如下,每条以 `syscall.rs` 对应 `sys_*` 函数的 pop 逆序为准核对后落**(出入以源码为准并入报告;`batch` 的参数名整表从 `sys_create_bullets_batch` 转录):

| name | param_names(草稿) | doc(草稿) |
|---|---|---|
| fire | appearance,x,y,speed,angle,xf,task | 发一颗弹;appearance 查外观表(越界 Fault);xf/task 为 xformdef/sub 名或 none;返弹句柄,失败 -1 |
| batch | (从 sys_create_bullets_batch pop 逆序转录) | N-way 批量发环;返实际创建数 |
| spawn_enemy | x,y,hp,drop_table,score | 造敌;sprite 固定 0、判定 12/16 默认;返敌句柄 |
| drop_item | x,y,item_type | 掉一颗道具(带随机喷发速度,消耗模拟 RNG);返句柄 |
| move_to | enemy,x,y,dur | 敌 easing 平移到 (x,y),dur 帧 |
| boss_set | enemy,hp_ratio,spell_id,timer_frames,phase_left,active | 整槽写 boss_ui 公告板(脚本写/UI 读;符卡 active 期 hp_ratio 由引擎自动喂) |
| pulse_signal | channel | 脉冲信号,唤醒 wait_signal 中的弹任务 |
| emit_req | id,a0,a1,a2,a3,a4,a5 | 通道 B 渲染请求;void 只能裸语句;args 裸载荷(fx 过 raw/angle 过 BAM/int 原样) |
| rand | n | 模拟 RNG 均匀 [0,n);确定性,随快照回卷 |
| global | slot | 读 globals 槽(GVAR_RANK=0 为难度) |
| set_global | slot,value | 写 globals 槽 |
| aim_player | (空) | 自身(敌/弹属主)指向自机的 BAM 角 |
| sin / cos | angle | 查表三角,返 fx(VM op 直发,非 syscall) |
| set_speed | handle,speed | 弹 setter:改速率(坏句柄 no-op+计数,下同) |
| set_angle | handle,angle | 弹 setter:改方向 |
| turn | handle,delta | 弹 setter:转向增量 |
| set_vel | handle,vx,vy | 弹 setter:直设速度向量 |
| set_ang_vel | handle,w | 弹 setter:角速度(POLAR_FX) |
| set_accel | handle,a | 弹 setter:切向加速度(POLAR_FX) |
| set_gravity | handle,gx,gy | 弹 setter:直角加速度(CART_FX) |
| stop_fx | handle | 弹 setter:停连续效果 |
| aim_at_player | handle,offset | 弹 setter:指向自机+偏移角 |
| spell_begin | slot,spell_id,pattern,time_limit,bonus0,flags,hp_threshold | 开卡:绑 boss/血线/计时/计分,spawn pattern 为卡绑定模式任务(随卡生死) |
| spell_end | (空) | 手动收卡(取卡按血线自动判,通常不需要) |
| spell_timer | (空) | 当前卡剩余帧数 |

- [ ] **Step 4: 绿 + 全量 + 金向量不变**(doc 字段不进镜像:golden 应逐字节同基线)

```bash
cargo test --workspace
cargo run --release -p stg-harness -- golden --out .superpowers/eclediting/golden-t2.txt
diff .superpowers/eclediting/golden-base.txt .superpowers/eclediting/golden-t2.txt && echo GOLDEN-OK
```

- [ ] **Step 5: fmt/clippy + commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add crates/stg-ecl-compiler/src/lang/builtins.rs
git commit -m "feat(ecl): Builtin 元数据补全——doc/param_names 字段 + all() 导出,单一真相源就位(编辑体验刀)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: `gen-ecl-meta` 生成器 + 防漂移

**Files:**
- Create: `crates/stg-harness/src/eclmeta.rs`、`editors/vscode/stg-ecl/ecl-meta.json`(生成物)
- Modify: `crates/stg-harness/src/main.rs`(`mod eclmeta;` + dispatch `Some("gen-ecl-meta") => eclmeta::cmd_gen()` + usage)

**Interfaces:**
- Consumes: Task 2 的 `stg_ecl_compiler::lang::builtins::{all, Builtin, ParamKind}` 与 `ast::Ty`。
- Produces: `ecl-meta.json`(Task 4 扩展读)+ `render_meta_json() -> String`(Task 5 文档生成段复用)+ 防漂移测试。

- [ ] **Step 1: 写失败测试**(eclmeta.rs 尾)

```rust
    /// 防漂移(verify-tables 同款):现生成 == commit 字节。
    #[test]
    fn committed_meta_matches_generated() {
        let generated = render_meta_json();
        let committed = std::fs::read_to_string(meta_path()).expect("ecl-meta.json 应已 commit");
        assert_eq!(generated, committed, "跑 `cargo run -p stg-harness -- gen-ecl-meta` 再 commit");
    }
```

- [ ] **Step 2: 红**(模块/函数未定义)。

- [ ] **Step 3: 实现 eclmeta.rs**

```rust
//! ecl-meta.json 生成器(编辑体验刀 spec §2)——单一真相源 = builtins::all()。
//! JSON 手写 format!(表序即输出序,确定性字节;无 serde 依赖)。

use std::process::ExitCode;
use stg_ecl_compiler::lang::ast::Ty;
use stg_ecl_compiler::lang::builtins::{Builtin, ParamKind, all};

pub fn meta_path() -> std::path::PathBuf {
    // harness 的 CARGO_MANIFEST_DIR = crates/stg-harness
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../editors/vscode/stg-ecl/ecl-meta.json")
}

fn ty_str(t: Ty) -> &'static str {
    match t {
        Ty::Int => "int",
        Ty::Fx => "fx",
        Ty::Angle => "angle",
    }
}

fn kind_str(k: ParamKind) -> String {
    match k {
        ParamKind::Val(t) => ty_str(t).to_string(),
        ParamKind::XformRef => "xform|none".to_string(),
        ParamKind::SubRef => "sub|none".to_string(),
        ParamKind::RawVal => "int|fx|angle".to_string(),
    }
}

/// `fire(appearance: int, x: fx, ...) -> int` 式签名渲染。
pub fn signature(b: &Builtin) -> String {
    let params: Vec<String> = b
        .params
        .iter()
        .zip(b.param_names.iter())
        .map(|(k, n)| format!("{n}: {}", kind_str(*k)))
        .collect();
    let ret = match b.ret {
        Some(t) => format!(" -> {}", ty_str(t)),
        None => String::new(),
    };
    format!("{}({}){ret}", b.name, params.join(", "))
}

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

pub fn render_meta_json() -> String {
    let mut out = String::from("{\n  \"version\": 1,\n  \"builtins\": [\n");
    let n = all().len();
    for (i, b) in all().iter().enumerate() {
        let params: Vec<String> = b
            .params
            .iter()
            .zip(b.param_names.iter())
            .map(|(k, name)| format!("{{\"name\": \"{}\", \"ty\": \"{}\"}}", esc(name), kind_str(*k)))
            .collect();
        out.push_str(&format!(
            "    {{\"name\": \"{}\", \"signature\": \"{}\", \"ret\": {}, \"doc\": \"{}\", \"params\": [{}]}}{}\n",
            esc(b.name),
            esc(&signature(b)),
            match b.ret { Some(t) => format!("\"{}\"", ty_str(t)), None => "null".to_string() },
            esc(b.doc),
            params.join(", "),
            if i + 1 == n { "" } else { "," }
        ));
    }
    out.push_str("  ]\n}\n");
    out
}

pub fn cmd_gen() -> ExitCode {
    match std::fs::write(meta_path(), render_meta_json()) {
        Ok(()) => {
            eprintln!("gen-ecl-meta: 写入 {}", meta_path().display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("gen-ecl-meta 失败: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // (Step 1 的防漂移测试放此)
}
```

(若 `ast::Ty`/`builtins` 的 pub 路径与此有出入,以实际模块路径为准调 use 并入报告——不许为路径搬类型。)

- [ ] **Step 4: 生成 + 绿**

```bash
mkdir -p editors/vscode/stg-ecl
cargo run -p stg-harness -- gen-ecl-meta
cargo test -p stg-harness committed_meta && cargo test --workspace
```

- [ ] **Step 5: fmt/clippy + commit**(生成物同 commit)

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add crates/stg-harness/src editors/vscode/stg-ecl/ecl-meta.json
git commit -m "feat(harness): gen-ecl-meta 生成器——builtins 单一真相源产 ecl-meta.json + 防漂移断言

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: VS Code 扩展

**Files:**
- Create: `editors/vscode/stg-ecl/package.json`、`syntaxes/ecl.tmLanguage.json`、`language-configuration.json`、`extension.js`、`README.md`

**Interfaces:**
- Consumes: `ecl-meta.json`(Task 3 生成物,同目录)。

- [ ] **Step 1: package.json**

```json
{
  "name": "stg-ecl",
  "displayName": "stg-engine ECL",
  "description": "stg-engine .ecl 弹幕脚本:高亮/补全/签名/hover(数据源 ecl-meta.json)",
  "version": "0.1.0",
  "publisher": "stg-engine-local",
  "engines": { "vscode": "^1.85.0" },
  "main": "./extension.js",
  "activationEvents": [],
  "contributes": {
    "languages": [
      {
        "id": "ecl",
        "extensions": [".ecl"],
        "aliases": ["ECL", "ecl"],
        "configuration": "./language-configuration.json"
      }
    ],
    "grammars": [
      { "language": "ecl", "scopeName": "source.ecl", "path": "./syntaxes/ecl.tmLanguage.json" }
    ]
  }
}
```

- [ ] **Step 2: language-configuration.json**(注释符按 lex.rs 实况:行 `//` + 块 `/* */`)

```json
{
  "comments": { "lineComment": "//", "blockComment": ["/*", "*/"] },
  "brackets": [["{", "}"], ["(", ")"]],
  "autoClosingPairs": [
    { "open": "{", "close": "}" },
    { "open": "(", "close": ")" },
    { "open": "/*", "close": " */" }
  ]
}
```

- [ ] **Step 3: tmLanguage**——关键字组**以 lex.rs `scan_ident_or_keyword` 的 match 全臂逐字转录**(已核实至少:`sub async var if else while loop for in wait spawn return break continue const xformdef as`,含 `none` 等若在臂内则一并;保留糖名 `wait_spell` 与类型名 `int fx angle` 各成组):

```json
{
  "$schema": "https://raw.githubusercontent.com/martinring/tmlanguage/master/tmlanguage.json",
  "name": "ECL",
  "scopeName": "source.ecl",
  "patterns": [
    { "include": "#comments" },
    { "include": "#keywords" },
    { "include": "#types" },
    { "include": "#numbers" },
    { "include": "#consts" },
    { "include": "#calls" },
    { "include": "#engvars" }
  ],
  "repository": {
    "comments": {
      "patterns": [
        { "name": "comment.line.double-slash.ecl", "match": "//.*$" },
        { "name": "comment.block.ecl", "begin": "/\\*", "end": "\\*/" }
      ]
    },
    "keywords": {
      "match": "\\b(sub|async|var|if|else|while|loop|for|in|wait|spawn|return|break|continue|const|xformdef|as|none|wait_spell)\\b",
      "name": "keyword.control.ecl"
    },
    "types": {
      "match": "\\b(int|fx|angle)\\b",
      "name": "storage.type.ecl"
    },
    "numbers": {
      "patterns": [
        { "match": "\\b\\d+\\.\\d+(fx|px)\\b", "name": "constant.numeric.fixed.ecl" },
        { "match": "\\b\\d+(fx|px)\\b", "name": "constant.numeric.fixed.ecl" },
        { "match": "\\b\\d+(deg|bam)\\b", "name": "constant.numeric.angle.ecl" },
        { "match": "\\b\\d+\\b", "name": "constant.numeric.int.ecl" }
      ]
    },
    "consts": {
      "match": "\\b[A-Z][A-Z0-9_]*\\b",
      "name": "variable.other.constant.ecl"
    },
    "calls": {
      "match": "\\b([a-z_][a-z0-9_]*)\\s*(?=\\()",
      "name": "entity.name.function.ecl"
    },
    "engvars": {
      "match": "\\$[a-z_]+",
      "name": "variable.language.engine.ecl"
    }
  }
}
```

(后缀四色是本语言第一易错点——`fx/px` 与 `deg/bam` 两组分 scope;`$engvar` 组按 lex.rs 是否真有 `$` 语法核实,无则删组并入报告。)

- [ ] **Step 4: extension.js**(零依赖三 provider)

```javascript
// stg-ecl 扩展:补全/签名/hover,数据源 = 同目录 ecl-meta.json(gen-ecl-meta 生成)。
const vscode = require("vscode");
const path = require("path");
const fs = require("fs");

function loadMeta(ctx) {
  const p = path.join(ctx.extensionPath, "ecl-meta.json");
  return JSON.parse(fs.readFileSync(p, "utf8"));
}

function activate(ctx) {
  const meta = loadMeta(ctx);
  const byName = new Map(meta.builtins.map((b) => [b.name, b]));

  ctx.subscriptions.push(
    vscode.languages.registerCompletionItemProvider("ecl", {
      provideCompletionItems() {
        return meta.builtins.map((b) => {
          const item = new vscode.CompletionItem(b.name, vscode.CompletionItemKind.Function);
          item.detail = b.signature;
          item.documentation = b.doc;
          const args = b.params.map((p, i) => "${" + (i + 1) + ":" + p.name + "}").join(", ");
          item.insertText = new vscode.SnippetString(b.name + "(" + args + ")");
          return item;
        });
      },
    }),
    vscode.languages.registerSignatureHelpProvider(
      "ecl",
      {
        provideSignatureHelp(doc, pos) {
          const line = doc.lineAt(pos.line).text.slice(0, pos.character);
          const m = line.match(/([a-z_][a-z0-9_]*)\s*\(([^()]*)$/);
          if (!m || !byName.has(m[1])) return null;
          const b = byName.get(m[1]);
          const sig = new vscode.SignatureInformation(b.signature, b.doc);
          sig.parameters = b.params.map(
            (p) => new vscode.ParameterInformation(p.name + ": " + p.ty)
          );
          const help = new vscode.SignatureHelp();
          help.signatures = [sig];
          help.activeSignature = 0;
          help.activeParameter = Math.min(
            (m[2].match(/,/g) || []).length,
            Math.max(0, b.params.length - 1)
          );
          return help;
        },
      },
      "(",
      ","
    ),
    vscode.languages.registerHoverProvider("ecl", {
      provideHover(doc, pos) {
        const range = doc.getWordRangeAtPosition(pos, /[a-z_][a-z0-9_]*/);
        if (!range) return null;
        const b = byName.get(doc.getText(range));
        if (!b) return null;
        const md = new vscode.MarkdownString();
        md.appendCodeblock(b.signature, "ecl");
        md.appendText(b.doc);
        return new vscode.Hover(md, range);
      },
    })
  );
}

function deactivate() {}
module.exports = { activate, deactivate };
```

- [ ] **Step 5: README.md**(装载两法 + 再生成 + 人肉判别清单)

```markdown
# stg-ecl(VS Code 扩展,仓库内目录装载)

## 装载
- 法一:`ln -s "$(pwd)/editors/vscode/stg-ecl" ~/.vscode/extensions/stg-ecl` 后重启 VS Code;
- 法二:VS Code 命令面板 `Developer: Install Extension from Location...` 选本目录。

## 数据再生成
builtin 变更后:`cargo run -p stg-harness -- gen-ecl-meta`(CI 防漂移测试会拦忘跑)。

## 人肉判别清单(改高亮/provider 后过一遍)
- [ ] `1.5fx` / `30deg` / `16384bam` 三种后缀三色,裸 `42` 第四色
- [ ] `fire(` 触发签名提示,逗号推进参数高亮位
- [ ] `fire` 悬停出签名+doc;补全列表含全部 builtin 且插入带参数占位
- [ ] `// 行注释` 与 `/* 块注释 */` 灰色;`sub/async/loop/wait` 关键字色;`SPELL_A` 常量色
```

- [ ] **Step 6: 人肉验收 + commit**——按 README 清单在 VS Code 里过一遍(若本机无 GUI,报告注明"清单待用户过",不算 BLOCKED):

```bash
git add editors/vscode/stg-ecl
git commit -m "feat(editors): VS Code 扩展——tmLanguage 四后缀高亮 + 补全/签名/hover(ecl-meta.json 数据驱动,零依赖)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: `ecl-lang.md` agent 优先重构 + 文档例可编译测试 + 收口

**Files:**
- Modify: `docs/ecl-lang.md`(重构)、`CLAUDE.md`(入口行标注)、`PROGRESS.md`(史+现在)
- Modify: `crates/stg-harness/src/eclmeta.rs`(加文档例编译测试 + 文档生成段渲染)

**Interfaces:**
- Consumes: Task 3 `signature()`/`all()`;Task 1 `check`。

- [ ] **Step 1: 重构 `docs/ecl-lang.md`**——保留现文档准确内容,重排为 agent 一次读全的结构(现有节:样例/三型/语句/sub 与 async/引擎变量/globals/常量/内建/符卡/通道 B/xformdef/错误格式):
  1. 开头加**《给 coding agent 的三行须知》**:①本文档是写/改 .ecl 的唯一权威;②改完必跑 `cargo run -p stg-harness -- check <file.ecl>` 看行列错误;③builtin 签名以 §内建函数生成段为准;
  2. **坑清单**新节(每条一行,全部实证):小数字面量必须 `fx`/`px` 后缀;角度字面量必须 `deg`/`bam` 后缀;有返回值的 builtin 不作表达式用时必须 `_ =` 弃值,void builtin 只能裸语句(不可 `_ =`);`fire` 的 xf/task 位是标识符或 `none`(非表达式);**实体从 spawn 后第 2 个 step 才存在**(born_frame 出生帧不跑);`drop_item` 消耗模拟 RNG(有随机喷发速度);难度读 `global(GVAR_RANK)`;符卡全套 = `spell_begin(slot, id, pattern, time, bonus0, flags, threshold)` + `wait_spell();`(`wait_spell` 是保留字);
  3. **内建函数节改为生成段**:以 `<!-- gen:builtins:begin -->` / `<!-- gen:builtins:end -->` 包围,内容 = 每 builtin 一行 `` `签名` — doc ``(由 Step 2 的渲染函数产);
  4. **debug 循环**新节:改 → `check` → 行列错误 → 再改;编译过后进金向量/游戏前先 `cargo test --workspace`;
  5. 全文所有 ```ecl 围栏例子**必须能过 check**(Step 3 测试押运——不合法的旧例子要修成合法或改非 ecl 围栏)。

- [ ] **Step 2: 文档生成段接进 gen-ecl-meta**(eclmeta.rs 加渲染+写盘;`cmd_gen` 同时刷新两个 sink)

```rust
/// 文档生成段(ecl-lang.md 的 builtin 表)。
pub fn render_doc_segment() -> String {
    let mut out = String::new();
    for b in all() {
        out.push_str(&format!("- `{}` — {}\n", signature(b), b.doc));
    }
    out
}

const DOC_BEGIN: &str = "<!-- gen:builtins:begin -->";
const DOC_END: &str = "<!-- gen:builtins:end -->";

pub fn doc_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/ecl-lang.md")
}

/// 幂等替换 ecl-lang.md 生成段;找不到标记 = 错。
pub fn splice_doc() -> Result<(), String> {
    let p = doc_path();
    let s = std::fs::read_to_string(&p).map_err(|e| e.to_string())?;
    let (Some(b), Some(e)) = (s.find(DOC_BEGIN), s.find(DOC_END)) else {
        return Err("ecl-lang.md 缺生成段标记".into());
    };
    let new = format!("{}{}\n{}{}", &s[..b + DOC_BEGIN.len()], "\n", render_doc_segment(), &s[e..]);
    std::fs::write(&p, new).map_err(|e| e.to_string())
}
```

(`cmd_gen` 里在写 meta 后调 `splice_doc()`,失败退 FAILURE。)防漂移测试加一条:

```rust
    /// 文档生成段防漂移:commit 的段内容 == 现渲染。
    #[test]
    fn committed_doc_segment_matches_generated() {
        let s = std::fs::read_to_string(doc_path()).unwrap();
        let b = s.find(DOC_BEGIN).expect("缺 begin 标记") + DOC_BEGIN.len();
        let e = s.find(DOC_END).expect("缺 end 标记");
        assert_eq!(s[b..e].trim_end(), format!("\n{}", render_doc_segment()).trim_end());
    }
```

- [ ] **Step 3: 文档例可编译测试**(eclmeta.rs tests;押运"文档例子永不腐烂")

```rust
    /// ecl-lang.md 的每个 ```ecl 围栏例子必须能编译(文档即规格,例子腐烂即红)。
    #[test]
    fn every_ecl_fenced_example_in_doc_compiles() {
        let s = std::fs::read_to_string(doc_path()).unwrap();
        let mut n = 0;
        let mut rest = s.as_str();
        while let Some(start) = rest.find("```ecl\n") {
            let body = &rest[start + 7..];
            let end = body.find("```").expect("未闭合的 ecl 围栏");
            let src = &body[..end];
            if let Err(errors) = stg_ecl_compiler::lang::compile(src, "doc.ecl") {
                let msg: Vec<String> = errors.iter().map(|e| e.render("doc.ecl")).collect();
                panic!("文档例 #{n} 编译失败:\n{src}\n---\n{}", msg.join("\n"));
            }
            n += 1;
            rest = &body[end + 3..];
        }
        assert!(n >= 3, "文档至少应有 3 个可编译示例,实得 {n}");
    }
```

(注:文档里刻意展示"错误示范"的片段用 ```text 围栏而非 ```ecl,天然豁免。片段级例子若非完整程序,重构时补成含 `sub main()` 的最小完整体。)

- [ ] **Step 4: 跑生成 + 全量 + 金向量终拍**

```bash
cargo run -p stg-harness -- gen-ecl-meta
cargo test --workspace
cargo run --release -p stg-harness -- golden --out .superpowers/eclediting/golden-final.txt
diff .superpowers/eclediting/golden-base.txt .superpowers/eclediting/golden-final.txt && echo GOLDEN-STABLE-OK
```

- [ ] **Step 5: 簿记 + commit**——`CLAUDE.md` 仓库结构 `docs/ecl-lang.md` 行加"**agent 必读**"、加 `editors/vscode/stg-ecl/` 一行;`PROGRESS.md` 史加行 + 「现在」段补一句(编辑体验刀落地);

```bash
cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings
git add docs/ecl-lang.md CLAUDE.md PROGRESS.md crates/stg-harness/src/eclmeta.rs editors/vscode/stg-ecl/ecl-meta.json
git commit -m "docs+feat: ecl-lang.md agent 优先重构(坑清单/生成段/debug 环/例子可编译押运)+ 编辑体验刀收口

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Self-Review 记录(plan 作者自查)

1. **Spec 覆盖**:§1 扩展四件→T4;§2 生成协议(doc 字段/gen/防漂移/生成段标记)→T2+T3+T5.2;§3 文档重构五要素→T5.1;§4 check→T1;§5 测试(防漂移/check 三路/金向量不变/文档例编译)→各任务+T5.3;§6 不做项无泄漏(无 LSP/热重载代码)。tmLanguage builtin 名单着色改"调用位通用 pattern"是 brainstorm 后的设计微调(少一个生成 sink),已体现于 T4 Step 3。
2. **占位符**:batch 参数名与关键字全臂是"从命名源码位置转录"的机械指令(Global Constraints 事实转录纪律),非 TBD;`$engvar` 组带"核实无则删"分支。
3. **类型一致性**:`all()/signature()/render_meta_json()/meta_path()/doc_path()/splice_doc()` 在 T2/T3 定义、T3/T5 消费一致;`check` 退码三路 T1 定义、T5 文档引用一致。
