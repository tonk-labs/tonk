// `markdown-it-mark` ships no types. It is a standard markdown-it plugin:
// a function passed to `md.use(...)` that adds `==highlight==` support,
// emitting `mark_open`/`mark_close` tokens.
declare module "markdown-it-mark" {
  import type MarkdownIt from "markdown-it";
  const markdownItMark: MarkdownIt.PluginSimple;
  export default markdownItMark;
}
