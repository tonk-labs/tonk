#!/usr/bin/env bash
# Thin WebDriver client over chromedriver. Headless Chrome with a
# fresh per-run profile. Subcommands:
#   start | stop | goto <url> | eval <js-expr> | eval-async <js-expr>
#   frame-eval <selector> <js-expr> | wait-render | wait-sw | shot <out.png>
#
# `eval` wraps the expression in `return (...)` and prints the JSON value.
# `eval-async` awaits a promise-valued expression via execute/async.
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
  # Double-start guard
  if [ -f "$RUN_DIR/chromedriver.pid" ] && kill -0 "$(cat "$RUN_DIR/chromedriver.pid")" 2>/dev/null; then
    echo "browser: already running (pid $(cat "$RUN_DIR/chromedriver.pid"))" >&2
    return 0
  fi

  set -m
  "${CHROMEDRIVER:?CHROMEDRIVER not set (enter the devshell)}" --port="$PORT" \
    > "$RUN_DIR/chromedriver.log" 2>&1 &
  echo $! > "$RUN_DIR/chromedriver.pid"
  set +m

  for _ in $(seq 1 20); do
    curl -fso /dev/null "$BASE/status" && break
    sleep 0.5
  done

  # Fail loudly if chromedriver never came up
  curl -fso /dev/null "$BASE/status" || {
    echo "browser: chromedriver did not start; tail of log:" >&2
    tail -5 "$RUN_DIR/chromedriver.log" >&2
    exit 1
  }

  local caps
  caps=$(jq -n --arg udd "$RUN_DIR/chrome-profile" '{capabilities:{alwaysMatch:{
    browserName:"chrome",
    "goog:chromeOptions":{args:[
      "--headless=new","--window-size=1280,900","--disable-gpu",
      "--no-first-run",("--user-data-dir="+$udd)]}}}}')
  wd POST /session "$caps" | jq -r '.value.sessionId' > "$SESSION_FILE"
  [ -s "$SESSION_FILE" ] || { echo "browser: no session" >&2; exit 1; }
  # Set async script timeout to 30 s so eval-async calls have headroom.
  wd POST "/session/$(sid)/timeouts" '{"script":30000}' >/dev/null
}

stop() {
  if [ -f "$SESSION_FILE" ]; then
    wd DELETE "/session/$(sid)" >/dev/null 2>&1 || true
    rm -f "$SESSION_FILE"
  fi
  if [ -f "$RUN_DIR/chromedriver.pid" ]; then
    local pid
    pid="$(cat "$RUN_DIR/chromedriver.pid")"
    kill -- "-$pid" 2>/dev/null || kill "$pid" 2>/dev/null || true
    for _ in $(seq 1 10); do
      kill -0 "$pid" 2>/dev/null || break
      sleep 0.5
    done
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

# evaljs_async: run a script that calls the WebDriver callback (arguments[0])
# with its result. The script has access to Promise and fetch — useful for
# SW-intercepted fetches that cannot be made synchronously.
#
# The script receives a done callback as arguments[0]; it must call
# done(value) to complete. Example:
#   evaljs_async "fetch('/api/foo', {method:'POST'}).then(r=>arguments[0](r.status))"
evaljs_async() {
  wd POST "/session/$(sid)/execute/async" \
    "$(jq -n --arg s "$1" '{script:$s,args:[]}')" \
    | jq -c '.value'
}

# Search the current frame tree for a document containing selector. On success
# leave WebDriver focused in that frame; callers restore the top frame after
# evaluating. This uses WebDriver frame switching because sealed guest iframes
# deliberately have opaque origins and cannot be traversed from page JS.
find_frame() {
  local selector="$1" selector_json frames count frame index
  selector_json="$(jq -n --arg selector "$selector" '$selector')"
  if [ "$(evaljs "document.querySelector($selector_json) !== null" 2>/dev/null || true)" = true ]; then
    return 0
  fi

  frames="$(wd POST "/session/$(sid)/elements" \
    '{"using":"css selector","value":"iframe"}')"
  count="$(printf '%s' "$frames" | jq '.value | length')"
  [ "$count" -gt 0 ] || return 1
  for index in $(seq 0 $((count - 1))); do
    frame="$(printf '%s' "$frames" | jq -c ".value[$index]")"
    wd POST "/session/$(sid)/frame" "$(jq -n --argjson id "$frame" '{id:$id}')" >/dev/null
    if find_frame "$selector"; then
      return 0
    fi
    wd POST "/session/$(sid)/frame/parent" '{}' >/dev/null
  done
  return 1
}

frame_eval() {
  local selector="$1" expression="$2" result
  wd POST "/session/$(sid)/frame" '{"id":null}' >/dev/null
  if ! find_frame "$selector"; then
    wd POST "/session/$(sid)/frame" '{"id":null}' >/dev/null
    return 1
  fi
  result="$(evaljs "$expression")"
  wd POST "/session/$(sid)/frame" '{"id":null}' >/dev/null
  printf '%s\n' "$result"
}

# Poll a JS predicate until it returns true. wait_for <js-expr> <timeout-s> <label>
wait_for() {
  local expr="$1" timeout="${2:-30}" label="${3:-condition}" out=""
  for _ in $(seq 1 $((timeout * 2))); do
    out="$(evaljs "$expr" 2>&1 || true)"
    [ "$out" = "true" ] && return 0
    sleep 0.5
  done
  echo "browser: timed out waiting for $label (last output: $out)" >&2
  return 1
}

# The SPA has rendered when the document is complete and the body has
# real content (Leptos mounts into <body>). For tonk-ui specifically we
# wait for <tonk-site> to be present — the mount root since the
# routing collapse (#567) replaced the old <tonk-host> element with a
# document-level install plus a single <tonk-site> on <body>.
wait_render() {
  wait_for "document.readyState === 'complete' && document.querySelector('tonk-site') !== null" 30 render
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
  eval-async) evaljs_async "$2" ;;
  frame-eval) frame_eval "$2" "$3" ;;
  wait-render) wait_render ;;
  wait-sw) wait_sw ;;
  shot) shot "$2" ;;
  *) echo "usage: browser.sh {start|stop|goto <url>|eval <js>|eval-async <js>|frame-eval <selector> <js>|wait-render|wait-sw|shot <png>}" >&2; exit 2 ;;
esac
