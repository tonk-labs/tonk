// Synthetic entry: the Web Awesome components the sealed guest needs, bundled by
// esbuild into a single self-contained ESM (wa.js) the portal injects. Importing
// a component's module registers its `<wa-*>` element. Keep in sync with the
// `<wa-*>` tags guest-rendered views use (tonk-display views, tonk-inspector
// result rendering, tonk-tree). A missing entry = an inert unknown element = no
// visible output. Rebuild with scripts/build-wa-guest.mjs.
import "../webawesome/components/button/button.js";
import "../webawesome/components/callout/callout.js";
import "../webawesome/components/card/card.js";
import "../webawesome/components/carousel/carousel.js";
import "../webawesome/components/carousel-item/carousel-item.js";
import "../webawesome/components/copy-button/copy-button.js";
import "../webawesome/components/dialog/dialog.js";
import "../webawesome/components/icon/icon.js";
import "../webawesome/components/input/input.js";
import "../webawesome/components/popup/popup.js";
import "../webawesome/components/spinner/spinner.js";
import "../webawesome/components/tooltip/tooltip.js";
// Added for the inspector result panel + the tree inspector (diagnose):
import "../webawesome/components/tree/tree.js";
import "../webawesome/components/tree-item/tree-item.js";
import "../webawesome/components/tab-group/tab-group.js";
import "../webawesome/components/tab/tab.js";
import "../webawesome/components/tab-panel/tab-panel.js";
import "../webawesome/components/comparison/comparison.js";
import "../webawesome/components/badge/badge.js";
// Added for the /console page: subscription rows show when each was opened,
// and `<wa-relative-time sync>` counts that up on its own. `<wa-details>`
// groups them by (repository, branch) — a collapsible section rather than a
// `<wa-tree>`, because a tree assigns its children through a slot and only
// sees DIRECT children, while the rows arrive wrapped in the
// `tonk-display`/`tonk-view` layers every nested view renders through.
import "../webawesome/components/relative-time/relative-time.js";
import "../webawesome/components/details/details.js";
