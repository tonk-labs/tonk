import WebSocket from 'ws';

// Create a WebSocket connection to localhost:8000/sync
const ws = new WebSocket('ws://localhost:8000/sync');

// Connection opened
ws.on('open', () => {
  console.log('Connected to WebSocket server at ws://localhost:8000/sync');
  
  // Create a sample binary message using Uint8Array
  const binaryData = new Uint8Array([0x68, 0x65, 0x6c, 0x6c, 0x6f]); // ASCII for "hello"
  ws.send(binaryData);
  
  // You can also send ArrayBuffer directly
  const buffer = new ArrayBuffer(4);
  const view = new Uint32Array(buffer);
  view[0] = 0x12345678;
  ws.send(buffer);
  
  console.log('Sent binary messages to server');
});

// Listen for messages
ws.on('message', (data) => {
  if (data instanceof Buffer) {
    console.log('Received binary message from server:', data);
    console.log('As hex:', data.toString('hex'));
    // Try to interpret as UTF-8 text if possible
    try {
      const text = data.toString('utf8');
      console.log('As text:', text);
    } catch (e) {
      console.log('Could not interpret as UTF-8 text');
    }
  } else {
    console.log('Received message from server:', data.toString());
  }
});

// Handle errors
ws.on('error', (error) => {
  console.error('WebSocket error:', error);
});

// Connection closed
ws.on('close', (code, reason) => {
  console.log(`Connection closed: Code: ${code}, Reason: ${reason}`);
});

// Keep the script running for a while
setTimeout(() => {
  console.log('Closing connection after timeout');
  ws.close();
  process.exit(0);
}, 10000); // Wait for 10 seconds before closing

console.log('Attempting to connect to WebSocket server at ws://localhost:8000/sync...'); 