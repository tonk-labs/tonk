export type BuiltinArtifact = {
  entity: string;
  name: string;
  description: string;
  html: string;
};

const EDITOR_HTML = `<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<style>
@import url('https://fonts.googleapis.com/css2?family=Inconsolata:wght@400;500&display=swap');
* { box-sizing: border-box; margin: 0; padding: 0; }
html, body { height: 100%; background: #fff; }
textarea {
  display: block;
  width: 100%;
  height: 100%;
  font-family: 'Inconsolata', Menlo, 'Courier New', monospace;
  font-size: 14px;
  line-height: 1.6;
  border: none;
  outline: none;
  padding: 16px;
  resize: none;
  background: #fff;
  color: #1a1a1a;
  caret-color: #1a1a1a;
}
</style>
</head>
<body>
<textarea spellcheck="false" autocomplete="off" autocorrect="off" autocapitalize="off"></textarea>
</body>
</html>`;

const TABLE_HTML = `<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<style>
* { box-sizing: border-box; margin: 0; padding: 0; }
html, body { height: 100%; font-family: system-ui, sans-serif; font-size: 13px; background: #fff; color: #111; }
.wrap { display: flex; flex-direction: column; height: 100%; }
.toolbar { display: flex; gap: 6px; padding: 8px 10px; border-bottom: 1px solid #e5e5e5; background: #f9f9f9; flex-shrink: 0; }
button { padding: 4px 10px; border: 1px solid #d0d0d0; border-radius: 5px; cursor: pointer; background: #fff; font-size: 12px; color: #333; }
button:hover { background: #f0f0f0; }
.scroll { flex: 1; overflow: auto; }
table { border-collapse: collapse; min-width: 100%; }
td { border: 1px solid #e0e0e0; min-width: 100px; padding: 0; }
td div[contenteditable] { outline: none; padding: 6px 8px; min-height: 30px; white-space: pre-wrap; }
td div[contenteditable]:focus { background: #f0f7ff; }
</style>
</head>
<body>
<div class="wrap">
  <div class="toolbar">
    <button onclick="addRow()">+ Row</button>
    <button onclick="addCol()">+ Column</button>
  </div>
  <div class="scroll">
    <table id="tbl">
      <tr><td><div contenteditable="true"></div></td><td><div contenteditable="true"></div></td><td><div contenteditable="true"></div></td></tr>
      <tr><td><div contenteditable="true"></div></td><td><div contenteditable="true"></div></td><td><div contenteditable="true"></div></td></tr>
      <tr><td><div contenteditable="true"></div></td><td><div contenteditable="true"></div></td><td><div contenteditable="true"></div></td></tr>
    </table>
  </div>
</div>
<script>
function cell() {
  var td = document.createElement('td');
  var d = document.createElement('div');
  d.contentEditable = 'true';
  td.appendChild(d);
  return td;
}
function addRow() {
  var t = document.getElementById('tbl');
  var cols = t.rows[0] ? t.rows[0].cells.length : 3;
  var tr = document.createElement('tr');
  for (var i = 0; i < cols; i++) tr.appendChild(cell());
  t.appendChild(tr);
}
function addCol() {
  var t = document.getElementById('tbl');
  for (var i = 0; i < t.rows.length; i++) t.rows[i].appendChild(cell());
}
</script>
</body>
</html>`;

const SITE_HTML = `<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<style>
* { box-sizing: border-box; margin: 0; padding: 0; }
html, body { height: 100%; display: flex; align-items: center; justify-content: center; font-family: system-ui, sans-serif; background: #fff; }
h1 { font-size: 2rem; font-weight: 600; color: #111; letter-spacing: -0.02em; }
</style>
</head>
<body>
<h1>Hello, World!</h1>
</body>
</html>`;

export const BUILTIN_ARTIFACTS: BuiltinArtifact[] = [
  {
    entity: "builtin:editor",
    name: "Editor",
    description: "Plaintext editor",
    html: EDITOR_HTML,
  },
  {
    entity: "builtin:table",
    name: "Table",
    description: "Editable table",
    html: TABLE_HTML,
  },
  {
    entity: "builtin:site",
    name: "Site",
    description: "HTML display",
    html: SITE_HTML,
  },
];

export function builtinSrc(entity: string): string {
  const b = BUILTIN_ARTIFACTS.find((a) => a.entity === entity);
  if (!b) return "";
  return `data:text/html;charset=utf-8,${encodeURIComponent(b.html)}`;
}

export function isBuiltin(entity: string): boolean {
  return entity.startsWith("builtin:");
}
