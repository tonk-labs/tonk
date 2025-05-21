# Docker Deployment Guide for Tonk Apps

This guide explains how to deploy your Tonk application using Docker, making it easy to self-host using the Tonk server.

## Prerequisites

- [Docker](https://docs.docker.com/get-docker/) installed on your system
- [Docker Compose](https://docs.docker.com/compose/install/) installed on your system

## Quick Start

1. Build your application first:
   ```bash
   pnpm build
   ```

2. Start the Tonk server container with your app:
   ```bash
   docker-compose up -d
   ```

3. Access your application at http://localhost:8000

4. View logs:
   ```bash
   docker-compose logs -f
   ```

5. Stop the container:
   ```bash
   docker-compose down
   ```

## How It Works

The Docker setup:
1. Starts the Tonk server container
2. Mounts your local `dist` directory into the container
3. Creates a bundle from your application
4. Uploads the bundle to the Tonk server
5. Starts your application on port 8000

This approach is similar to using `tonk push` and `tonk start` with the Tonk CLI locally.

## Configuration

### Environment Variables

Edit the `docker-compose.yml` file to add any required API keys or environment variables for your services:

```yaml
tonk-server:
  environment:
    - PORT=7777
    # Add more environment variables as needed
```

### Ports

By default, the Tonk server runs on port 7777 and your application runs on port 8000. You can change these ports in the `docker-compose.yml` file:

```yaml
tonk-server:
  ports:
    - "7777:7777"  # Tonk server port (change the first number to change the host port)
    - "8000:8000"  # App bundle port (change the first number to change the host port)
```

You'll also need to update the port in the start command if you change the app port:

```yaml
command: >
  sh -c "
    # ...
    curl -X POST -H 'Content-Type: application/json' -d '{\"bundleName\":\"tonk-app\",\"port\":8000}' http://localhost:7777/start
    # ...
  "
```

### Volumes

The Docker setup creates two persistent volumes:
- `tonk-data`: Stores the Tonk server's data
- `tonk-bundles`: Stores application bundles

These volumes ensure your data persists even if the container is removed.

## Advanced Configuration

### Custom Tonk Server Image

If you have a custom Tonk server image, you can specify it in the `docker-compose.yml` file:

```yaml
tonk-server:
  image: your-custom-tonk-server-image:tag
```

### Using a Reverse Proxy

For production deployments, it's recommended to use a reverse proxy such as Nginx to handle SSL termination and routing:

```yaml
services:
  nginx:
    image: nginx:alpine
    ports:
      - "80:80"
      - "443:443"
    volumes:
      - ./nginx.conf:/etc/nginx/nginx.conf
      - ./ssl:/etc/nginx/ssl
    depends_on:
      - tonk-server
    networks:
      - tonk-network

  tonk-server:
    image: tonklabs/tonk-server:latest
    volumes:
      - tonk-data:/app/stores
      - tonk-bundles:/app/bundles
      - ./dist:/tmp/app-bundle
    expose:
      - "7777"
      - "8000"
    environment:
      - PORT=7777
    networks:
      - tonk-network
    command: >
      sh -c "
        # Install curl if not already installed
        echo 'Installing curl...'
        apk add curl

        # Start Tonk server in the background
        echo 'Starting Tonk server...'
        tsx src/docker-start.ts &
        
        # Wait for the server to be ready
        echo 'Waiting for Tonk server to start...'
        until $(curl --output /dev/null --silent --fail http://localhost:7777/ping); do
          printf '.'
          sleep 1
        done
        echo 'Tonk server is up!'
        
        # Create a tarball from the mounted dist directory
        echo 'Creating app bundle...'
        cd /tmp
        tar -czf app-bundle.tar.gz -C /tmp/app-bundle .
        
        # Upload the bundle to the Tonk server
        echo 'Uploading app bundle to Tonk server...'
        curl -X POST -F 'bundle=@/tmp/app-bundle.tar.gz' -F 'name=tonk-app' http://localhost:7777/upload-bundle
        
        # Start the bundle
        echo 'Starting app bundle...'
        curl -X POST -H 'Content-Type: application/json' -d '{\"bundleName\":\"tonk-app\",\"port\":8000}' http://localhost:7777/start
        
        # Keep the container running
        tail -f /dev/null
      "

networks:
  tonk-network:
    driver: bridge

volumes:
  tonk-data:
  tonk-bundles:
```

## Troubleshooting

### Container Fails to Start

If the container fails to start, check the logs:

```bash
docker-compose logs tonk-server
```

### Data Persistence Issues

If you're experiencing data persistence issues, make sure the volumes are properly mounted:

```bash
docker volume ls
```

You should see `tonk-data` and `tonk-bundles` in the list.

### Bundle Deployment Issues

If your application bundle isn't being deployed correctly:

1. Make sure your application is built before starting the container:
   ```bash
   pnpm build
   ```

2. Check that the `dist` directory exists and contains your built application.

3. Check the container logs for any errors during the bundle creation or upload process:
   ```bash
   docker-compose logs tonk-server
   ```

## Resources

- [Docker Documentation](https://docs.docker.com/)
- [Docker Compose Documentation](https://docs.docker.com/compose/)
- [Tonk Documentation](https://github.com/tonk-labs/tonk)
