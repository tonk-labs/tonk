# Docker Deployment Guide for Tonk Apps

This guide explains how to deploy your Tonk application using Docker, making it easy to self-host with the Tonk server.

## Prerequisites

- [Docker](https://docs.docker.com/get-docker/) installed on your system

## Quick Start

1. Build and run the containerized application:
   ```bash
   docker build -t my-tonk-app .
   docker run -p 7777:7777 -p 8000:8000 my-tonk-app
   ```

2. Access your application at http://localhost:8000

3. View logs:
   ```bash
   docker logs <container-id>
   ```

4. Stop the container:
   ```bash
   docker stop <container-id>
   ```

## How It Works

The Docker setup uses a multi-stage build:
1. **Build stage**: Installs dependencies and builds your application
2. **Production stage**: Uses the Tonk server base image
3. Copies the built application into the container
4. Runs a startup script that:
   - Starts the Tonk server
   - Creates a bundle from your application
   - Uploads the bundle to the Tonk server
   - Starts your application on port 8000

This provides a completely self-contained deployment with no external dependencies.

## Configuration

### Environment Variables

Pass environment variables to the container at runtime:

```bash
docker run -p 7777:7777 -p 8000:8000 -e API_KEY=your-key my-tonk-app
```

### Ports

By default, the Tonk server runs on port 7777 and your application runs on port 8000. You can map these to different host ports:

```bash
docker run -p 8080:7777 -p 9000:8000 my-tonk-app
```

### Volumes

For data persistence, mount volumes:

```bash
docker run -p 7777:7777 -p 8000:8000 \
  -v tonk-data:/app/stores \
  -v tonk-bundles:/app/bundles \
  my-tonk-app
```

## Advanced Configuration

### Custom Base Image

To use a custom Tonk server base image, modify the Dockerfile:

```dockerfile
# Change the base image in the production stage
FROM your-custom-tonk-server:tag
```

### Health Checks

The Dockerfile includes a health check that verifies your app is running on port 8000. You can customize this:

```dockerfile
HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
  CMD wget --no-verbose --tries=1 --spider http://localhost:8000/health || exit 1
```

### Production Deployment

For production, consider using Docker Compose or Kubernetes for orchestration:

```yaml
# docker-compose.yml
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
      - app

  app:
    build: .
    expose:
      - "8000"
    environment:
      - NODE_ENV=production
    volumes:
      - app-data:/app/stores
      - app-bundles:/app/bundles

volumes:
  app-data:
  app-bundles:
```

## Troubleshooting

### Container Fails to Start

Check the container logs:

```bash
docker logs <container-id>
```

### Build Issues

If the build fails, ensure you have the required dependencies and the correct Node.js version:

```bash
docker build --no-cache -t my-tonk-app .
```

### Application Not Accessible

1. Verify ports are correctly mapped:
   ```bash
   docker ps
   ```

2. Check if the application started successfully in the logs:
   ```bash
   docker logs <container-id>
   ```

3. Test the health check endpoint:
   ```bash
   curl http://localhost:8000/
   ```

## Resources

- [Docker Documentation](https://docs.docker.com/)
- [Docker Compose Documentation](https://docs.docker.com/compose/)
- [Tonk Documentation](https://github.com/tonk-labs/tonk)
