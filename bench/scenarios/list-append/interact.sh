#!/usr/bin/env bash
# Exercise the mounted projection through a real form submit, then reload and
# require the durable item to render again before the structural verifier runs.
set -euo pipefail

ROOT="${ROOT:?}"
RUN_DIR="${RUN_DIR:?}"
BENCH_URL="${BENCH_URL:?}"
SPACE_NAME="${SPACE_NAME:?}"
B="$ROOT/bench/bin/browser.sh"
EXPECTED="Buy milk from the benchmark"
URL="$BENCH_URL/space/$SPACE_NAME/todo/list"

wait_for() {
  local selector="$1" expression="$2" label="$3" out=""
  for _ in $(seq 1 60); do
    out="$("$B" frame-eval "$selector" "$expression" 2>&1 || true)"
    [ "$out" = true ] && return 0
    sleep 0.5
  done
  echo "interact: timed out waiting for $label (last output: $out)" >&2
  return 1
}

"$B" goto "$URL"
"$B" wait-render
wait_for 'input[name=title]' true "mounted title control"

submit_result="$("$B" frame-eval 'input[name=title]' "(function () {
  const input = document.querySelector('input[name=title]');
  input.value = '$EXPECTED';
  input.dispatchEvent(new Event('input', { bubbles: true, composed: true }));
  input.dispatchEvent(new Event('change', { bubbles: true, composed: true }));
  const form = input.form;
  if (!form) return 'missing-form';
  form.requestSubmit();
  return 'submitted';
})()")"
[ "$submit_result" = '"submitted"' ] || {
  echo "interact: mounted submit failed: $submit_result" >&2
  exit 1
}

wait_for '[data-todo]' "document.body.textContent.includes('$EXPECTED')" "submitted todo"
"$B" goto "$URL"
"$B" wait-render
wait_for '[data-todo]' "document.body.textContent.includes('$EXPECTED')" "todo after reload"

push_raw="$("$B" eval-async "(function(done){fetch('/api/repository/$SPACE_NAME/branch/main/sync/push', {method:'POST'}).then(function(r){return r.text().then(function(t){done(r.status+':'+t.slice(0,200));});}).catch(function(e){done('err:'+String(e));});})(arguments[0])")"
push_raw="${push_raw#\"}"
push_raw="${push_raw%\"}"
case "$push_raw" in
  200:*) ;;
  *) echo "interact: browser push did not confirm: $push_raw" >&2; exit 1 ;;
esac

projection_errors="$(grep -Ei '((projection|invocation|invoke).*(error|failed))|((error|failed).*(projection|invocation|invoke))' \
  "$RUN_DIR/chrome-profile/chrome_debug.log" || true)"
if [ -n "$projection_errors" ]; then
  echo "interact: projection/invocation console error: $projection_errors" >&2
  exit 1
fi

jq -n --arg title "$EXPECTED" --arg url "$URL" '{
  available: true,
  passed: true,
  submitted_title: $title,
  survived_reload: true,
  pushed: true,
  projection_console_errors: 0,
  url: $url
}' > "$RUN_DIR/browser.json"
