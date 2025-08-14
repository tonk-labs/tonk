// Simple test worker for testing CLI functionality
export default {
  name: 'simple-worker',
  version: '1.0.0',

  async handler(request: any) {
    return {
      message: 'Hello from simple worker',
      timestamp: new Date().toISOString(),
      data: request.data || null,
    };
  },

  async onStart() {
    console.log('Simple worker started');
  },

  async onStop() {
    console.log('Simple worker stopped');
  },
};
