# AI Worker

A Tonk worker that provides a unified API for interacting with Claude via the Anthropic API with full MCP (Model Context Protocol) support for tools and function calling. Designed for orchestration workflows and complex AI tasks.

## Quick Start

### 1. Setup Environment

Copy the example environment file and configure your API key:

```bash
cp .env.example .env
```

Edit `.env` and add your Anthropic API key:

```bash
ANTHROPIC_API_KEY=your_anthropic_api_key_here
```

Get your API key from [Anthropic Console](https://console.anthropic.com/).

### 2. Install Dependencies

```bash
pnpm install
```

### 3. Start the Worker

```bash
pnpm start
```

The worker will start on `http://localhost:5556` with API endpoints at `/api/`.

**Prerequisites for MCP:**
- Node.js 18+ (for MCP server support)
- MCP servers you want to use (e.g., browsermcp, gmail-mcp, etc.)

The worker will automatically spawn and manage MCP servers as defined in your templates.

## API Endpoints

### GET /health

Health check endpoint that shows the status of the AI worker and Claude API configuration.

```bash
curl http://localhost:5556/health
```

**Response:**
```json
{
  "status": "ok",
  "services": {
    "ai": "running",
    "keepsync": "running",
    "claude": "configured"
  }
}
```

### POST /api/tonk

Main endpoint for Claude API completion with MCP tool support.

```bash
curl -X POST http://localhost:5556/api/tonk \
  -H "Content-Type: application/json" \
  -d '{
    "prompt": "Search the web for information about MCP",
    "allowedTools": ["browsermcp"],
    "mcpConfig": {
      "browsermcp": {
        "command": "npx",
        "args": ["@browsermcp/mcp@latest"]
      }
    }
  }'
```

**Response:**
```json
{
  "content": "I found information about MCP...",
  "totalCostUsd": 0.0123,
  "sessionId": "msg_abc123",
  "provider": "claude-api",
  "success": true
}
```

## Streaming Responses

The `/api/tonk` endpoint supports streaming by setting `"stream": true`.

**Example:**
```bash
curl -X POST http://localhost:5556/api/tonk \
  -H "Content-Type: application/json" \
  -d '{
    "prompt": "Write a short poem about coding",
    "stream": true
  }'
```

**Response:**
- Content-Type: `text/plain`
- Transfer-Encoding: `chunked`
- Text streams in real-time as it's generated

## Parameters

| Parameter | Type | Description | Default |
|-----------|------|-------------|---------|
| `prompt` | String | Text prompt for completion | Required |
| `systemPrompt` | String | System message | Optional |
| `maxTurns` | Number | Maximum conversation turns | `10` |
| `allowedTools` | Array | List of allowed MCP tools | `[]` |
| `disallowedTools` | Array | List of disallowed MCP tools | `[]` |
| `mcpConfig` | Object | MCP server configurations | `{}` |
| `permissionMode` | String | Permission mode for tools | `"default"` |
| `stream` | Boolean | Enable streaming | `false` |
| `verbose` | Boolean | Enable verbose logging | `false` |

## MCP Integration

The AI worker supports MCP (Model Context Protocol) for tool integration. Configure MCP servers in your requests:

```json
{
  "prompt": "Search for flights to Paris",
  "allowedTools": ["browsermcp"],
  "mcpConfig": {
    "browsermcp": {
      "command": "npx",
      "args": ["@browsermcp/mcp@latest"]
    }
  }
}
```

**Supported Permission Modes:**
- `"default"`: Prompt for potentially dangerous operations
- `"acceptEdits"`: Auto-approve file edits, prompt for commands
- `"bypassPermissions"`: Skip all permission prompts
- `"plan"`: Show execution plan before running

## Development

### Setup
```bash
cd workers/ai
pnpm install
pnpm build
```

### Development Mode
```bash
pnpm dev
```

### CLI Commands
```bash
# Setup credentials
npx tsx dist/cli.js setup

# Start worker (default ports: AI=5556, Chroma=8888)
npx tsx dist/cli.js start

# Start on custom ports
npx tsx dist/cli.js start --port 3000 --chroma-port 8001

# Environment variables
WORKER_PORT=5556 CHROMA_PORT=8888 npx tsx dist/cli.js start
```

## Architecture

### LLM Provider Interface

The worker uses a flexible provider interface that makes adding new LLMs straightforward:

```typescript
interface LLMProvider {
  name: string;
  models: string[];
  complete(request: LLMRequest): Promise<LLMResponse>;
  stream(request: LLMRequest): AsyncIterable<string>;
  isConfigured(): boolean;
}
```

### Adding New Providers

To add a new LLM provider (e.g., Llama):

1. Implement the `LLMProvider` interface
2. Add credential configuration to `BaseCredentialsManager`
3. Register the provider in the `LLMService`

Example:
```typescript
class LlamaProvider implements LLMProvider {
  name = 'llama';
  models = ['llama-2-7b', 'llama-2-13b'];
  
  async complete(request: LLMRequest): Promise<LLMResponse> {
    // Implementation
  }
  
  async* stream(request: LLMRequest): AsyncIterable<string> {
    // Implementation
  }
  
  isConfigured(): boolean {
    // Check if credentials exist
  }
}
```

### File Structure

```
src/
├── index.ts              # Main HTTP server and routes
├── cli.ts                # Command-line interface
├── services/
│   └── llmProvider.ts    # LLM provider interfaces and implementations
└── utils/
    └── baseCredentialsManager.ts # Credential management
```

## Security

- Credentials are stored in `creds/` directory (git-ignored)
- API keys are validated before storage
- No credentials are logged or exposed in responses
- CORS headers configured for cross-origin requests

## Error Handling

The API returns appropriate HTTP status codes:

- `200` - Success
- `400` - Bad request (missing parameters)
- `404` - Endpoint not found
- `500` - Server error (including LLM provider errors)

Error responses include descriptive messages:
```json
{
  "error": "prompt is required"
}
```

## Integration with Tonk Ecosystem

This worker integrates with the Tonk's `keepsync` sync engine for data synchronization and can be used by other Tonk components to add AI capabilities to your workspace.

## Contributing

When adding new features:

1. Follow the existing provider interface pattern
2. Add appropriate error handling
3. Update this README with new endpoints or parameters
4. Test both streaming and non-streaming modes

## License

MIT © Tonk Labs
