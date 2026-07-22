#!/usr/bin/env bash
# Capture the scenario's checkpoint screenshots. Each line of the
# scenario's `checkpoints` file is either:
#   home                — the shell root ($BENCH_URL/)
#   display:<view-name> — resolved at capture time from the view's anchor name
#   <path>              — a suffix under /space/$SPACE_NAME/
# Blank lines and #-comments are skipped. A failed screenshot is
# recorded as missing, not fatal — the judge sees what there is.
#
# Every `tonk` call here resolves through TONK_SPOT (exported by
# run.sh). The CLI never consults the cwd, so there is nothing to
# `cd` into.
#
# Env: ROOT, RUN_DIR, BENCH_URL, SPACE_NAME, SCENARIO (scenario dir),
#      TONK_SPOT
set -euo pipefail

ROOT="${ROOT:?}"; RUN_DIR="${RUN_DIR:?}"; BENCH_URL="${BENCH_URL:?}"
SPACE_NAME="${SPACE_NAME:-bench}"; SCENARIO="${SCENARIO:?}"
B="$ROOT/bench/bin/browser.sh"
TONK="$ROOT/target/release/tonk"
mkdir -p "$RUN_DIR/shots"

# Print the concept name a view's `model` field points at, or nothing
# when it cannot be resolved. Never fails — the caller decides what to
# do with an empty answer.
#
# `tonk query view <view-name>` resolves the name through the branch's
# name table, so it lands on the row whichever way the view was
# authored: `tonk view add --name <view-name>` writes an explicit
# `this: id:<view-name>`, while the bare anchor form (`view!:
# &<view-name>` with no `this:`, what every scenario here uses) derives
# a `did:key:` entity from the row's content and publishes the name as
# a separate referent fact. Matching a hardcoded `id:<view-name>` would
# see only the first shape.
view_model() {
  local view_name="$1"
  command -v jq >/dev/null 2>&1 || return 0

  local view_json
  view_json="$("$TONK" query view "$view_name" --json 2>/dev/null)" || return 0

  local model_uri
  model_uri="$(printf '%s' "$view_json" \
    | jq -r '.matches_before[0].results[0].fields.model // empty' 2>/dev/null)" || return 0
  [ -n "$model_uri" ] || return 0

  # The model field holds a concept URI; the concept's own row carries
  # the name the display route wants.
  "$TONK" eval --no-sync --format json -c 'concept:' 2>/dev/null \
    | jq -r --arg u "$model_uri" \
        '.matches_before[0].results[] | select(.this == $u) | .fields.name // empty' \
      2>/dev/null || return 0
}

# Resolve a display:<view-name> checkpoint to a navigable URL.
#
# Strategy:
# 1. Push the repo so the browser side sees the current branch.
# 2. Resolve the view name to the concept name of its model
#    (`view_model` above).
# 3. Build the URL as that model's directory route:
#      /space/<SPACE_NAME>/<model>
#
# Falls back to the view name as the model name when the lookup fails,
# and says so on stderr — the URL still resolves, but likely to an
# error state. Failures append to MISSING, never fatal.
resolve_display() {
  local view_name="$1"
  local site="$RUN_DIR/site"
  # Canonical spot storage puts the repository directly under the site
  # directory — there is no `.tonk` level to look for.
  if [ ! -d "$site" ]; then
    echo "shots: no site at $site — skipping display:$view_name" >&2
    return 1
  fi

  # Step 1: push, so the browser side sees the current branch.
  local push_stderr
  push_stderr="$(mktemp)"
  "$TONK" push >/dev/null 2>"$push_stderr" || {
    echo "shots: tonk push failed: $(cat "$push_stderr")" >&2
    rm -f "$push_stderr"
    return 1
  }
  rm -f "$push_stderr"

  # Step 2: resolve the model concept name from the view's name.
  local model_name
  model_name="$(view_model "$view_name")"
  if [ -n "$model_name" ]; then
    echo "shots: display:$view_name → model concept '$model_name'" >&2
  else
    model_name="$view_name"
    echo "shots: display:$view_name → no model resolved, using '$model_name'" >&2
  fi

  # Step 3: build the display URL as the model's directory route:
  #   /space/<SPACE_NAME>/<model>
  # This matches the `/{*model}` route (core.yaml), which mounts
  # <tonk-display model=<model>> with no entity — directory mode, which
  # resolves the model's view and renders its instances. The older
  # `<model>!tonk:view` bang form no longer routes: `!` only has meaning
  # in the `{entity}@{model}!{view}` route, which requires an `@entity`
  # segment, so a bare `<model>!tonk:view` fell through to the directory
  # route with the whole literal string as the model name and matched no
  # concept.
  printf '%s/space/%s/%s' "$BENCH_URL" "$SPACE_NAME" "$model_name"
}

n=0
while IFS= read -r line; do
  case "$line" in ''|'#'*) continue ;; esac
  n=$((n + 1))
  if [ "$line" = "home" ]; then
    url="$BENCH_URL/"
    name="$(printf '%02d-home' "$n")"
  elif [ "$line" = "space" ]; then
    url="$BENCH_URL/space/$SPACE_NAME/"
    name="$(printf '%02d-space' "$n")"
  elif printf '%s' "$line" | grep -q '^display:'; then
    view_name="${line#display:}"
    name="$(printf '%02d-display-%s' "$n" "$view_name")"
    url="$(resolve_display "$view_name")" || {
      echo "$name display:$view_name" >> "$RUN_DIR/shots/MISSING"
      continue
    }
  else
    url="$BENCH_URL/space/$SPACE_NAME/$line"
    name="$(printf '%02d-%s' "$n" "$(printf '%s' "$line" | tr '/?=&!:' '------')")"
  fi
  echo "shots: $url" >&2
  if "$B" goto "$url" && "$B" wait-render && "$B" shot "$RUN_DIR/shots/$name.png"; then
    :
  else
    echo "$name $url" >> "$RUN_DIR/shots/MISSING"
  fi
done < "$SCENARIO/checkpoints"

# Reference render of the original artifact, when the scenario has one.
if [ -f "$SCENARIO/fixtures/artifact.html" ]; then
  # wait-doc: wait only for document.readyState complete — no tonk-host.
  wait_doc() {
    local timeout=10
    for _ in $(seq 1 $((timeout * 2))); do
      out="$("$B" eval "document.readyState === 'complete'" 2>&1 || true)"
      [ "$out" = "true" ] && return 0
      sleep 0.5
    done
    echo "shots: timed out waiting for doc ready" >&2
    return 1
  }
  if "$B" goto "file://$SCENARIO/fixtures/artifact.html" \
    && wait_doc \
    && "$B" shot "$RUN_DIR/shots/reference.png"; then
    :
  else
    echo "reference file://$SCENARIO/fixtures/artifact.html" >> "$RUN_DIR/shots/MISSING"
  fi
fi
