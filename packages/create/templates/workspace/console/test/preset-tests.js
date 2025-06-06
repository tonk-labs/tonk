#!/usr/bin/env node

/**
 * Preset test commands for the RPC endpoint
 * This script runs a series of common commands to test the functionality
 */

const SERVER_URL = 'http://localhost:6080';

const TEST_COMMANDS = [
  {
    name: 'Basic echo test',
    command: 'echo',
    args: ['Hello from RPC!']
  },
  {
    name: 'List current directory',
    command: 'ls',
    args: ['-la']
  },
  {
    name: 'Show current directory',
    command: 'pwd',
    args: []
  },
  {
    name: 'Node.js version',
    command: 'node',
    args: ['--version']
  },
  {
    name: 'NPM version',
    command: 'npm',
    args: ['--version']
  },
  {
    name: 'Current user',
    command: 'whoami',
    args: []
  },
  {
    name: 'Date and time',
    command: 'date',
    args: []
  },
  {
    name: 'Sleep test (3 seconds)',
    command: 'sleep',
    args: ['3']
  },
  {
    name: 'Multi-line output test',
    command: 'node',
    args: ['-e', 'for(let i=1; i<=5; i++) { console.log(`Line ${i}`); }']
  },
  {
    name: 'Error test (invalid command)',
    command: 'invalid-command-that-does-not-exist',
    args: []
  }
];

async function executeCommand(command, args = [], cwd = null) {
  try {
    const response = await fetch(`${SERVER_URL}/api/execute`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({
        command,
        args,
        cwd
      })
    });

    if (!response.ok) {
      const errorText = await response.text();
      console.error(`❌ HTTP Error ${response.status}: ${errorText}`);
      return;
    }

    // Stream the response
    const reader = response.body.getReader();
    const decoder = new TextDecoder();

    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      
      const chunk = decoder.decode(value, { stream: true });
      process.stdout.write(chunk);
    }
    
  } catch (error) {
    console.error(`❌ Error: ${error.message}`);
  }
}

async function runTests() {
  console.log('🧪 Running RPC Command Tests');
  console.log('=' .repeat(60));
  
  for (let i = 0; i < TEST_COMMANDS.length; i++) {
    const test = TEST_COMMANDS[i];
    console.log(`\n${i + 1}. ${test.name}`);
    console.log(`   Command: ${test.command} ${test.args.join(' ')}`);
    console.log('   ' + '─'.repeat(50));
    
    await executeCommand(test.command, test.args);
    
    console.log('   ' + '─'.repeat(50));
    console.log('   ✅ Test completed\n');
    
    // Small delay between tests
    await new Promise(resolve => setTimeout(resolve, 500));
  }
  
  console.log('=' .repeat(60));
  console.log('🎉 All tests completed!');
}

// Check if server is running first
async function checkServer() {
  try {
    const response = await fetch(`${SERVER_URL}/api/health`);
    if (response.ok) {
      console.log('✅ Server is running on port 6080');
      return true;
    }
  } catch (error) {
    console.error('❌ Server not running. Please start the server first:');
    console.error('   pnpm dev:server');
    return false;
  }
}

// Run the tests
async function main() {
  const serverRunning = await checkServer();
  if (serverRunning) {
    await runTests();
  }
}

main(); 