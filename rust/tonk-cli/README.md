# tonk

A local-only CLI for reading and writing tonk facts via asserted-notation.

`tonk` is the headless companion to tonk-ui: it operates on a `.tonk/` site
(a single dialog repository named `main`, working on the `main` branch) without
a browser. The mutating verb is `eval`, which runs a notation document through
the analyze → query → plan → commit pipeline. The other subcommands are
read-only introspection, one-shot setup, sync, and sharing helpers. The crate
also exposes a small library surface (`tonk::eval`, `tonk::site`, …) so
integration tests and SDK consumers can drive the same code paths as the binary.

## Usage

```sh
# Initialize a .tonk/ repo in the current directory.
tonk init

# Evaluate a notation document: inline, from a file, or piped.
tonk eval -c 'person:'
tonk eval ./doc.notation
cat doc.notation | tonk eval -
tonk eval -c 'person:' --format json --quiet

# Inspect the branch.
tonk schema       # every named attribute + concept as re-submittable notation
tonk concepts     # user-defined concepts: name<TAB>description
tonk views        # entities with a text/html claim: name<TAB>entity<TAB>bytes
tonk guide        # baked-in asserted-notation reference (also: guide notation|views|all)

# CSV transfer over the main branch.
tonk export --out data.csv
tonk import data.csv

# Remotes and sync.
tonk remote add prod https://access.example.com
tonk remote set-upstream prod
tonk push
tonk pull
tonk status       # synced | ahead | behind | diverged | no-upstream

# Delegate access and share live views.
tonk invite --remote prod
tonk join 'https://...#invite'
tonk share concept person
tonk share view my-page
tonk share display alice --view person-card
```

## How it works

### The `.tonk/` site

A site is a working directory containing a `.tonk/` sub-directory that holds one
dialog repository (`main`), opened on the `main` branch. Multi-branch and
multi-repo workflows are intentionally not exposed. Most commands call
`TonkSite::discover_and_open`, which walks up from the current directory to find
`.tonk/` and assembles the working context: the user's profile, an operator
rooted at `.tonk/`, the repository handle, and the opened branch. The local
identity is a shared profile (`tonk identity` prints its DID; `--reset` mints a
fresh one).

### The eval pipeline

`tonk eval` resolves its source (inline `-c`, a path, `-` or piped stdin),
opens the site, and drives `tonk_evaluator::evaluate` against the `main`
branch's transaction. The evaluator analyzes the notation, runs the synthesized
queries, stages mutations, and fires installed effects, yielding a transaction
that tonk commits. The response is rendered as YAML notation (default) or JSON;
`--quiet` drops the matches section and emits only the envelope. Exit codes are
distinct per failure stage (`ParseError`, `AnalyzeError`, `CommitError`,
`IoError`) so agent harnesses can branch without parsing stderr.

When an upstream is configured, a committing eval is wrapped with an automatic
pull-before / push-after. `--no-sync` (or `TONK_NO_SYNC`) skips it; manual
`tonk push` / `tonk pull` stay available either way.

### Sync and sharing

`push` / `pull` are fast-forward sync over `Branch::push()` / `Branch::pull()`,
with errors that name the upstream-not-configured and non-fast-forward cases.
`status` classifies the local branch against its upstream without merging.

Remotes are UCAN-S3 access services registered on the repository's meta branch.
`tonk invite` mints a UCAN delegation chain over the repo and prints an
audience-open invite URL (anyone holding it can claim by redelegating from the
embedded ephemeral key); `tonk join` claims one into a fresh `.tonk/`. The
`share` subcommands push to the upstream, mint an invite that embeds the
upstream URL, and append a launcher URL that lands the recipient on a live view
of the data: an auto-rendered concept route (`share concept`), the iframe HTML
viewer (`share view`), or a `<tonk-display>` declarative view (`share display`).

## Built on

`tonk` drives documents through `tonk-evaluator` (analyze → compile → evaluate),
parses with `tonk-notation`, reads schema types from `tonk-schema`, builds
invites with `tonk-invite`, and talks to dialog repositories, storage, UCAN
credentials, and the UCAN-S3 remote through the `dialog-*` crates.
