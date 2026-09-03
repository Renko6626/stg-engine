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
  // `$` 引擎变量（D18）：同一份 ecl-meta.json 的第二节。旧版扩展只有一条通配高亮正则,
  // 于是打 `$` 没补全、悬停没 hover、写错名字（`$self_vel`）编辑器不吭声——要到 check 才报错。
  const engVars = meta.engine_vars || [];
  const engByName = new Map(engVars.map((v) => [v.name, v]));

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
    vscode.languages.registerCompletionItemProvider(
      "ecl",
      {
        provideCompletionItems(doc, pos) {
          // 只在刚打出 `$`（或正在补 `$na|`）时给引擎变量，避免污染普通标识符补全
          const line = doc.lineAt(pos.line).text.slice(0, pos.character);
          if (!/\$[a-z0-9_]*$/.test(line)) return null;
          return engVars.map((v) => {
            const item = new vscode.CompletionItem(v.name, vscode.CompletionItemKind.Variable);
            item.detail = "$" + v.name + ": " + v.ty;
            item.documentation = new vscode.MarkdownString(v.doc);
            return item;
          });
        },
      },
      "$"
    ),
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
        // 引擎变量优先：词范围带上前导 `$`，否则 `$self_x` 会被当成内建名 `self_x` 查空
        const evRange = doc.getWordRangeAtPosition(pos, /\$[a-z_][a-z0-9_]*/);
        if (evRange) {
          const v = engByName.get(doc.getText(evRange).slice(1));
          if (v) {
            const md = new vscode.MarkdownString();
            md.appendCodeblock("$" + v.name + ": " + v.ty, "ecl");
            md.appendMarkdown(v.doc);
            return new vscode.Hover(md, evRange);
          }
        }
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
