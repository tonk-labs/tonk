# AI Worker

A Tonk worker that provides a unified API for interacting with Large Language Models (LLMs) and vector databases. Currently supports OpenAI's GPT models and includes an integrated Chroma vector database for RAG (Retrieval-Augmented Generation) capabilities.

## Quick Start

### 1. Setup Credentials

```bash
cd workers/ai
npx tsx dist/cli.js setup
```

This will prompt you to paste your OpenAI API key. Get one from [OpenAI API Keys](https://platform.openai.com/api-keys).

### 2. Start the Worker

```bash
pnpm start
```

The worker will start on `http://localhost:5556` with API endpoints at `/api/`.
Additionally, it will automatically start a Chroma vector database server on `http://localhost:8888`.

**Prerequisites for Chroma:**
- Docker (recommended) OR
- Python with `pip install chromadb`

The worker will attempt to start Chroma via Docker first, then fall back to a local Python installation if Docker is unavailable.

## API Endpoints

### GET /health

Health check endpoint that shows the status of both AI worker and Chroma server.

```bash
curl http://localhost:5556/health
```

**Response:**
```json
{
  "status": "ok",
  "services": {
    "ai": "running",
    "chroma": "running"
  }
}
```

### GET /api/providers

Lists available LLM providers and their configuration status.

```bash
curl http://localhost:5556/api/providers
```

**Response:**
```json
{
  "providers": [
    {
      "name": "openai",
      "models": ["gpt-4", "gpt-4-turbo", "gpt-3.5-turbo"],
      "configured": true
    }
  ]
}
```

### POST /api/chat

Chat completion with message history support.

**Request:**
```json
{
  "messages": [
    {"role": "system", "content": "You are a helpful assistant."},
    {"role": "user", "content": "Hello!"}
  ],
  "model": "gpt-3.5-turbo",
  "temperature": 0.7,
  "max_tokens": 150,
  "stream": false
}
```

**Response (Non-streaming):**
```json
{
  "content": "Hello! How can I help you today?",
  "usage": {
    "prompt_tokens": 20,
    "completion_tokens": 9,
    "total_tokens": 29
  },
  "model": "gpt-3.5-turbo",
  "provider": "openai"
}
```

### POST /api/complete

Simple text completion interface.

**Request:**
```json
{
  "prompt": "The future of AI is",
  "system": "You are a technology expert.",
  "model": "gpt-3.5-turbo",
  "temperature": 0.8,
  "stream": false
}
```

## Streaming Responses

Both `/api/chat` and `/api/complete` support streaming by setting `"stream": true`.

**Example:**
```bash
curl -X POST http://localhost:5556/api/complete \
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
| `messages` | Array | Chat messages for `/api/chat` | Required |
| `prompt` | String | Text prompt for `/api/complete` | Required |
| `system` | String | System message for `/api/complete` | Optional |
| `model` | String | Model to use | `gpt-3.5-turbo` |
| `temperature` | Number | Randomness (0-2) | `0.7` |
| `max_tokens` | Number | Maximum response length | OpenAI default |
| `stream` | Boolean | Enable streaming | `false` |
| `provider` | String | LLM provider to use | `openai` |

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
