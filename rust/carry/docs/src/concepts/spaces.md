# Spaces

A **space** is an isolated namespace within a `.carry/` repository. Each space has its own cryptographic identity, its own data store, and its own set of claims.

## Why Spaces?

Spaces let you keep separate workstreams within the same project without data leaking between them. You might have:

- A `main` space for production data
- A `research` space for exploratory work
- A `staging` space for testing imports

Each space is fully independent. Claims in one space are invisible to queries in another.

## Anatomy of a Space

When you run `carry init`, a space is created with:

- An **Ed25519 keypair** -- the space's cryptographic identity
- A **DID** -- derived from the public key, e.g. `did:key:zAbc123`
- A **credentials** file -- the private key, stored at `.carry/<did>/credentials` (mode 0600)
- A **facts/** directory -- the Dialog DB storage (prolly tree)
- An optional **label** -- a human-readable name like "my-project"

The directory structure looks like:

```
.carry/
  @active                    # plain text: DID of the active space
  did:key:zAbc123/
    credentials              # 32-byte Ed25519 secret key
    facts/                   # Dialog DB storage
  did:key:zDef456/
    credentials
    facts/
```

## Managing Spaces

### List spaces

```bash
carry space list
```

Output:
```
* did:key:zAbc123  my-project
  did:key:zDef456  research
```

The `*` marks the active space.

### Create a new space

```bash
carry space create research
```

Creates a new space with the label "research" and switches to it.

### Switch active space

```bash
carry space switch research
carry space switch did:key:zAbc123
```

Accepts either a label or a DID.

### Show active space

```bash
carry space active
```

### Delete a space

```bash
carry space delete research
carry space delete research --yes    # skip confirmation
```

You cannot delete the active space. Switch to a different one first.

## Targeting a Space Without Switching

Use `--space` on any command to target a specific space without changing the active space:

```bash
carry query com.app.person name --space research
carry assert com.app.person name=Alice --space research
```

This is read-only with respect to the active space setting -- the active space remains unchanged.

## Repo Discovery

When you run a Carry command without `--repo`, Carry walks up the filesystem tree from `$PWD` toward `$HOME`, looking for a `.carry/` directory. The first one found is used.

You can override this with:

- `--repo /path/to/project` -- point to a specific repository
- `CARRY_REPO` environment variable

This means you can run Carry commands from any subdirectory of your project and it will find the right repository automatically.
