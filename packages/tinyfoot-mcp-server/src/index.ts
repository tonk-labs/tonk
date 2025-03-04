import {ModuleRegistry} from './moduleRegistry.js';
import path from 'path';

let registry: ModuleRegistry | null = null;
let isShuttingDown = false;

class ServerError extends Error {
  constructor(message: string, public code: number = 1) {
    super(message);
    this.name = 'ServerError';
  }
}

// Export the runServer function so it can be used programmatically
export async function runServer() {
  if (registry) {
    console.log('Server is already running');
    return;
  }

  try {
    const modulesDir = path.join(process.cwd(), 'src', 'modules');
    registry = new ModuleRegistry(modulesDir);
    await registry.initialize();

    const shutdown = async (signal?: string) => {
      if (isShuttingDown) return;
      isShuttingDown = true;

      console.log(`\nReceived ${signal || 'shutdown'} signal`);
      console.log('Gracefully shutting down server...');

      if (registry) {
        await registry.dispose();
        registry = null;
      }

      // Give time for cleanup before throwing
      await new Promise(resolve => setTimeout(resolve, 100));
      console.log('Server shutdown complete');
      throw new ServerError('Server shutdown', 0);
    };

    // Handle termination signals
    process.on('SIGINT', () => {
      void shutdown('SIGINT').catch(error => {
        console.error('Error during SIGINT shutdown:', error);
        process.exit(0);
      });
    });
    process.on('SIGTERM', () => {
      void shutdown('SIGTERM').catch(error => {
        console.error('Error during SIGTERM shutdown:', error);
        process.exit(0);
      });
    });
    process.on('uncaughtException', error => {
      console.error('Uncaught Exception:', error);
      void shutdown('UNCAUGHT_EXCEPTION').catch(err => {
        console.error('Error during uncaught exception shutdown:', err);
        process.exit(0);
      });
    });

    console.log('Server started successfully');
  } catch (error) {
    console.error('Failed to start server:', error);
    throw new ServerError('Failed to start server');
  }
}

// CLI interface
async function cli() {
  console.log('CLI started. Args:', process.argv);
  const command = process.argv[2];

  switch (command) {
    case 'start':
      console.log('Starting MCP server...');
      await runServer();
      console.log('Server running. Press Ctrl+C to stop.');
      // Keep the process alive
      process.stdin.resume();
      break;
    case '--help':
    case '-h':
      console.log(`
TinyFoot MCP Server CLI

Usage:
  tinyfoot-mcp-server <command>

Commands:
  start     Start the MCP server
  --help    Show this help message

`);
      throw new ServerError('Help displayed', 0);
    default:
      console.log('Unknown command. Use --help to see available commands.');
      throw new ServerError('Unknown command', 1);
  }
}

// For ES Modules, always run the CLI
console.log('Module loaded. Process argv:', process.argv);
console.log('URL:', import.meta.url);
console.log('Bin path:', process.argv[1]);

// Always run the CLI when executed directly
cli().catch(error => {
  console.error('CLI error:', error);
  process.exit(1);
});
