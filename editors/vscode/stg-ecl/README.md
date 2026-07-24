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
