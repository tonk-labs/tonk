#!/usr/bin/env bash
# Capture the scenario's checkpoint screenshots. Each line of the
# scenario's `checkpoints` file is either:
#   home                — the shell root ($BENCH_URL/)
#   display:<view-name> — resolved at capture time via `slide share display`
#   <path>              — a suffix under /space/$SPACE_NAME/
# Blank lines and #-comments are skipped. A failed screenshot is
# recorded as missing, not fatal — the judge sees what there is.
#
# Env: ROOT, RUN_DIR, BENCH_URL, SPACE_NAME, SCENARIO (scenario dir)
set -euo pipefail

ROOT="${ROOT:?}"; RUN_DIR="${RUN_DIR:?}"; BENCH_URL="${BENCH_URL:?}"
SPACE_NAME="${SPACE_NAME:-bench}"; SCENARIO="${SCENARIO:?}"
B="$ROOT/bench/bin/browser.sh"
SLIDE="$ROOT/target/release/slide"
mkdir -p "$RUN_DIR/shots"

# URL-decode a percent-encoded string (handles %XX sequences).
urldecode() {
  local encoded="$1"
  printf '%b' "$(printf '%s' "$encoded" | sed 's/%/\\x/g')"
}

# Resolve a display:<view-name> checkpoint to a navigable URL.
#
# Strategy:
# 1. Run `slide share display <view-name> --view <view-name>` to confirm
#    the view bookmark exists and push the repo. Parse the view entity
#    URI from stderr.
# 2. Query the view instance to find its model URI and resolve that to a
#    concept name via the eval JSON API.
# 3. Build the URL directly in tonk-ui display-route format:
#      /space/<SPACE_NAME>/<model>!<view>
#    The `{model}!{view}` subject encoding puts `<tonk-display>` into
#    directory mode (all instances of <model>) using the named view
#    template.
#
# Falls back to `<view-name>!<view-name>` when the model lookup fails —
# the URL resolves but may render an error state. Failures append to
# MISSING, never fatal.
resolve_display() {
  local view_name="$1"
  local site="$RUN_DIR/site"
  if [ ! -d "$site/.tonk" ]; then
    echo "shots: no site at $site — skipping display:$view_name" >&2
    return 1
  fi

  # Step 1: confirm view exists and push. Capture the view entity URI
  # from stderr ("subject: <name> (<entity>)" or "subject: <entity>").
  local share_stderr view_entity
  share_stderr="$(mktemp)"
  cd "$site" && "$SLIDE" share display "$view_name" --view "$view_name" \
    >"$share_stderr.stdout" 2>"$share_stderr" || {
    echo "shots: slide share display $view_name --view $view_name failed: $(cat "$share_stderr")" >&2
    rm -f "$share_stderr" "$share_stderr.stdout"
    return 1
  }
  view_entity="$(grep '^subject:' "$share_stderr" | head -1 \
    | grep -oE '\(did:key:[^)]+\)' | tr -d '()' || true)"
  if [ -z "$view_entity" ]; then
    view_entity="$(grep '^subject:' "$share_stderr" | head -1 | awk '{print $2}' || true)"
  fi
  rm -f "$share_stderr" "$share_stderr.stdout"

  # Step 2: resolve the model concept name from the view entity URI.
  local model_name="$view_name"
  if [ -n "$view_entity" ] && command -v jq >/dev/null 2>&1; then
    local model_uri
    model_uri="$(cd "$site" && "$SLIDE" eval --no-sync --format json -c 'view:' 2>/dev/null \
      | jq -r --arg e "$view_entity" \
          '.matches_before[0].results[] | select(.this == $e) | .fields.model // empty' \
        2>/dev/null || true)"
    if [ -n "$model_uri" ]; then
      local resolved
      resolved="$(cd "$site" && "$SLIDE" eval --no-sync --format json -c 'concept:' 2>/dev/null \
        | jq -r --arg u "$model_uri" \
            '.matches_before[0].results[] | select(.this == $u) | .fields.name // empty' \
          2>/dev/null || true)"
      if [ -n "$resolved" ]; then
        model_name="$resolved"
        echo "shots: display:$view_name → model concept '$model_name'" >&2
      fi
    fi
  fi

  # Step 3: build the display URL in tonk-ui format:
  #   /space/<SPACE_NAME>/<model>!tonk:view
  # `<tonk-display model=<model> view=tonk:view>` resolves the view
  # concept (tonk:view) to find the view instance whose model field
  # matches, which is the view the agent defined. The view anchor name
  # is not the right value for the `view` attribute — tonk:view is the
  # concept that owns the view instance.
  printf '%s/space/%s/%s!tonk:view' "$BENCH_URL" "$SPACE_NAME" "$model_name"
}

n=0
while IFS= read -r line; do
  case "$line" in ''|'#'*) continue ;; esac
  n=$((n + 1))
  if [ "$line" = "home" ]; then
    url="$BENCH_URL/"
    name="$(printf '%02d-home' "$n")"
  elif printf '%s' "$line" | grep -q '^display:'; then
    view_name="${line#display:}"
    name="$(printf '%02d-display-%s' "$n" "$view_name")"
    url="$(resolve_display "$view_name")" || {
      echo "$name display:$view_name" >> "$RUN_DIR/shots/MISSING"
      continue
    }
  else
    url="$BENCH_URL/space/$SPACE_NAME/$line"
    name="$(printf '%02d-%s' "$n" "$(printf '%s' "$line" | tr '/?=&' '----')")"
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
