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
- `-u, --user <name>` - SSH username (default: ec2-user)
- `-p, --provision` - Provision a new EC2 instance
- `-n, --key-name <name>` - AWS key pair name (for provisioning)

### Deploy to AWS

Deploy your Tonk app to an EC2 instance:

```bash
tonk deploy
```

Options:
- `-i, --instance <address>` - EC2 instance address
- `-k, --key <path>` - Path to SSH key file
- `-u, --user <name>` - SSH username (default: ec2-user)
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

During deployment, you'll be guided through setting up your preferred storage options interactively if not specified via command-line options.

## Development

1. Clone the repository
2. Install dependencies: `npm install`
4. Run the dev mode: `npm run dev`

## License

Simplicity and freedom.

MIT © Tonk
