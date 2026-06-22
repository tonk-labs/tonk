#!/usr/bin/env bash
# Make the browser a member of the run's space: mint an invite from
# the slide site, open it, and wait until the space route renders. The
# join component auto-joins (no name form — a joined space is addressed
# by its repository DID, returned as repository.name), so the only job
# here is to navigate to the invite and confirm we land off /join, then
# pull the data so the shots don't race the background sync.
#
# Join component states:
#   Loading          — spinner
#   (auto-join)      — claims the invite, then JS-navigates to /space/<DID>
#   InvalidInvite    — error shown
#   AudienceMismatch — error shown
#
# SPACE_NAME is the repository DID (set by run.sh from site.sh's
# space.did); it addresses both the pull endpoint and the shot URLs.
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

# The join component auto-joins: it claims the invite and JS-navigates
# to /space/<repository-DID> with no form interaction. Poll until the
# URL leaves /join.
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
