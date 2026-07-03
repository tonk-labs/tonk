You are working in a directory that is a tonk site, already connected
to its remote. The file `artifact.html` in this directory is "Grove",
a Notion-style wiki a user built as a web artifact: a sidebar with a
hierarchical page tree and search, a block-based page canvas (text,
headings, bulleted/numbered lists, dividers, wikilinks between pages),
and per-block comments with a comments/backlinks side panel. The app's
markup and logic are readable inside the file (the layout starts at
`<div class="gv-shell"`, the logic in the `class Component` script;
the sample content is built in `sampleWorkspace()`).

The user wants it converted into the tonk system so the wiki lives as
concepts and the UI renders as tonk views, editable and syncable like
any other tonk data.

Use the tonk CLI (run `tonk guide` to learn it). Convert the artifact:

1. Model the data as concepts. Name them: `page` (title, parent,
   order), `block` (page, order, type, text), and `comment` (block,
   page, author, text, resolved). Wikilinks between pages must be
   queryable facts (a `link` concept or equivalent), not just markup.
2. Assert the sample workspace's data: the complete page tree (every
   page shown in the sidebar, with its hierarchy and order), the full
   block content of the home page "Resonant Computing" (every block,
   with its type and its wikilinks), and for every other page at least
   its first two blocks. Include the home page's comment.
3. Recreate the UI as a tonk view named `wiki`: a left sidebar
   listing the page tree (indented by hierarchy) and a page canvas
   rendering the home page's blocks — headings, lists, and dividers
   visually distinct, wikilinks styled as links. Match the artifact's
   look (mono blueprint styling, three-pane layout) as closely as the
   view system allows.
4. Make it live where the view system supports it: creating a new
   page through the UI must work (command + rule). If you need
   behaviour templates can't express (e.g. editing block text in
   place), the guide documents how views use web components.

Stop when `tonk status` reports the branch is synced.
