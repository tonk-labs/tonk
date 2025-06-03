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

Packages and uploads your application bundle to the Tonk server.

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

Lists available application bundles on the Tonk server.

```bash
Usage: tonk ls [options]

List available bundles on the Tonk server

Options:
  -u, --url <url>  URL of the Tonk server (default: "http://localhost:7777")
  -h, --help       display help for command
```

### `tonk ps`

Shows currently running bundle servers.

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

### `tonk proxy <bundle-name>`

Creates a reverse proxy to access a Tonk bundle using SSH tunnelling with Pinggy service.

```bash
Usage: tonk proxy [options] <bundleName>

Create a reverse proxy to access a Tonk bundle

Arguments:
  bundleName         Name of the bundle to proxy

Options:
  -u, --url <url>    URL of the Tonk server (default: "http://localhost:7777")
  -h, --help         display help for command
```

This command checks if the specified bundle is running, then creates an SSH tunnel using Pinggy to make the bundle accessible via a public URL with QR code for easy mobile access.

### `tonk worker`

Manages worker processes and configurations. The worker command provides comprehensive lifecycle management for Tonk workers.

```bash
Usage: tonk worker [command] [options]

Manage Tonk workers

Commands:
  inspect <nameOrId>     Inspect a specific worker
  ls                     List all registered workers
  rm <nameOrId>          Remove a registered worker
  ping <nameOrId>        Ping a worker to check its status
  start <nameOrId>       Start a worker
  stop <nameOrId>        Stop a worker
  logs <nameOrId>        View logs for a worker
  register [dir]         Register a worker with Tonk
  install <package>      Install and start a worker from npm
  init                   Initialize a new worker configuration file

Options:
  -h, --help             display help for command
```

#### Worker Subcommands

**`tonk worker inspect <nameOrId>`**
Inspect a specific worker and optionally perform actions on it.

```bash
Options:
  -s, --start            Start the worker
  -S, --stop             Stop the worker
  -c, --config <path>    Path to worker configuration file
  -p, --ping             Ping the worker to check its status
  -h, --help             display help for command
```

**`tonk worker logs <nameOrId>`**
View logs for a worker using PM2 integration.

```bash
Options:
  -f, --follow           Follow log output
  -l, --lines <n>        Number of lines to show (default: "100")
  -e, --error            Show only error logs
  -o, --out              Show only standard output logs
  -h, --help             display help for command
```

**`tonk worker register [dir]`**
Register a worker with Tonk from a directory containing worker configuration.

```bash
Arguments:
  dir                    Path to worker directory (defaults to current directory)

Options:
  -n, --name <n>         Name of the worker
  -e, --endpoint <endpoint>  Endpoint URL of the worker
  -p, --port <port>      Port number for the worker
  -d, --description <description>  Description of the worker
  -h, --help             display help for command
```

**`tonk worker install <package>`**
Install and start a worker directly from an npm package.

```bash
Arguments:
  package                NPM package name

Options:
  -p, --port <port>      Specify a port for the worker (default: auto-detect)
  -n, --name <n>         Custom name for the worker (default: npm package name)
  -h, --help             display help for command
```

**`tonk worker init`**
Initialize a new worker configuration file in the current or specified directory.

```bash
Options:
  -d, --dir <directory>  Directory to create the configuration file in (default: ".")
  -n, --name <n>         Name of the worker
  -p, --port <port>      Port number for the worker (default: "5555")
  -h, --help             display help for command
```

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
