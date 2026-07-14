# @tonk/cli

The Tonk stack command line utility — `tonk`.

```sh
npx @tonk/cli --help
# or install globally
npm install -g @tonk/cli
tonk --help
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
