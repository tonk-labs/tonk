# Build stage
FROM node:20-alpine as build

WORKDIR /app

# Copy package files and install dependencies
COPY package.json ./
# Copy lock file if it exists (supports npm, yarn, pnpm)
COPY package-lock.json* yarn.lock* pnpm-lock.yaml* ./
# Use appropriate install command based on available lock files
RUN if [ -f "package-lock.json" ]; then npm ci; \
  elif [ -f "yarn.lock" ]; then yarn install --frozen-lockfile; \
  elif [ -f "pnpm-lock.yaml" ]; then npm install -g pnpm && pnpm install --frozen-lockfile; \
  else npm install; \
  fi

# Copy source files and build the app
COPY . .
RUN npm run build

# Production stage - Use tonklabs/tonk-server as base
FROM tonklabs/tonk-server:latest

WORKDIR /app

# Copy built app from build stage to tmp directory
COPY --from=build /app/dist /tmp/app-bundle

# Copy startup script
COPY scripts/start-with-bundle.sh /app/start-with-bundle.sh
RUN chmod +x /app/start-with-bundle.sh

# Expose both Tonk server port and app port
EXPOSE 7777 8000

# Health check on the app port
HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
  CMD wget --no-verbose --tries=1 --spider http://localhost:8000/ || exit 1

# Use the startup script
CMD ["sh", "/app/start-with-bundle.sh"]
