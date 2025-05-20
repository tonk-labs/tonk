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
    script: './dist/cli.js',
    cwd: './',
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

  // Additional configuration
  config: {},
};
