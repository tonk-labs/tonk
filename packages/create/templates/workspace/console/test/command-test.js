#!/usr/bin/env node

/**
 * Test script for command execution RPC endpoint
 * Usage: node test/command-test.js [command] [arg1] [arg2] ...
 * 
 * Examples:
 *   node test/command-test.js ls -la
 *   node test/command-test.js echo "Hello World"
 *   node test/command-test.js npm --version
 *   node test/command-test.js pwd
 */

const SERVER_URL = 'http://localhost:6080';

async function executeCommand(command, args = [], cwd = null) {
  console.log(`\n🚀 Executing: ${command} ${args.join(' ')}`);
  console.log('─'.repeat(50));
  
  try {
    const controller = new AbortController();
    
    const response = await fetch(`${SERVER_URL}/api/execute`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'Connection': 'keep-alive',
      },
      body: JSON.stringify({
        command,
        args,
        cwd
      }),
      signal: controller.signal
    });

    if (!response.ok) {
      const errorText = await response.text();
      console.error(`❌ HTTP Error ${response.status}: ${errorText}`);
      return;
    }

    // Stream the response
    const reader = response.body.getReader();
    const decoder = new TextDecoder();

    try {
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        
        const chunk = decoder.decode(value, { stream: true });
        process.stdout.write(chunk);
      }
    } finally {
      reader.releaseLock();
    }

    console.log('\n' + '─'.repeat(50));
    console.log('✅ Command completed');
    
  } catch (error) {
    console.error(`❌ Error: ${error.message}`);
  }
}

// Parse command line arguments
const args = process.argv.slice(2);

if (args.length === 0) {
  console.log(`
📋 Command Test Script
Usage: node test/command-test.js [command] [arg1] [arg2] ...

Examples:
  node test/command-test.js ls -la
  node test/command-test.js echo "Hello World"
  node test/command-test.js npm --version
  node test/command-test.js pwd
  node test/command-test.js date
  node test/command-test.js whoami
  node test/command-test.js sleep 3
  
For interactive testing, try:
  node test/command-test.js node -e "console.log('Hello from Node.js!')"
  `);
  process.exit(0);
}

const command = args[0];
const commandArgs = args.slice(1);

// Execute the command
executeCommand(command, commandArgs); 