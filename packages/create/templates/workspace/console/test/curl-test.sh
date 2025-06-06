#!/bin/bash

echo "Testing RPC endpoint with curl..."
echo "=================================="

# Test with pwd command
echo "Running: pwd"
curl -X POST http://localhost:6080/api/execute \
  -H "Content-Type: application/json" \
  -d '{"command": "pwd", "args": []}' \
  --no-buffer

echo ""
echo "=================================="

# Test with echo command
echo "Running: echo 'Hello World'"
curl -X POST http://localhost:6080/api/execute \
  -H "Content-Type: application/json" \
  -d '{"command": "echo", "args": ["Hello World"]}' \
  --no-buffer

echo ""
echo "=================================="

# Test with ls command
echo "Running: ls -la"
curl -X POST http://localhost:6080/api/execute \
  -H "Content-Type: application/json" \
  -d '{"command": "ls", "args": ["-la"]}' \
  --no-buffer

echo ""
echo "=================================="
echo "All tests completed!" 