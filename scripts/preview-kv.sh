#!/usr/bin/env bash
# Delete the KV namespaces a pull request's preview deploy created.
#
# Every preview creates three KV namespaces (`tonk-access-revocations-pr-N`,
# `tonk-access-servability-pr-N`, `tonk-access-test-inbox-pr-N`, see the
# `deploy` job in .github/workflows/publish.yml). The account holds at most
# 1000, so a preview whose namespaces outlive its pull request uses up the
# quota and every later preview deploy fails to create its own.
#
#   preview-kv.sh pr <number>   delete one pull request's namespaces
#   preview-kv.sh sweep         delete the namespaces of every closed pull
#                               request (needs GH_TOKEN and GITHUB_REPOSITORY)
#
# Needs CLOUDFLARE_API_TOKEN and CLOUDFLARE_ACCOUNT_ID. Set DRY_RUN=1 to only
# print what would be deleted.
set -euo pipefail

api="https://api.cloudflare.com/client/v4/accounts/$CLOUDFLARE_ACCOUNT_ID/storage/kv/namespaces"
pattern='^tonk-access-(revocations|servability|test-inbox)-pr-([0-9]+)$'

cf() {
  # Retried, so a transient refusal does not end a sweep halfway.
  for attempt in 1 2 3 4; do
    if curl -fsS -H "Authorization: Bearer $CLOUDFLARE_API_TOKEN" "$@"; then
      return 0
    fi
    sleep $((attempt * 2))
  done
  return 1
}

# Every preview namespace as `<id> <title>`, all pages.
preview_namespaces() {
  page=1
  while :; do
    body="$(cf "$api?per_page=100&page=$page")"
    jq -r --arg re "$pattern" '.result[] | select(.title | test($re)) | "\(.id) \(.title)"' <<<"$body"
    pages="$(jq -r '.result_info.total_pages // 1' <<<"$body")"
    [ "$page" -lt "$pages" ] || return 0
    page=$((page + 1))
  done
}

delete() {
  id="$1" title="$2"
  if [ -n "${DRY_RUN:-}" ]; then
    echo "would delete $title ($id)"
    return 0
  fi
  cf -X DELETE "$api/$id" >/dev/null
  echo "deleted $title ($id)"
}

case "${1:-}" in
  pr)
    number="${2:?usage: preview-kv.sh pr <number>}"
    preview_namespaces | while read -r id title; do
      [[ "$title" =~ $pattern ]] && [ "${BASH_REMATCH[2]}" = "$number" ] || continue
      delete "$id" "$title"
    done
    ;;
  sweep)
    # Collect first: deleting while paging would shift the pages.
    namespaces="$(preview_namespaces)"
    echo "$(grep -c . <<<"$namespaces" || true) preview namespaces"
    declare -A state=()
    while read -r id title; do
      [ -n "$id" ] || continue
      [[ "$title" =~ $pattern ]] || continue
      number="${BASH_REMATCH[2]}"
      if [ -z "${state[$number]:-}" ]; then
        state[$number]="$(gh api "repos/$GITHUB_REPOSITORY/pulls/$number" -q .state 2>/dev/null || echo unknown)"
      fi
      # An open pull request's preview is in use; one GitHub does not know
      # is left for a person to look at.
      [ "${state[$number]}" = closed ] || continue
      delete "$id" "$title"
    done <<<"$namespaces"
    ;;
  *)
    echo "usage: preview-kv.sh pr <number> | sweep" >&2
    exit 2
    ;;
esac
