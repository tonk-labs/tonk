# slide

A local-only CLI for reading and writing tonk facts via asserted-notation.

`slide` is the headless companion to tonk-ui: it operates on a `.tonk/` site
(a single dialog repository named `main`, working on the `main` branch) without
a browser. The mutating verb is `eval`, which runs a notation document through
the analyze → query → plan → commit pipeline. The other subcommands are
read-only introspection, one-shot setup, sync, and sharing helpers. The crate
also exposes a small library surface (`slide::eval`, `slide::site`, …) so
integration tests and SDK consumers can drive the same code paths as the binary.

## Usage

```sh
# Initialize a .tonk/ repo in the current directory.
slide init

# Evaluate a notation document: inline, from a file, or piped.
slide eval -c 'person:'
slide eval ./doc.notation
cat doc.notation | slide eval -
slide eval -c 'person:' --format json --quiet

# Inspect the branch.
slide schema       # every named attribute + concept as re-submittable notation
slide concepts     # user-defined concepts: name<TAB>description
slide views        # entities with a text/html claim: name<TAB>entity<TAB>bytes
slide guide        # baked-in asserted-notation reference (also: guide notation|views|all)

# CSV transfer over the main branch.
slide export --out data.csv
slide import data.csv

# Remotes and sync.
slide remote add prod https://access.example.com
slide remote set-upstream prod
slide push
slide pull
slide status       # synced | ahead | behind | diverged | no-upstream

# Delegate access and share live views.
slide invite --remote prod
slide join 'https://...#invite'
slide share concept person
slide share view my-page
slide share display alice --view person-card
```

## How it works

### The `.tonk/` site

A site is a working directory containing a `.tonk/` sub-directory that holds one
dialog repository (`main`), opened on the `main` branch. Multi-branch and
multi-repo workflows are intentionally not exposed. Most commands call
`SlideSite::discover_and_open`, which walks up from the current directory to find
`.tonk/` and assembles the working context: the user's profile, an operator
rooted at `.tonk/`, the repository handle, and the opened branch. The local
identity is a shared profile (`slide identity` prints its DID; `--reset` mints a
fresh one).

### The eval pipeline

`slide eval` resolves its source (inline `-c`, a path, `-` or piped stdin),
opens the site, and drives `tonk_evaluator::evaluate` against the `main`
branch's transaction. The evaluator analyzes the notation, runs the synthesized
queries, stages mutations, and fires installed effects, yielding a transaction
that slide commits. The response is rendered as YAML notation (default) or JSON;
`--quiet` drops the matches section and emits only the envelope. Exit codes are
distinct per failure stage (`ParseError`, `AnalyzeError`, `CommitError`,
`IoError`) so agent harnesses can branch without parsing stderr.

When an upstream is configured, a committing eval is wrapped with an automatic
pull-before / push-after. `--no-sync` (or `SLIDE_NO_SYNC`) skips it; manual
`slide push` / `slide pull` stay available either way.

### Sync and sharing

`push` / `pull` are fast-forward sync over `Branch::push()` / `Branch::pull()`,
with errors that name the upstream-not-configured and non-fast-forward cases.
`status` classifies the local branch against its upstream without merging.

Remotes are UCAN-S3 access services registered on the repository's meta branch.
`slide invite` mints a UCAN delegation chain over the repo and prints an
audience-open invite URL (anyone holding it can claim by redelegating from the
embedded ephemeral key); `slide join` claims one into a fresh `.tonk/`. The
`share` subcommands push to the upstream, mint an invite that embeds the
upstream URL, and append a launcher URL that lands the recipient on a live view
of the data: an auto-rendered concept route (`share concept`), the iframe HTML
viewer (`share view`), or a `<tonk-display>` declarative view (`share display`).

## Built on

`slide` drives documents through `tonk-evaluator` (analyze → compile → evaluate),
parses with `tonk-notation`, reads schema types from `tonk-schema`, builds
invites with `tonk-invite`, and talks to dialog repositories, storage, UCAN
credentials, and the UCAN-S3 remote through the `dialog-*` crates.
