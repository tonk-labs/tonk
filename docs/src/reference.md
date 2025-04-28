# Reference

This reference guide provides detailed information about Tonk commands, features, and troubleshooting tips.

## Command reference

The Tonk CLI includes the following commands:

### `tonk hello`

Initializes the Tonk daemon, which provides synchronization services for your apps.

```bash
Usage: tonk hello [options]

Say hello to start and launch the tonk daemon

Options:
  -h, --help  display help for command
```

### `tonk create`

Creates a new Tonk application with an interactive setup process.

```bash
Usage: tonk create [options]

Create a new tonk application or component

Options:
  -i, --init  initialize in the folder
  -h, --help  display help for command
```

### `tonk push`

Packages and uploads your application bundle.

```bash
Usage: tonk push [options]

Package and upload a bundle to the Tonk server

Options:
  -u, --url <url>    URL of the Tonk server (default: "http://localhost:7777")
  -n, --name <name>  Name for the bundle (defaults to directory name)
  -d, --dir <dir>    Directory to bundle (defaults to ./dist)
  -s, --start        Start the bundle after upload
  -h, --help         display help for command
```

### `tonk ls`

Lists available application bundles.

```bash
Usage: tonk ls [options]

List available bundles on the Tonk server

Options:
  -u, --url <url>  URL of the Tonk server (default: "http://localhost:7777")
  -h, --help       display help for command
```

### `tonk ps`

Shows running bundle servers.

```bash
Usage: tonk ps [options]

List running bundle servers

Options:
  -u, --url <url>  URL of the Tonk server (default: "http://localhost:7777")
  -h, --help       display help for command
```

### `tonk start <bundle-name>`

Starts a bundle server for a specific bundle.

```bash
Usage: tonk start [options] <bundleName>

Start a bundle server

Arguments:
  bundleName         Name of the bundle to start

Options:
  -u, --url <url>    URL of the Tonk server (default: "http://localhost:7777")
  -p, --port <port>  Port for the bundle server (optional)
  -h, --help         display help for command
```

### `tonk kill <server-id>`

Stops a running bundle server.

```bash
Usage: tonk kill [options] <serverId>

Stop a running bundle server

Arguments:
  serverId         ID of the server to stop

Options:
  -u, --url <url>  URL of the Tonk server (default: "http://localhost:7777")
  -h, --help       display help for command
```

## Using llms.txt files

Tonk projects include `llms.txt` files containing instructions for AI assistants. These files help AI models understand the project structure and conventions when helping you develop your app.

## FAQ

### Pre-requisites to install

1. You'll need Node.js and npm installed to run the Tonk installation command.

### How do I get it working on Windows?

Tonk should work on Windows without any extra configuration.

1. Install Tonk via npm:

```bash
npm install -g @tonk/cli
```

2. Start Tonk:

```bash
tonk hello
```
