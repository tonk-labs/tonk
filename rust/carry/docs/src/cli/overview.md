# CLI Reference

Carry provides a small set of commands for interacting with Dialog DB. Every command follows a consistent pattern and shares global options.

## Commands

| Command | Alias | Description |
|---|---|---|
| [`carry init`](./init.md) | `i` | Create a new repository |
| [`carry assert`](./assert.md) | `a` | Assert claims (add or update data) |
| [`carry query`](./query.md) | `q` | Query entities by domain or concept |
| [`carry retract`](./retract.md) | `r` | Retract claims (remove data) |
| [`carry status`](./status.md) | `st` | Show repository info |

## Global Options

Every command accepts:

| Flag | Description |
|---|---|
| `--repo <PATH>` | Path to a specific `.carry/` repository. Skips filesystem walk. |
| `--format <FORMAT>` | Output format: `yaml` (default), `json`, or `triples`. |

### Repo Resolution

When `--repo` is omitted, Carry walks up the filesystem tree from `$PWD` toward `$HOME`, looking for a `.carry/` directory. The first one found is used. You can also set the `CARRY_REPO` environment variable.

### Output Formats

| Format | Description | Best for |
|---|---|---|
| `yaml` | Asserted notation (default) | Human reading, file round-trips |
| `json` | Array of objects with `id` field | Programmatic consumption |
| `triples` | Flat EAV YAML (`the`/`of`/`is`) | Piping between carry commands |

The preferred format can also be persisted as a setting:

```bash
carry assert xyz.tonk.carry output-format=json
```

Command-line `--format` always takes precedence over the persisted preference.

## Target Syntax

Several commands accept a `<TARGET>` argument. The syntax is:

| Pattern | Interpretation | Example |
|---|---|---|
| Contains `.` | Domain target | `com.app.person` |
| No `.` | Concept target (resolved by bookmark name) | `person` |
| `-` | Read from stdin | `-` |
| Contains `/` or ends in `.yaml`/`.yml`/`.json` | File path | `schema.yaml` |

## Field Syntax

Commands that accept fields use the format `FIELD[=VALUE]`:

| Syntax | Meaning |
|---|---|
| `name` | Projection: include this field in output |
| `name="Alice"` | Filter: only match entities where name is Alice |
| `this=did:key:z...` | Target a specific entity |
| `@myname` | Assert `dialog.meta/name` on the entity (bookmark) |

## Value Auto-Detection

When asserting values via the CLI, Carry auto-detects the type:

| Input | Detected Type |
|---|---|
| `did:key:z...` | Entity reference |
| `123` | Unsigned integer |
| `-5` | Signed integer |
| `3.14` | Float |
| `true` / `false` | Boolean |
| Anything else | Text (string) |
