(self) => `
  :host { display: block; position: relative; min-height: 18rem; background: #191919; color: #fff; outline: none; overflow: hidden; font-family: system-ui, -apple-system, 'Segoe UI', Roboto, sans-serif; }
  :host(:focus-visible) { box-shadow: inset 0 0 0 2px #42affa; }
  [hidden] { display: none !important; }
  .stage { position: absolute; left: 50%; top: 50%; width: 1280px; height: 720px; transform-origin: center; }
  .slides { position: absolute; inset: 0; }
  section { position: absolute; inset: 0; box-sizing: border-box; padding: 36px 72px; display: flex; flex-direction: column; justify-content: center; align-items: center; text-align: center; font-size: 38px; line-height: 1.3; opacity: 0; visibility: hidden; transition: opacity .3s ease, transform .3s ease, visibility .3s; }
  section[data-state="present"] { opacity: 1; visibility: visible; transform: none; }
  section[data-state="past"][data-axis="h"] { transform: translateX(-6%); }
  section[data-state="future"][data-axis="h"] { transform: translateX(6%); }
  section[data-state="past"][data-axis="v"] { transform: translateY(-6%); }
  section[data-state="future"][data-axis="v"] { transform: translateY(6%); }
  section > * { max-width: 100%; }
  h1, h2, h3, h4 { margin: 0 0 .4em; font-weight: 700; line-height: 1.15; text-transform: uppercase; }
  h1 { font-size: 2.3em; } h2 { font-size: 1.5em; } h3 { font-size: 1.2em; } h4 { font-size: 1em; }
  p { margin: .3em 0; }
  ul, ol { display: inline-block; text-align: left; margin: .3em 0; padding-left: 1.3em; }
  li { margin: .15em 0; }
  a { color: #42affa; text-decoration: none; }
  del { opacity: .7; }
  code { font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; font-size: .9em; }
  pre { width: 92%; box-sizing: border-box; margin: .4em auto; padding: .7em 1em; text-align: left; font-size: 21px; line-height: 1.45; background: #0d1117; color: #e6edf3; border-radius: 8px; box-shadow: 0 5px 15px rgba(0, 0, 0, .3); overflow: auto; max-height: 520px; }
  pre code { font-size: 1em; white-space: pre; }
  table { border-collapse: collapse; margin: .4em auto; font-size: .78em; }
  th, td { padding: .25em .7em; border-bottom: 1px solid rgba(255, 255, 255, .35); text-align: left; }
  blockquote { width: 80%; margin: .4em auto; padding: .2em 1em; border-left: 4px solid #666; font-style: italic; color: #ccc; text-align: left; }
  img { max-width: 100%; max-height: 520px; }
  .diagram { width: 100%; margin: .3em auto; }
  .diagram svg { display: block; width: 100%; height: auto; max-height: 500px; margin: 0 auto; }
  .t-comment, .t-attr { color: #8b949e; } .t-string { color: #a5d6ff; } .t-keyword { color: #ff7b72; } .t-number, .t-var { color: #79c0ff; }
  .t-type, .t-anchor { color: #ffa657; } .t-macro, .t-call, .t-head { color: #d2a8ff; } .t-key, .t-arg { color: #7ee787; }
  .empty { position: absolute; inset: 0; display: grid; place-items: center; padding: 2rem; text-align: center; font-size: 16px; color: #aaa; }
  .progress { position: absolute; left: 0; bottom: 0; height: 4px; width: 0; background: #42affa; transition: width .3s ease; z-index: 30; }
  .controls { position: absolute; right: 10px; bottom: 12px; z-index: 40; display: grid; grid-template-columns: repeat(3, 26px); grid-template-rows: repeat(3, 22px); }
  .controls button { border: none; background: none; color: #42affa; font-size: 20px; line-height: 1; cursor: pointer; padding: 0; }
  .controls button:disabled { opacity: .15; cursor: default; }
  .controls [data-dir="up"] { grid-area: 1 / 2; } .controls [data-dir="left"] { grid-area: 2 / 1; }
  .controls [data-dir="right"] { grid-area: 2 / 3; } .controls [data-dir="down"] { grid-area: 3 / 2; }
  .bar { position: absolute; top: .5rem; right: .5rem; z-index: 40; display: flex; gap: .5rem; align-items: center; opacity: .25; transition: opacity .2s; font: 12px system-ui, sans-serif; }
  :host(:hover) .bar, :host(:focus-within) .bar { opacity: 1; }
  .bar button { font: inherit; padding: .25rem .6rem; border-radius: 4px; border: 1px solid #666; background: #222; color: #eee; cursor: pointer; }
`
