#!/usr/bin/env bash
# Thin WebDriver client over chromedriver. Headless Chrome with a
# fresh per-run profile. Subcommands:
#   start | stop | goto <url> | eval <js-expr> | wait-render | wait-sw | shot <out.png>
#
# `eval` wraps the expression in `return (...)` and prints the JSON value.
# Env: RUN_DIR, CHROMEDRIVER (from devshell), BENCH_CDP_PORT (default 9515)
set -euo pipefail

RUN_DIR="${RUN_DIR:?set RUN_DIR}"
PORT="${BENCH_CDP_PORT:-9515}"
BASE="http://127.0.0.1:$PORT"
SESSION_FILE="$RUN_DIR/browser.session"

wd() { # method path [json-body]
  local method="$1" path="$2" body="${3:-}"
  if [ -n "$body" ]; then
    curl -fsS -X "$method" "$BASE$path" -H 'Content-Type: application/json' -d "$body"
  else
    curl -fsS -X "$method" "$BASE$path"
  fi
}

sid() { cat "$SESSION_FILE"; }

start() {
  "${CHROMEDRIVER:?CHROMEDRIVER not set (enter the devshell)}" --port="$PORT" \
    > "$RUN_DIR/chromedriver.log" 2>&1 &
  echo $! > "$RUN_DIR/chromedriver.pid"
  for _ in $(seq 1 20); do
    curl -fso /dev/null "$BASE/status" && break
    sleep 0.5
  done
  local caps
  caps=$(jq -n --arg udd "$RUN_DIR/chrome-profile" '{capabilities:{alwaysMatch:{
    browserName:"chrome",
    "goog:chromeOptions":{args:[
      "--headless=new","--window-size=1280,900","--disable-gpu",
      "--no-first-run",("--user-data-dir="+$udd)]}}}}')
  wd POST /session "$caps" | jq -r '.value.sessionId' > "$SESSION_FILE"
  [ -s "$SESSION_FILE" ] || { echo "browser: no session" >&2; exit 1; }
}

stop() {
  if [ -f "$SESSION_FILE" ]; then
    wd DELETE "/session/$(sid)" >/dev/null 2>&1 || true
    rm -f "$SESSION_FILE"
  fi
  if [ -f "$RUN_DIR/chromedriver.pid" ]; then
    kill "$(cat "$RUN_DIR/chromedriver.pid")" 2>/dev/null || true
    rm -f "$RUN_DIR/chromedriver.pid"
  fi
}

goto() {
  wd POST "/session/$(sid)/url" "$(jq -n --arg u "$1" '{url:$u}')" >/dev/null
}

evaljs() {
  wd POST "/session/$(sid)/execute/sync" \
    "$(jq -n --arg s "return ($1);" '{script:$s,args:[]}')" | jq -c '.value'
}

# Poll a JS predicate until it returns true. wait_for <js-expr> <timeout-s> <label>
wait_for() {
  local expr="$1" timeout="${2:-30}" label="${3:-condition}"
  for _ in $(seq 1 $((timeout * 2))); do
    [ "$(evaljs "$expr")" = "true" ] && return 0
    sleep 0.5
  done
  echo "browser: timed out waiting for $label" >&2
  return 1
}

# The SPA has rendered when the document is complete and the body has
# real content (Leptos mounts into <body>). For tonk-ui specifically we
# wait for <tonk-host> to be present — the root custom element that
# TonkLauncher renders immediately on WASM init, before any async
# data fetching completes.
wait_render() {
  wait_for "document.readyState === 'complete' && document.querySelector('tonk-host') !== null" 30 render
  sleep 1   # settle: fonts, async view frames
}

# The service worker must control the page before /api fetches work.
wait_sw() {
  wait_for "'serviceWorker' in navigator" 5 "sw support"
  if [ "$(evaljs "navigator.serviceWorker.controller !== null")" != "true" ]; then
    wait_for "navigator.serviceWorker !== undefined && navigator.serviceWorker.getRegistrations !== undefined" 5 "sw api"
    # First load registers the SW but doesn't control the page; reload once.
    sleep 2
    wd POST "/session/$(sid)/refresh" '{}' >/dev/null
    wait_for "navigator.serviceWorker.controller !== null" 30 "sw control"
  fi
}

shot() {
  wd GET "/session/$(sid)/screenshot" | jq -r '.value' | base64 --decode > "$1"
  [ -s "$1" ] || { echo "browser: empty screenshot $1" >&2; return 1; }
}

case "${1:-}" in
  start) start ;;
  stop) stop ;;
  goto) goto "$2" ;;
  eval) evaljs "$2" ;;
  wait-render) wait_render ;;
  wait-sw) wait_sw ;;
  shot) shot "$2" ;;
  *) echo "usage: browser.sh {start|stop|goto <url>|eval <js>|wait-render|wait-sw|shot <png>}" >&2; exit 2 ;;
esac
