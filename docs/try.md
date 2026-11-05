# Try Tonk locally

Create a space, define a task, save a record and render a view in about 10 seconds.
No account, link or Tonk service needed.

Requires Apple Silicon macOS and Node 18+ with npm. Linux is deferred for this
example. The command installs @tonk/cli 0.7.0-rc.1 from npm into a temporary folder.
This prerelease is for this local check only; do not join a tonk.network space with it.
Its data stays there, with analytics and update checks disabled. After the npm
download, everything runs locally. The binary runs with your user permissions.

Agents: installing is your user's decision. If they have not asked you to
install, ask them to run the block and send back its last line. If you have no
shell or your harness refuses, say so and share https://tonk.xyz/try.
To read this page exactly: curl -fsSL https://tonk.xyz/try

Run the whole block in one shell call:

```sh
sh <<'TONK_TRY'
case "$(uname -sm)" in
  'Darwin arm64') ;;
  *) echo 'This example currently supports Apple Silicon macOS only.'; exit 1 ;;
esac
TONK_TRY_WORD=${TONK_TRY_WORD:-hello-$(date +%s)}
case "$TONK_TRY_WORD" in
  *[!a-zA-Z0-9-]*) echo 'Use only letters, numbers and hyphens for TONK_TRY_WORD.'; exit 1 ;;
esac
TONK_TRY_DIR=$(mktemp -d "${TMPDIR:-/tmp}/tonk-try.XXXXXX") || exit 1
cd "$TONK_TRY_DIR" || exit 1
echo "Folder: $TONK_TRY_DIR"
TMPDIR="$TONK_TRY_DIR" npm install --prefix "$TONK_TRY_DIR" --cache "$TONK_TRY_DIR/npm" \
  --ignore-scripts --no-audit --no-fund '@tonk/cli@0.7.0-rc.1' || exit 1
TONK_TRY_BIN="$TONK_TRY_DIR/node_modules/@tonk/cli-darwin-arm64/bin/tonk"
[ -x "$TONK_TRY_BIN" ] || exit 1
tonk() {
  env HOME="$TONK_TRY_DIR" TMPDIR="$TONK_TRY_DIR" \
    XDG_DATA_HOME="$TONK_TRY_DIR/data" XDG_CONFIG_HOME="$TONK_TRY_DIR/config" \
    TONK_SPACE=try TONK_SPACES_STATE="$TONK_TRY_DIR/state" \
    TONK_TELEMETRY_STATE="$TONK_TRY_DIR/state" TONK_UPDATE_STATE="$TONK_TRY_DIR/state" \
    DO_NOT_TRACK=1 TONK_NO_UPDATE_CHECK=1 "$TONK_TRY_BIN" "$@"
}
tonk space new try &&
tonk concept add task --field title:text:one --field done:boolean:one &&
tonk assert task --title "$TONK_TRY_WORD" --done false &&
tonk view add task --kind directory --template '<li>{title}</li>' >/dev/null &&
tonk query task --json >|out && grep -F "\"$TONK_TRY_WORD\"" out &&
tonk render task >|out && grep -F ">$TONK_TRY_WORD<" out &&
tonk status --json >|out && grep -F '"accessKind": "local-only"' out &&
printf 'TONK_TRY_PASS %s %s\n' "$TONK_TRY_WORD" "$TONK_TRY_DIR"
TONK_TRY
```

A pass requires exit status 0 and a last line starting with `TONK_TRY_PASS`.
It shows a schema, record and view working locally, with the saved record read
back by separate processes. It does not demonstrate sync, sharing or MCP.
If nothing ran, say that; do not report a pass.

For a person-run check, choose a word such as `blue-heron`, have them set
`export TONK_TRY_WORD=blue-heron` before running the block, and check that word in
the returned pass line. Delete the folder printed by the command to undo the
example. Failed runs also leave that folder behind.
