# Deploying Tonk Apps

This guide covers various deployment strategies for Tonk applications, from local containerization to cloud deployment on platforms like AWS EC2.

## Overview

Tonk applications can be deployed in several ways:

1. **Local Development**: Using `tonk -d` daemon for development
2. **Docker Containerization**: Packaging apps and the Tonk server in containers
3. **One-Touch Hosting**: Using `tonk deploy` for managed hosting (experimental)

## Docker Deployment

Tonk provides built-in Docker support for both the Tonk server and individual applications.

### Tonk Server

The Tonk server is available as a pre-built Docker image at `tonklabs/tonk-server:latest`. Pull and run the image with:

```bash
docker run -d \
  --name tonk-server \
  -p 7777:7777 \
  -v tonk-data:/data/tonk \
  tonklabs/tonk-server:latest
```

### Tonk Apps

When you create a Tonk app using `tonk create`, a `docker-compose.yml` file is automatically included in your project. This file is pre-configured to work with your app.

1. **Build your Tonk app:**
   ```bash
   cd my-tonk-app
   pnpm run build
   ```

2. **Start the containers using the included configuration:**
   ```bash
   docker-compose up -d
   ```

3. **Access your app:**
   - Tonk server: http://localhost:7777
   - Your app: http://localhost:8000

### Customizing Your Docker Setup

You can customize the included `docker-compose.yml` file for your specific needs:

```yaml
services:
  tonk-server:
    image: tonklabs/tonk-server:latest
    container_name: tonk-server
    volumes:
      - tonk-data:/data/tonk/stores
      - tonk-bundles:/data/tonk/bundles
      - ./dist:/tmp/app-bundle
    ports:
      - "7777:7777"
      - "8000:8000"
    environment:
      - PORT=7777
      - NODE_ENV=production
      - VERBOSE=false           # Disable verbose logging for production
      - SYNC_INTERVAL=30000     # Set sync interval to 30 seconds
    restart: unless-stopped
    # The command section handles app deployment automatically
```

### Environment Configuration

The Tonk server Docker image supports several environment variables:

| Variable | Default | Description |
|----------|---------|-------------|
| `PORT` | 7777 | Port for the Tonk server |
| `BUNDLES_PATH` | /data/tonk/bundles | Directory for storing app bundles |
| `STORES_PATH` | /data/tonk/stores | Directory for storing data |
| `CONFIG_PATH` | /data/tonk/config | Directory for configuration files |
| `VERBOSE` | true | Enable verbose logging |
| `SYNC_INTERVAL` | 0 | Sync interval in milliseconds |
| `NODE_ENV` | production | Node.js environment |

## Troubleshooting

### Common Issues

1. **Port conflicts**: Ensure ports 7777 and 8000 are available
2. **Permission issues**: Check file permissions for data directories
3. **Network connectivity**: Verify security group settings
4. **Resource limits**: Monitor CPU and memory usage

### Debugging Commands

```bash
# Check container logs
docker logs tonk-server

# Check container status
docker ps

# Check Tonk server health
curl http://localhost:7777/ping

# Check running bundles
tonk ps
```

## One-Touch Hosting (Experimental)

⚠️ **EXPERIMENTAL FEATURE** ⚠️

The `tonk deploy` command provides experimental one-touch hosting for Tonk applications. **This feature is highly experimental and will change drastically. Expect data loss.** You must obtain an access code to use this feature (see below).

### Usage

In your Tonk app directory run:
```bash
tonk deploy
```

Options:
- `-n, --name <name>`: Name for the deployed app (defaults to package.json name)
- `-r, --region <region>`: Region to deploy to (default: ord)
- `-m, --memory <memory>`: Memory allocation (e.g., 256mb, 1gb) (default: 1gb)
- `-c, --cpus <cpus>`: Number of CPUs (default: 1)
- `--skip-build`: Skip the build step
- `--remote`: Use remote Docker build

### Important Warnings

- **Data Loss**: Your deployed applications and data may be lost at any time
- **Breaking Changes**: The deployment architecture will change soon
- **No SLA**: This is an experimental service with no uptime guarantees
- **Access Required**: You must have a valid deploy code from Tonk

### How It Works

1. Automatically builds your Tonk application with the correct configuration
2. Creates a compressed bundle of your project
3. Uploads the bundle to Tonk's hosting service
4. Starts the application with route-based serving
5. Returns a public URL for your deployed application

Use `tonk push` for local deployment or `tonk deploy` for cloud hosting.

### Getting Access

Contact Jack at Tonk ([Telegram](https://t.me/jackddouglas)) to request a deploy code.

## Next Steps

- Explore [Tonk Workers](./workers.md) for background processing
