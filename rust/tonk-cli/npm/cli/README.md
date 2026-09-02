# @tonk/cli

The Tonk stack command line utility — `tonk`.

```sh
npx @tonk/cli --help
# or install globally
npm install -g @tonk/cli
tonk --help
```

These commands install the final release held by Tonk's `stable` branch.
To try the newest explicitly released prerelease instead:

```sh
npx @tonk/cli@next --help
# or install globally
npm install -g @tonk/cli@next
```

`@tonk/cli` is a thin launcher. The actual `tonk` program is a native
binary shipped in a per-platform package that npm installs automatically
for your OS and CPU:

| Platform      | Package                   |
| ------------- | ------------------------- |
| macOS (arm64) | `@tonk/cli-darwin-arm64`  |
| Linux (x64)   | `@tonk/cli-linux-x64`     |

More platforms coming soon.

Source: [`rust/tonk-cli`](https://github.com/tonk-labs/tonk/tree/main/rust/tonk-cli).
