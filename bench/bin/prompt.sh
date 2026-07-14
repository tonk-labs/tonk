#!/usr/bin/env bash
# Render the agent-invite prompt exactly as the product would: extract
# the <pre class="agent-prompt__pre"> body from core.yaml's
# id:agent-invite/prompt view and fill its template fields from a real
# minted invite. Copy edits in core.yaml are automatically under test —
# there is no frozen prompt.
#
# Usage: prompt.sh --invite-url <url> --name <label>
# Env: ROOT
set -euo pipefail

ROOT="${ROOT:?}"
CORE_YAML="$ROOT/rust/tonk-core/assets/library/core.yaml"
INVITE_URL="" NAME="bench"
while [ $# -gt 0 ]; do
  case "$1" in
    --invite-url) [ $# -ge 2 ] || { echo "prompt: --invite-url requires a value" >&2; exit 2; }; INVITE_URL="$2"; shift ;;
    --name) [ $# -ge 2 ] || { echo "prompt: --name requires a value" >&2; exit 2; }; NAME="$2"; shift ;;
    *) echo "prompt: unknown flag $1" >&2; exit 2 ;;
  esac
  shift
done
[ -n "$INVITE_URL" ] || { echo "prompt: --invite-url required" >&2; exit 2; }

# Escape chars special to bash pattern-substitution REPLACEMENT text:
# an unescaped `&` back-references the match, and `\` is an escape.
psub_escape() { printf '%s' "$1" | sed 's/[\\&]/\\&/g'; }

# Extract the <pre> body: start at the opening tag (dropping the tag
# itself), stop at </pre>. Then strip the 4-space YAML block indent and
# unescape the HTML entities the template carries.
raw="$(awk '
  /<pre class="agent-prompt__pre">/ { f=1; sub(/.*<pre class="agent-prompt__pre">/, ""); print; next }
  f && /<\/pre>/ { sub(/<\/pre>.*/, ""); print; exit }
  f { print }
' "$CORE_YAML" | sed -e 's/^    //' -e 's/&amp;/\&/g' -e 's/&quot;/"/g')"

[ -n "$raw" ] || { echo "prompt: extraction from $CORE_YAML came up empty — did the agent-prompt view template change shape?" >&2; exit 1; }

# Fill the template. The join URL is one composite placeholder; replace
# it whole with the real minted invite URL. Use escaped copies as the
# replacement text so a literal `&` (e.g. a synced repo's `&remote=`)
# isn't treated as a back-reference to the matched pattern.
esc_url="$(psub_escape "$INVITE_URL")"
esc_name="$(psub_escape "$NAME")"
filled="$raw"
filled="${filled//\{dom.host\/data-base\}?access=\{access\}\{remote\}#\{code\}/$esc_url}"
filled="${filled//\{name\}/$esc_name}"
filled="${filled//\{dom.host\/data-page\}/this repo}"

# Self-check: no unfilled placeholders may survive; the join command
# must be present. Fail loudly — a silently wrong prompt poisons runs.
if printf '%s' "$filled" | grep -qE '\{(access|remote|code|name|dom\.host)' ; then
  echo "prompt: unfilled placeholder survived:" >&2
  printf '%s\n' "$filled" | grep -nE '\{' >&2
  exit 1
fi
printf '%s' "$filled" | grep -q "tonk join" || { echo "prompt: no 'tonk join' in rendered prompt" >&2; exit 1; }

printf '%s\n' "$filled"
