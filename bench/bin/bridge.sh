#!/usr/bin/env bash
# Make the browser a member of the run's space: mint an invite from
# the slide site, open it, fill the join form's name input (wa-input
# Web Awesome custom element: set .value + dispatch input event on the
# custom element; the Leptos on:input handler reads from event.target.value),
# submit the form, and wait until the space route renders. The single
# UI interaction in the whole harness.
#
# Join component states:
#   Loading         — spinner, no form
#   NewMember       — form with <wa-input name="space-name"> + <wa-button type="submit">
#   AlreadyMember   — auto-joins via JS navigate, no form rendered
#   InvalidInvite   — error shown, no form
#   AudienceMismatch — error shown, no form
#
# Env: ROOT, RUN_DIR, BENCH_URL, SPACE_NAME
set -euo pipefail

ROOT="${ROOT:?}"
RUN_DIR="${RUN_DIR:?}"
BENCH_URL="${BENCH_URL:?}"
SPACE_NAME="${SPACE_NAME:-bench}"
B="$ROOT/bench/bin/browser.sh"

INVITE_URL="$("$ROOT/bench/bin/site.sh" invite | tr -d '[:space:]')"
echo "bridge: raw invite url: $INVITE_URL" >&2

# Rewrite the origin to the local stack if the minted URL's host differs.
# Preserve any #fragment — it carries the ephemeral seed for open invites.
case "$INVITE_URL" in
  "$BENCH_URL"*) ;;
  *)
    # Extract path+query+fragment by stripping the scheme+host prefix.
    PATH_QF="$(printf '%s' "$INVITE_URL" | sed -E 's#^https?://[^/]+##')"
    INVITE_URL="$BENCH_URL$PATH_QF"
    ;;
esac

echo "bridge: navigating to $INVITE_URL" >&2
"$B" goto "$INVITE_URL"
"$B" wait-render
"$B" wait-sw

# After the service worker controls the page it will reload once;
# wait for render again so the join component is fully hydrated.
"$B" wait-render

# Detect the current join state before attempting to fill the form.
# wa-input is a Web Awesome custom element — its .value property is
# what the Leptos on:input handler reads from event.target.value.
FILL_RESULT="$("$B" eval "(() => {
  const waInput = document.querySelector('wa-input[name=\"space-name\"]');
  if (!waInput) return 'no-input';
  const setter = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(waInput), 'value')
    || Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'value');
  if (setter && setter.set) {
    setter.set.call(waInput, '$SPACE_NAME');
  } else {
    waInput.value = '$SPACE_NAME';
  }
  waInput.dispatchEvent(new Event('input', { bubbles: true }));
  return 'filled';
})()")"

echo "bridge: fill result: $FILL_RESULT" >&2

# Submit the form. wa-button[type=submit] inside a form triggers
# form submission when clicked; also try form.requestSubmit() as backup.
SUBMIT_RESULT="$("$B" eval "(() => {
  const form = document.querySelector('form');
  if (form) {
    try { form.requestSubmit(); return 'submitted-requestSubmit'; } catch(_) {}
    const btn = form.querySelector('wa-button[type=\"submit\"]');
    if (btn) { btn.click(); return 'submitted-btn-click'; }
    form.submit();
    return 'submitted-form-submit';
  }
  const btn = document.querySelector('wa-button[type=\"submit\"]');
  if (btn) { btn.click(); return 'submitted-btn-outer'; }
  return 'no-form';
})()")"

echo "bridge: submit result: $SUBMIT_RESULT" >&2

# On the AlreadyMember fast-path, no-input/no-form is expected — the
# component auto-navigates without rendering any form. Either way,
# poll until the URL leaves /join.
for _ in $(seq 1 60); do
  loc="$("$B" eval "window.location.pathname")"
  # strip outer JSON quotes from eval output
  loc="${loc#\"}"
  loc="${loc%\"}"
  case "$loc" in
    */join*) sleep 1 ;;
    *) echo "bridge: landed on $loc" >&2; break ;;
  esac
done

# Re-check after loop — if still on /join, the join timed out.
loc="$("$B" eval "window.location.pathname")"
loc="${loc#\"}"
loc="${loc%\"}"
case "$loc" in
  */join*)
    echo "bridge: join did not complete within 60s" >&2
    "$B" eval "document.title" >&2 || true
    "$B" eval "window.location.href" >&2 || true
    exit 1
    ;;
esac

# After the join navigation lands us on the space route, the SW must
# be controlling the page before the /api pull XHR will be intercepted.
# wait-sw issues a reload if the SW isn't controlling yet and polls
# until it is.
"$B" wait-render
"$B" wait-sw

# Poll the pull endpoint until the response confirms success, or we
# time out (~30 s). join.rs calls pull already, but racing the
# service-worker startup means it may land before the worker is ready;
# polling here is cheap (no-op when already at HEAD) and makes the
# data-visible guarantee tight.
#
# The pull endpoint is handled by the service worker (not Caddy), so
# we fire it from the page via an async fetch() (eval-async): Chrome
# does not route synchronous XHR through async SW fetch handlers.
echo "bridge: waiting for pull to confirm data for space $SPACE_NAME..." >&2
pull_confirmed=0
for i in $(seq 1 60); do
  pull_raw="$("$B" eval-async "(function(done){fetch('/api/repository/$SPACE_NAME/branch/main/sync/pull', {method:'POST'}).then(function(r){return r.text().then(function(t){done(r.status+':'+t.slice(0,200));});}).catch(function(e){done('err:'+String(e));});})(arguments[0])" 2>/dev/null || true)"
  pull_raw="${pull_raw#\"}"
  pull_raw="${pull_raw%\"}"
  case "$pull_raw" in
    200:*'"success":true'*) echo "bridge: pull confirmed (poll $i)" >&2; pull_confirmed=1; break ;;
    200:*) echo "bridge: pull ok but success missing (poll $i): $pull_raw" >&2; pull_confirmed=1; break ;;
    *) echo "bridge: pull poll $i: $pull_raw" >&2; sleep 0.5 ;;
  esac
done
if [ "$pull_confirmed" -eq 0 ]; then
  echo "bridge: data sync did not confirm" >&2
  exit 1
fi
echo "bridge: done" >&2
