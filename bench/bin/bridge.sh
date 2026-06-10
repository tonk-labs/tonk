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
    *) echo "bridge: landed on $loc" >&2; exit 0 ;;
  esac
done

echo "bridge: join did not complete within 60s" >&2
# Print current page state for debugging
"$B" eval "document.title" >&2 || true
"$B" eval "window.location.href" >&2 || true
exit 1
