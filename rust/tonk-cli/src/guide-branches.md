# Branches

A space is one repository, and a repository holds many named branches. Two
branches share no facts until one is merged into the other, so a branch is a
place to work without disturbing what everyone else reads.

Every space starts on `main`, its *content branch*: the roster, the space's own
name, and its invitations live there, and that is what `tonk invite` publishes
and what a joiner receives. `tonk` also keeps a `meta` branch for its own
records (remotes, tracking links); it is listed but cannot be checked out,
merged, or deleted.

## The checkout

`tonk branch` lists the branches and marks the *checkout* with `*`. The
checkout is the branch every other command reads and writes — `assert`,
`query`, `eval`, `render`, `push`, `pull`, `status`. Change it with
`tonk branch switch <name>`.

The checkout is recorded beside the space's data and never syncs: which branch
this device is looking at is nobody else's business, exactly as with git's
`HEAD`.

To address another branch for one command without changing the checkout, pass
`--branch <name>` or set `TONK_BRANCH`. Resolution order is `--branch`, then
`TONK_BRANCH`, then the checkout, then `main`.

```
tonk branch                      # list; * marks the checkout
tonk branch create draft         # from the checkout
tonk branch create draft --revision main
tonk branch create draft --revision origin/main
tonk branch switch draft         # or: tonk branch switch -c draft
tonk --branch main query note    # read another branch, once
```

## Start points

`--revision <REV>` names another branch to start from: one of this space's, or
`<remote>/<branch>` for one on a registered remote. There is no starting a
branch at a bare tree hash — a head is a claim signed by the session that
minted it, so `tonk` cannot mint one pointing wherever you like. Naming a
branch is the whole vocabulary of start points there is.

## Merging

`tonk branch merge <name>` merges `<name>` into the checkout. It is the same
three-way merge a pull runs: both sides' claims are integrated by causality, so
there is no conflicted state to resolve by hand and nothing to abort. A branch
with nothing the checkout lacks reports that it is already up to date.

Merging does not change what the checkout tracks.

## Syncing a branch

`tonk push` and `tonk pull` move the checkout, not `main`. A branch needs an
upstream first:

```
tonk branch set-upstream origin           # origin/<this branch>
tonk branch set-upstream origin/draft     # an explicitly named remote branch
tonk branch set-upstream main --for draft # track another local branch
```

`tonk remote set-upstream <remote>` still wires the content branch specifically,
because a remote is where the space lives rather than where one branch does.

Wiring a branch also records the tracking link the browser reads, so a branch
wired here is one the web UI keeps in sync.

## Branches in the web UI

A space URL carries the branch in front of the space:

```
/space/did:key:zSpace            # main
/space/test@did:key:zSpace       # the `test` branch
/space/test@did:key:zSpace/board # and any route within it
```

A bare space segment is `main`. Navigating to a branch that has an upstream on
the space's remote mounts it on that device and keeps it synced from then on;
navigating to one that exists nowhere shows an empty branch, because that is
what it is.

## Deleting

`tonk branch delete <name>` drops the branch and its head. A branch's commits
are reachable only through that head, so anything committed there and never
merged or pushed becomes unreachable; the command asks you to type the name
back, or takes `--yes`. The content branch, `meta`, and the checkout are
refused.
