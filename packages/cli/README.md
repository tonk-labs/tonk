# Tonk CLI

Command line utilities for your Tonk network

## Installation

```bash
npm install -g @tonk/cli
```

## Commands

### Create a new Tonk app

```bash
tonk create my-app
```

### Development

Run your Tonk app in development mode:

```bash
tonk dev
```

### Serve

Serve your built Tonk app locally:

```bash
tonk serve
```

### Bundle Management

Tonk supports managing application bundles through the following commands:

#### Push a bundle to the server

Package your dist folder and upload it to the Tonk server:

```bash
# Push with defaults (from ./dist folder, using package name)
tonk push

# Push with custom options
tonk push -n my-bundle -d ./build -s
```

Options:

- `-u, --url <url>` - URL of the Tonk server (default: http://localhost:7777)
- `-n, --name <name>` - Name for the bundle (defaults to directory name or package name)
- `-d, --dir <dir>` - Directory to bundle (defaults to ./dist)
- `-s, --start` - Start the bundle after upload

#### List available bundles

List all bundles available on the server:

```bash
tonk ls
```

Options:

- `-u, --url <url>` - URL of the Tonk server (default: http://localhost:7777)

#### List running bundle servers

Show all currently running bundle servers:

```bash
tonk ps
```

Options:

- `-u, --url <url>` - URL of the Tonk server (default: http://localhost:7777)

#### Start a bundle server

Start a bundle server for a specific bundle:

```bash
tonk start my-bundle

# Specify a port
tonk start my-bundle -p 8080
```

Options:

- `-u, --url <url>` - URL of the Tonk server (default: http://localhost:7777)
- `-p, --port <port>` - Port for the bundle server (optional)

#### Kill a running bundle server

Stop a running bundle server by its ID:

```bash
tonk kill <server-id>
```

Options:

- `-u, --url <url>` - URL of the Tonk server (default: http://localhost:7777)

### Configure AWS Deployment

Configure your AWS EC2 instance for deployment:

```bash
# Interactive configuration (with existing instance)
tonk config

# Provision a new EC2 instance
tonk config --provision [--key-name my-key]

# Configure with existing EC2 instance
tonk config --instance ec2-xx-xx-xx-xx.compute-1.amazonaws.com --key ~/path/to/key.pem
```

Options:

- `-i, --instance <address>` - EC2 instance address
- `-k, --key <path>` - Path to SSH key file
- `-u, --user <n>` - SSH username (default: ec2-user)
- `-p, --provision` - Provision a new EC2 instance
- `-n, --key-name <n>` - AWS key pair name (for provisioning)

### Deploy to AWS

Deploy your Tonk app to an EC2 instance:

```bash
tonk deploy
```

Options:

- `-i, --instance <address>` - EC2 instance address
- `-k, --key <path>` - Path to SSH key file
- `-u, --user <n>` - SSH username (default: ec2-user)
- `-t, --token <token>` - Pinggy.io access token for public URL
- `-b, --backblaze` - Enable Backblaze B2 storage for document backup
- `-f, --filesystem` - Enable filesystem storage for document backup

#### Persistence Features

Tonk supports multiple options for persisting your documents:

1. **Backblaze B2 Storage**: Cloud-based backup for your documents
   - Secure, scalable cloud storage
   - Automatic synchronization at configurable intervals
   - Requires a Backblaze B2 account with bucket and application keys

2. **Filesystem Storage**: Local file-based storage on your server
   - Store documents directly on the server's filesystem
   - Configurable storage path and sync intervals
   - Automatic directory creation

3. **Dual Storage**: Use both Backblaze and filesystem storage
   - Configure a primary storage option
   - Automatic fallback to secondary storage

During deployment, you'll be guided through setting up your preferred storage options interactively
if not specified via command-line options.

## Development

1. Clone the repository
2. Install dependencies: `npm install`
3. Run the dev mode: `npm run dev`

## Troubleshooting

### Permission Denied Error During Installation

If you encounter a permission denied error when installing the Tonk CLI globally:

```bash
npm install -g @tonk/cli
# Error: EACCES: permission denied, mkdir '/usr/local/lib/node_modules'
```

This is a common npm issue on Unix systems. Here are several solutions:

#### Option 1: Fix npm permissions

```bash
sudo chown -R $(whoami) $(npm config get prefix)/{lib/node_modules,bin,share}
```

#### Option 2: Configure npm to use a different directory

```bash
mkdir ~/.npm-global
npm config set prefix '~/.npm-global'
echo 'export PATH="$HOME/.npm-global/bin:$PATH"' >> ~/.bashrc
source ~/.bashrc
npm install -g @tonk/cli
```

## License

Simplicity and freedom.

MIT © Tonk
