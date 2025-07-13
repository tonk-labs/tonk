FROM node:18-alpine

# Install build dependencies, nginx, and pnpm
RUN apk add --no-cache python3 make g++ git nginx && \
    npm install -g pnpm tsx

# Create app directory
WORKDIR /app

# Install app dependencies
# Copy package.json and pnpm-lock.yaml first for better layer caching
COPY package.json pnpm-lock.yaml* ./
RUN pnpm install --frozen-lockfile

# Copy app source
COPY . .

# Build the app if needed (uncomment if you have a build step)
# RUN pnpm run build

# Create directories for persistence
RUN mkdir -p /data/tonk/bundles /data/tonk/stores

# Set environment variables with defaults
ENV PORT=7777 \
    BUNDLES_PATH=/data/tonk/bundles \
    STORES_PATH=/data/tonk/stores \
    CONFIG_PATH=/data/tonk/config \
    VERBOSE=true \
    SYNC_INTERVAL=0

# Expose the port
EXPOSE $PORT

# Set healthcheck
HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
  CMD wget --no-verbose --tries=1 --spider http://localhost:$PORT/ping || exit 1

# Run the server
CMD ["tsx", "src/docker-start.ts"]
