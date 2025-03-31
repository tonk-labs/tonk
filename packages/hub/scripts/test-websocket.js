const WebSocket = require('ws');

const PORT = 3060;
const URL = `ws://localhost:${PORT}`;

console.log(`Attempting to connect to WebSocket server at ${URL}...`);

const ws = new WebSocket(URL);

ws.on('open', () => {
  console.log('Successfully connected to WebSocket server!');
  
  // Send a test init message
  const initMessage = {
    type: 'init',
    cols: 80,
    rows: 24,
    shell: process.env.SHELL || 'bash'
  };
  
  console.log('Sending init message:', initMessage);
  ws.send(JSON.stringify(initMessage));
});

ws.on('message', (data) => {
  const message = JSON.parse(data);
  console.log('Received message from server:', message);
  
  // If we receive an error, log it prominently
  if (message.type === 'error') {
    console.error('\nSERVER ERROR:', message.data, '\n');
  }
});

ws.on('error', (error) => {
  console.error('WebSocket connection error:', error.message);
  process.exit(1);
});

ws.on('close', () => {
  console.log('Connection closed');
});

// Keep the process alive for a bit
setTimeout(() => {
  ws.close();
  process.exit(0);
}, 5000); 