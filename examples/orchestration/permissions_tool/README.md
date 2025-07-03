# Permissions MCP Server

A simple Model Context Protocol (MCP) server that provides permission prompting functionality for Claude Code SDK. This server allows Claude to request user permissions via command-line prompts.

## Features

- Provides an `approve` tool that prompts users for yes/no responses
- Integrates with Claude Code SDK's permission system
- Simple command-line interface for user interaction

## Installation

```bash
npm install
# or
pnpm install
```

## Usage

### Standalone Usage

Run the MCP server directly:

```bash
npm start
```

### With Claude Code SDK

1. Create an MCP configuration file (see `mcp-config.json` example):

```json
{
  "mcpServers": {
    "permissions": {
      "command": "node",
      "args": ["index.js"],
      "cwd": "./examples/orchestration/permissions_tool"
    }
  }
}
```

2. Use with Claude Code SDK:

```bash
# Basic usage with MCP permissions tool
claude -p "Deploy the application" \
  --mcp-config mcp-config.json \
  --allowedTools "mcp__permissions__approve" \
  --permission-prompt-tool mcp__permissions__approve
```

### Tool Details

The server provides one tool:

- **`approve`**: Prompts the user for permission approval
  - Parameters:
    - `message` (required): The permission request message to display
    - `action` (optional): Additional context about the action being requested
  - Returns: "APPROVED" or "DENIED" based on user input

## Example Interactions

When Claude needs permission, the tool will prompt:

```
Deploy to production server? (y/n): y
```

The user can respond with:

- `y`, `yes` → APPROVED
- `n`, `no` → DENIED

## MCP Tool Name

When used with Claude Code SDK, this tool is accessible as:

```
mcp__permissions__approve
```

Following the MCP naming convention: `mcp__<serverName>__<toolName>`

## Integration Example

```bash
# Example: Let Claude manage files but ask for permissions
claude -p "Organize the project files and clean up old code" \
  --mcp-config mcp-config.json \
  --allowedTools "Read,Write,mcp__permissions__approve" \
  --permission-prompt-tool mcp__permissions__approve
```

This will allow Claude to:

1. Read and write files
2. Ask for user permission when needed using the permissions tool
3. Wait for user approval via command line before proceeding with sensitive operations
