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

# Production stage
FROM node:20-alpine

WORKDIR /app

# Copy package files and install production dependencies
COPY package.json ./
# Copy lock file if it exists (supports npm, yarn, pnpm)
COPY package-lock.json* yarn.lock* pnpm-lock.yaml* ./
# Install tonk CLI globally in the production stage
RUN npm install -g @tonk/cli@^0.1.9

# Use appropriate install command based on available lock files
RUN if [ -f "package-lock.json" ]; then npm ci --production; \
  elif [ -f "yarn.lock" ]; then yarn install --frozen-lockfile --production; \
  elif [ -f "pnpm-lock.yaml" ]; then npm install -g pnpm && pnpm install --prod; \
  else npm install --production; \
  fi

# Copy built app from build stage
COPY --from=build /app/dist ./dist

# Expose the port the server runs on
EXPOSE 8080

# Start the server using the built-in serve command
CMD ["npm", "run", "serve"]
