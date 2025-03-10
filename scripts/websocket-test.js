
/**
 * WebSocket Test Client for localhost:3000
 * This script tests connection, message sending, and receiving from the WebSocket server.
 */

const WebSocket = require('ws');
const readline = require('readline');

// Colors for console output
const colors = {
  green: '\x1b[32m',
  red: '\x1b[31m',
  yellow: '\x1b[33m',
  blue: '\x1b[34m',
  reset: '\x1b[0m'
};

// Create interface for reading user input
const rl = readline.createInterface({
  input: process.stdin,
  output: process.stdout
});

console.log(`${colors.blue}WebSocket Test Client${colors.reset}`);
console.log(`${colors.blue}====================${colors.reset}`);
console.log(`Attempting to connect to ${colors.yellow}ws://localhost:3000${colors.reset}...`);

// Create WebSocket connection
const ws = new WebSocket('ws://localhost:3000');

// Connection opened
ws.on('open', () => {
  console.log(`${colors.green}Connected to WebSocket server!${colors.reset}`);
  console.log(`${colors.blue}Type a message and press Enter to send. Type 'exit' to quit.${colors.reset}`);
  
  // Start reading user input
  promptUser();
});

// Listen for messages
ws.on('message', (data) => {
  try {
    const message = JSON.parse(data.toString());
    console.log(`${colors.green}Received:${colors.reset}`, message);
  } catch (error) {
    // Handle non-JSON messages
    console.log(`${colors.green}Received:${colors.reset} ${data}`);
  }
  promptUser();
});

// Handle errors
ws.on('error', (error) => {
  console.log(`${colors.red}Error:${colors.reset} ${error.message}`);
  if (error.message.includes('ECONNREFUSED')) {
    console.log(`${colors.yellow}Make sure the WebSocket server is running on localhost:3000${colors.reset}`);
  }
  process.exit(1);
});

// Connection closed
ws.on('close', () => {
  console.log(`${colors.yellow}Disconnected from server${colors.reset}`);
  rl.close();
  process.exit(0);
});

// Prompt user for input
function promptUser() {
  rl.question('> ', (input) => {
    if (input.toLowerCase() === 'exit') {
      console.log(`${colors.yellow}Closing connection...${colors.reset}`);
      ws.close();
      return;
    }
    
    try {
      // Try to send as JSON
      const messageObj = { message: input };
      ws.send(JSON.stringify(messageObj));
      console.log(`${colors.blue}Sent:${colors.reset}`, messageObj);
    } catch (error) {
      console.log(`${colors.red}Error sending message:${colors.reset} ${error.message}`);
    }
  });
}

// Handle script termination
process.on('SIGINT', () => {
  console.log(`\n${colors.yellow}Closing connection...${colors.reset}`);
  ws.close();
  rl.close();
  process.exit(0);
}); 
