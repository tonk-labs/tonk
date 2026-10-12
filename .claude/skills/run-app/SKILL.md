---
name: run-app
description: Launch the tonk web app in this container without nix (local access service + trunk serve) and drive it in headless Chromium with Playwright. Use to run, screenshot, or verify any UI change in the real app, not just its tests.
---

# Run the tonk web app (no nix)

`nix develop -c dev:web` is the normal path; this container has no nix. The
same thing by hand:

```bash
.claude/skills/run-app/dev-web.sh      # ~8 min cold, seconds warm
```

It installs `trunk` 0.21.14 and the `wasm-bindgen` CLI matching the
workspace pin into `~/.local/bin`, starts `tonk-access-local`, writes the
gitignored `rust/tonk-ui/.Trunk.dev.toml` / `.index.dev.html` with the
proxies `dev:web` adds (`/@`, `/.well-known/tonk`, `/.well-known/did.json`,
`/customer/`, `/ucan/`), and runs `trunk serve` on `:8080`. Logs are in
`/tmp/tonk-run/`. Trunk watches the workspace, so edits rebuild; a library
change (`rust/tonk-core/assets/library/*.yaml`) is served at once.

Stop: `lsof -ti:8080 -sTCP:LISTEN | xargs -r kill` and kill
`tonk-access-local` by its PID (`pgrep -f tonk-access-local`) — avoid broad
`pkill -f` patterns.

## Test a local template catalog

Discover lists the deployment's default catalog, which the access service
serves at `/.well-known/tonk/discover` from `TEMPLATE_CATALOG_URL` (a
wrangler var per environment; the local service defaults to the production
URL). Point it at a catalog you serve yourself:

```bash
# A catalog is fetched cross-origin from the sealed Hub, so serve it with
# `Access-Control-Allow-Origin: *` (python's http.server does not).
TEMPLATE_CATALOG_URL=http://localhost:8777/catalog.json .claude/skills/run-app/dev-web.sh
curl -s localhost:8080/.well-known/tonk/discover   # {"catalog": "http://localhost:8777/catalog.json"}
```

Only `https:` or `http:` on `localhost` / `127.0.0.1` / `[::1]` is served;
anything else answers `"catalog": null` and the Hub falls back to the
`catalog-url` its library names. Catalogs added in Discover's "catalogs"
control are held to the same rule.

## Drive it

Playwright is installed globally; link it next to your script, and Chromium
is preinstalled (`PLAYWRIGHT_BROWSERS_PATH`, never `playwright install`):

```bash
cd /tmp/some-dir && ln -sf "$(npm root -g)" node_modules
cp /home/user/tonk/.claude/skills/run-app/drive.mjs .
OUT=shot.png node drive.mjs            # load, settle, screenshot
```

A fresh browser context is a fresh profile: it seeds the current libraries
and lands in the onboarding "Welcome" space (`/space/did:key:…`).

## Gotchas

- **Frames.** The page is nested sealed iframes (`about:srcdoc`): the
  profile chrome guest (FABB, `<command-palette>`) and inside it the space
  guest. Find elements with `frameWith(page, () => !!document.querySelector(…))`
  and re-find after anything that may remount a guest.
- **Keys.** `page.keyboard` goes to the focused frame. To hit a document
  listener in a guest, dispatch there:
  `frame.evaluate(() => document.dispatchEvent(new KeyboardEvent('keydown', {key:'k', ctrlKey:true})))`.
- **Author elements** (`element!:`) must not be named `tonk-*` or `wa-*`: the
  element runtime never announces those tags, so they stay inert.
- **Settle time.** First load runs onboarding; wait ~30s before driving.
