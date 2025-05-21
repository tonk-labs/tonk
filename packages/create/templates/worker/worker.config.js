/**
 * Tonk Worker Configuration
 */
module.exports = {
  // Runtime configuration
  runtime: {
    port: {{port}},
    healthCheck: {
      endpoint: '/health',
      method: 'GET',
      interval: 30000,
      timeout: 5000,
    },
  },

  // Process management
  process: {
    file: "cli.js",
    args: "start",
    cwd: "./dist",
    instances: 1,
    autorestart: true,
    watch: false,
    max_memory_restart: '500M',
    env: {
      NODE_ENV: 'production',
    },
  },

  // CLI configuration
  cli: {
    script: './dist/cli.js',
    command: 'start',
    args: ['--port', '{{port}}'],
  },

  // Data schema
  // This section defines the schemas for data stored in keepsync
  // Can be used to validate data before storing it
  schemas: {
    // Define schemas for different document types
    documents: {
      // Main document schema
      default: {},

      // Additional document types can be defined here
      // For example:
      //
      // specialDocument: {
      //   type: "object",
      //   properties: { ... }
      // }
    },
  },

  // Additional configuration
  config: {},
};
