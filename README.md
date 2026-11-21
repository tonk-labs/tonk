# Tonk

Tonk is a local-first workspace where agents and people work on the same data.
Give an agent scoped access to a **space** and it can query or update the same
records that people see in the browser. For example, an agent can update a
project's tasks while teammates use a live board to review and edit them.

A space is a [Dialog DB](https://github.com/dialog-db/dialog-db) repository.
It stores data, schemas, rules and reactive views together, with a signed
history of changes. Data is stored as entity–attribute–value claims;
*concepts* give those claims a queryable shape. A view is an HTML template
that updates as claims change, and rules turn commands into changes to those
claims. Clients work on local replicas and sync with an upstream when
connected. Sharing links carry capabilities, such as read or read-write access.

## Try it

- **Web:** [Tonk](https://tonk.network). See the [guide](https://tonk.network/guide/) for a worked example, from modeling a counter to wiring up a button.
- **Desktop:** [Tonk Nightly for Apple Silicon](https://github.com/tonk-labs/tonk/releases/download/tonk-desktop-dev/Tonk-macos-arm64.zip) (macOS 15+). Unzip and move **Tonk Nightly.app** to Applications. Nightly uses `tonk.foundation` by default and keeps separate local data from the stable app.
- **CLI:** Install on Apple Silicon macOS or x86_64 Linux:

  ```sh
  curl -fsSL https://github.com/tonk-labs/tonk/releases/latest/download/install.sh | sh
  ```

  The installer verifies the release checksum and installs to `/usr/local/bin`
  or `~/.local/bin`. Add the latter to your `PATH` if needed. Use
  `tonk update` to update a script-installed binary on its recorded channel.
  To install a different channel, use `TONK_CHANNEL=staging sh` or
  `TONK_CHANNEL=dev sh` in place of `sh`. See [releases](https://github.com/tonk-labs/tonk/releases) for direct downloads.

With the CLI, create a local space, define a concept, and write and query a record:

```sh
tonk space new garden
tonk concept add task --field title:text:one --description "A task to do"
tonk eval -c 'task!: {title: Water the garden}'
tonk query task
```

The last command should return **Water the garden**. Creating the space also
binds the current directory to it; the replica lives in Tonk's data directory.
Use `tonk help tutorial` for a walkthrough or `tonk help notation` for the
YAML-based query and write language.

## Connect an agent

Use the CLI in a local coding agent (`tonk help agent`), or use the hosted
MCP endpoint at `https://tonk.network/mcp`. The hosted connection starts with
approval in Tonk: choose the space and grant read or read-write access.
Agents can query the granted space and, with write access, evaluate changes.

[Add Tonk to Claude](https://claude.ai/customize/connectors?modal=add-custom-connector&connectorName=Tonk&connectorUrl=https%3A%2F%2Ftonk.network%2Fmcp)
or follow the [MCP setup guide](https://tonk-labs.github.io/docs/mcp.html) for
other clients, including ChatGPT custom apps in developer mode. For the separate
per-call agent-link protocol,
see [the agent-link contract](mcp/AGENT-LINKS.md); treat those links as credentials.

## Run Tonk locally or on your own infrastructure

Install [Nix](https://nixos.org/download/) with flakes enabled, then start the
web app and its local access service from this repository:

```sh
git clone https://github.com/tonk-labs/tonk.git
cd tonk
ACCESS_STATE_DIR="$PWD/.tonk-dev/access" nix develop . -c dev:web
```

Open [http://localhost:8080](http://localhost:8080) and create a space. To
check that the frontend is proxying discovery to the local service, run this in
a second terminal:
`curl -fsS http://localhost:8080/.well-known/tonk`. It should return JSON,
not the app's HTML. The first build can take a while. The state directory
preserves the local service's signing identity, customer records and blob
snapshot across restarts. Activation email is captured locally rather than sent.

This is a local development stack, not a public deployment. To build just the
browser assets, run `nix build .#tonk-ui`; the default output is `./result`.
On another origin, serve the assets over HTTPS with SPA fallback and provide
the same-origin discovery and service routes. Serving the assets alone does not
provide accounts, remote sync or hosted MCP. For a deployment with those
services, you'll need an [access service](rust/tonk-access-service/README.md)
with its own R2, D1, KV and Durable Object bindings, an account provider, and
the [hosted MCP worker](rust/tonk-host-worker/README.md) if you want remote MCP.
See the [deployment workflow](.github/workflows/publish.yml) for how they fit
together. The checked-in [Wrangler config](wrangler.toml) targets Tonk Labs'
resources; don't deploy it unchanged.

## Develop Tonk

This repository contains the Rust storage and sync stack, web app, CLI and MCP
integration, plus the macOS desktop app. See [architecture and crate map](ARCHITECTURE.md)
and the [data model](guide/src/the-model.md).

```sh
nix develop . -c test:native:debug
nix develop . -c test:web:debug
nix develop . -c lint
nix develop . -c dev:desktop  # macOS
```

Run `nix develop` for the full command list. For builds without Nix, see
[Wasm browser tests](docs/wasm-tests-without-nix.md).

[Website](https://tonk.xyz) · [Updates](https://tonk.xyz/updates) ·
[For agents](https://tonk.xyz/llms.txt) · [Discord](https://discord.com/invite/hBPQ9xPWF7)

[MIT](LICENSE) © Tonk Labs
