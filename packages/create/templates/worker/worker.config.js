module.exports = {
  // Worker runtime configuration
  runtime: {
    // Default port (can be overridden)
    port: 5555,

    // Health check configuration
    healthCheck: {
      enabled: true,
      path: "/health",
      interval: 30000,
    },
  },

  // Worker-specific configuration
  worker: {},
};
