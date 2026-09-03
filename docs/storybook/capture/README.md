# Visual capture protocol

Screenshots are review evidence attached to a stable screen ID. They are not
proof that every variant or interrupt in the linked journeys works.

## Evidence labels

- **Running product:** captured from the built Tonk browser shell with real
  browser input. Use this for Hub, space, routing, and ordinary reachable
  account states.
- **Production-source fixture:** the checked-in production HTML and CSS are
  loaded directly and populated with documented fixture values. Use this for a
  state that otherwise requires an external passkey, email, another device, or
  destructive account data. It proves the current authored appearance, not the
  runtime transition into the state.
- **Captured CLI output:** stdout and stderr came from the binary at the visual
  commit, in an isolated data directory. The capture page only gives that text
  a stable terminal viewport.

The evidence label is visible in the Storybook. Never call a fixture a running
product capture.

## Browser capture

1. Start the repository web environment with `nix develop . -c dev:web`. If
   the caller exports `NO_COLOR=1`, remove that variable for this command;
   mdBook accepts only `true` or `false` for its `--no-color` environment
   binding.
2. Use a fresh, isolated browser profile. Set the viewport to 1440 by 960 and
   device scale factor to 1.
3. Reach the state with normal browser input and wait for fonts, service worker,
   and any visible sync work to settle.
4. Capture the viewport, not a cropped component. Keep browser chrome out.
5. Copy the generated artifact to `app/screens/{screen-id}-{slug}.png` and set
   the screen's `visual_commit`, source ownership, and evidence label in
   `screens.json`.

For account and activation states that cannot be reached safely, serve the
repository root and open `capture/fixture.html?screen=WEB-11` (or another
supported ID: `WEB-10` through `WEB-15`). The fixture fetches
`rust/tonk-workspace/src/ui_account_settings.html`, `rust/tonk-ui/src/activate.html`,
and `account.css` directly; it does not keep copied product markup. Sign-up,
log-in, and link-an-account are the registration cluster the hub raises and
are captured from the running product.

## CLI capture

1. Build the current binary and use a temporary `XDG_DATA_HOME`,
   `TONK_SPACES_STATE`, telemetry/update state, and working directory. Never
   replace `HOME`, and never capture a developer's real account or spaces.
2. Run the command with color disabled and a fixed 96-column terminal. Record
   the command, stdout, stderr, and exit class in `capture/cli/{screen-id}.txt`.
3. Serve the repository root and open
   `capture/fixture.html?screen={screen-id}`. The fixture fetches that exact
   transcript into the stable terminal viewport.
4. Capture to the matching file under `app/screens/`.

## Review and update rule

After any user-facing `tonk-ui` or `tonk-cli` change:

1. Find the affected journey and screen IDs in the explorer.
2. Update prose, variants, verification rows, or triage if behavior changed.
3. Recapture affected artifacts if pixels or terminal output changed.
4. Run `python3 docs/storybook/scripts/build.py` and then `--check`.
   `test:storybook` performs the checked-data and local-link pass from the
   repository development shell.
5. In the pull request, state either `Storybook updated: {IDs}` or
   `Storybook impact: none — {reason}`.

When a bug is found, add or update a journey/check/triage item before closing
the fix. The regression test and Storybook item should cite the same stable ID.
