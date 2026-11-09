# Release readiness

This is the observation phase. **Existing tag-triggered publishers still operate;
these reports do not enforce production release gates.** Do not interpret a green
report or Promotion preflight as a release. Full promotion is deferred until
installers/updaters use a protected channel manifest and a release App exists.

After these workflows land on main, use GitHub Actions → **Release readiness**.
Select main as the workflow branch, a full source SHA already on main (or leave
blank for the workflow commit), and an RC version such as `0.7.0-rc.1`. Stable
candidate preparation deliberately fails until staging receipts are implemented.
The selected source must contain the new release test launcher.

The workflow builds version-stamped Linux/macOS CLI and browser artifacts,
runs regression/MCP/packaging checks and ChatGPT authorization/session/packaging
contracts (`chatgpt`), and runs three browser E2E shards against
the selected CLI and browser bytes with retries disabled. The report is pinned
to source, controller, policy, run, attempt, version and artifact digests. It
rejects missing tests/shards, flakes, skipped checks and mismatched identities.
The required automated checks table reports ChatGPT contracts separately; missing
or failed contracts block readiness. This runs for every candidate at its exact
source SHA, independently of PR path filters. Native Desktop and real ChatGPT
connectivity remain outside the required manual checklist.

Open the Actions summary or download `release-candidate-<attempt>` and open
`index.html`. JSON records accompany the report. Initial runs remain blocked
until human verification exists; this is expected. Artifacts expire after 30
days. Rerun all jobs for a new attempt; partial reruns cannot combine old and new
evidence into a pass. Never manually edit report files to approve a candidate.

## Record running-product verification

Use disposable accounts and sanitized evidence. Install the candidate CLI
artifacts and serve the candidate browser artifact using an appropriately
configured test environment. Record the source/version, origins, platforms and
artifact identity in your evidence; checking current production or a Storybook
fixture does not verify this candidate. Automatic candidate deployment and
staging identity collection are not implemented yet.

Run **Record release verification** on main with the readiness run ID, group,
observed result (`passed`, `failed` or `blocked`), HTTPS evidence URL and notes.
The workflow checks the authenticated actor's current maintain/admin permission;
it does not accept a supplied reviewer name. Verify every item in a group:

| Group | Required observations |
| --- | --- |
| accounts | Create and activate; sign in/out and switch accounts; cancel/retry; delete only the disposable account. |
| spaces | Create/edit/reload offline; sync to a second actor; share/join/revoke; public/private visibility; preserve unrelated data. |
| cli | Install/update on Linux and macOS; linking approval, decline and timeout; tool join/read/write/restart/revoke. |
| visual | Compare signup, activation, settings, hub, sharing and linking in the running candidate on desktop and narrow screens against DESIGN.md. |

Manual evidence expires after 24 hours and binds to the exact candidate digest.
A newer failed/blocked observation replaces an earlier pass. Changed artifacts,
policy or run attempt require fresh verification. Do not rerun verification jobs;
create a new dispatch so the actor and observation are unambiguous. Each
verification run publishes an updated report; the original report is immutable.

**Promotion preflight** re-reads candidate artifacts and current reviewer
permissions and checks live repository policy. It always has publishing disabled.
Its policy audit can require elevated read access unavailable to GITHUB_TOKEN;
missing access blocks the audit. The empty App/custodian/cutover configuration in
`release/github/policy.json` intentionally prevents readiness for cutover.

## Maintaining the contract

`release/flows.json` maps required flow groups to exact nextest test IDs and
Storybook journey IDs. Those IDs are navigation aids; Storybook's claimed status
is never accepted as release proof. The automated mapping covers selected parts
of each flow; the human checklist covers the broader behavior. Review additions
or deletions as changes to release coverage, not routine snapshot maintenance.

Run the focused checks with:

```sh
for test_file in scripts/tests/test-release-*.py; do
  PYTHONDONTWRITEBYTECODE=1 python3 "$test_file" || exit 1
done
python3 scripts/tests/test-cli.py
```

Do not disable the old publishers until the channel migration and full protected
promotion pilot pass. The intended single future path is main candidate →
approved staging deployment → verified staging receipt → approved stable
publication. Branch merges, fast-forwards and GitHub Release creation cannot be
made equivalent to that path by merely adding tag rules.
