# Build stage
FROM node:20-alpine as build

WORKDIR /app

# Copy package files and install dependencies
COPY package.json ./
COPY pnpm-workspace.yaml ./
# Copy lock file if it exists (supports npm, yarn, pnpm)
COPY package-lock.json* yarn.lock* pnpm-lock.yaml* ./
# Copy server package.json
COPY server/package.json ./server/
# Use appropriate install command based on available lock files
RUN if [ -f "package-lock.json" ]; then npm ci; \
  elif [ -f "yarn.lock" ]; then yarn install --frozen-lockfile; \
  elif [ -f "pnpm-lock.yaml" ]; then npm install -g pnpm && pnpm install --frozen-lockfile; \
  else npm install; \
  fi

# Copy source files and build the app
COPY . .
ARG APP_ROUTE
ENV APP_ROUTE=${APP_ROUTE:-/my-app}
RUN VITE_BASE_PATH=${APP_ROUTE}/ npm run build

# Production stage - Use tonklabs/tonk-server as base
FROM tonklabs/tonk-server:latest

WORKDIR /app

# Copy built app from build stage to tmp directory
COPY --from=build /app/dist /tmp/app-bundle

# Get app name from environment or use default
ARG APP_NAME
ENV APP_NAME=${APP_NAME:-my-app}
ARG APP_ROUTE
ENV APP_ROUTE=${APP_ROUTE}

# Expose only the unified server port
EXPOSE 7777

# Health check on the unified server
HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
  CMD wget --no-verbose --tries=1 --spider http://localhost:7777/ping || exit 1

# Startup script that deploys app to unified server
CMD sh -c ' \
  echo "Starting Tonk unified server..." && \
  tsx src/docker-start.ts & \
  SERVER_PID=$! && \
  echo "Waiting for Tonk server to start..." && \
  until curl --output /dev/null --silent --fail http://localhost:7777/ping; do \
    printf "."; \
    sleep 1; \
  done && \
  echo "Tonk server is up!" && \
  echo "Creating app bundle..." && \
  cd /tmp && tar -czf app-bundle.tar.gz -C /tmp/app-bundle . && \
  echo "Uploading app bundle to Tonk server..." && \
  curl -X POST -F "bundle=@/tmp/app-bundle.tar.gz" -F "name=${APP_NAME}" -F "serverPin=${SERVER_PIN}" http://localhost:7777/upload-bundle && \
  echo "Starting app bundle on route..." && \
  ROUTE_PATH=${APP_ROUTE:-/${APP_NAME}} && \
  curl -X POST -H "Content-Type: application/json" -d "{\"bundleName\":\"${APP_NAME}\",\"route\":\"${ROUTE_PATH}\",\"serverPin\":\"${SERVER_PIN}\"}" http://localhost:7777/start && \
  echo "App started successfully on route ${ROUTE_PATH}!" && \
  echo "App available at: http://localhost:7777${ROUTE_PATH}" && \
  wait $SERVER_PID \
'
