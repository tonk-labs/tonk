# Write-Worker Prompt

## Purpose

You are scaffolding a new Tonk Worker — a Node.js-based microservice that integrates with the Tonk ecosystem. Your primary goal is to help a semi-technical developer define the worker’s structure by asking clarifying questions and generating a spec document with checklists for any remaining setup tasks.

This is a collaborative process. Ask questions before making assumptions. Always check existing project files and instructions before generating or modifying code.

## Before You Start

1. **Location**: ALWAYS run `tonk create` from inside the `workers/` directory
2. **Creation**: Workers are created using `tonk create` - this scaffolds the complete structure
3. **Directory**: Ensure the new worker is created as a subdirectory within `workers/`

## Key Architecture Patterns

### Project Structure

A Tonk worker includes:

- `src/` - Main application source code
  - `index.ts` - HTTP server entry point
  - `cli.ts` - CLI interface for worker control
  - `utils/` - Utility modules
  - `listeners/` - Event listeners (file, keepsync)
  - `io/` - I/O handlers and types
- `instructions/` - LLM-readable documentation
- `creds/` - Credential storage (auto-created)
- Configuration files: `worker.config.js`, `tonk.config.json`, `package.json`

## Development Workflow

### Initial Setup

```bash
cd workers/  # CRITICAL: Must be in workers directory
tonk create  # Select "worker" when prompted
cd worker-name/
pnpm install
```

### Running the Worker

```bash
# CRITICAL: Always run from within the worker directory
cd workers/your-worker-name/
pnpm install      # Install dependencies
pnpm dev         # Start with hot reload
pnpm build       # Compile TypeScript
pnpm start       # Run production build
```

### Development Ports

- Worker HTTP server: `http://localhost:5555` (configurable)
- Health check: `http://localhost:5555/health`
- Main endpoint: `http://localhost:5555/tonk`
- KeepSync: WebSocket to `ws://localhost:7777/sync`

## Configuration Files

### worker.config.js

Primary configuration defining:

- **Runtime**: Port, health endpoints, intervals
- **Process**: File execution, instances, auto-restart, memory limits
- **CLI**: Script paths and command arguments
- **Schemas**: Data validation schemas for KeepSync documents
- **Environment**: Production/development settings

### tonk.config.json

Tonk platform integration:

- Worker name and description
- Platform type ("worker")
- Template reference
- Project metadata

### Environment Variables

```bash
SYNC_WS_URL=ws://localhost:7777/sync    # KeepSync WebSocket
SYNC_URL=http://localhost:7777          # KeepSync HTTP
WORKER_PORT=5555                        # Worker HTTP port
NODE_ENV=development                    # Runtime environment
```

## Critical: Credential Management

### BaseCredentialsManager Usage

**⭐ ALWAYS use BaseCredentialsManager for external API integration (Google APIs, OpenAI, etc.)**

```typescript
import { BaseCredentialsManager } from "./utils/BaseCredentialsManager";

// 1. Define credential configurations
const credentialsManager = new BaseCredentialsManager([
  {
    name: "Google Service Account",
    filename: "google_service_account.json",
    description: "Google Cloud service account for API access",
    instructions:
      "Download from Google Cloud Console > IAM & Admin > Service Accounts",
    validationFn: (content) => {
      try {
        const parsed = JSON.parse(content);
        return {
          valid: !!parsed.client_email,
          message: "Must be valid service account JSON",
        };
      } catch {
        return { valid: false, message: "Must be valid JSON" };
      }
    },
  },
  {
    name: "OpenAI API Key",
    filename: "openai_api_key.txt",
    description: "OpenAI API key for processing",
    instructions: "Get from https://platform.openai.com/api-keys",
    header: "Authorization", // Will add as "Authorization: Bearer <key>"
  },
]);

// 2. Initialize and check credentials
await credentialsManager.init();
const { complete, missing } = await credentialsManager.checkCredentials();
if (!complete) {
  await credentialsManager.setupCredentials();
}

// 3. Use credentials in your code
const googleCreds = await credentialsManager.getCredentialByName(
  "Google Service Account"
);
const openaiKey = await credentialsManager.getCredentialByName(
  "OpenAI API Key"
);
```

### Credential Types Supported

- **API Keys**: Simple text-based keys with optional headers
- **JSON Credentials**: Complex JSON objects (service accounts, OAuth tokens)
- **Certificates**: PEM/CRT files for TLS authentication
- **Custom Validation**: Define validation functions for credential format

### Creation of the Worker Task Checklist

```bash
cd workers/worker-name
touch task-checklist.md

```

#### Example Checklist

```markdown
## Overview

Create a daily report generator that automatically processes topics of interest from Obsidian notes, generates research queries, searches the web, summarizes findings, and presents them in a React application.

## Data Flow
```

Obsidian Note → Parse Topics → Generate Queries → Web Search → Read Websites → Summarize → Daily Report → React View

```

## Outstanding LLMs.txt advice
- Have the agent use git to checkpoint work

## Implementation Steps

### Phase 1: Obsidian Note Parser Worker
- [ ] Create worker: `obsidian-notes`
- [ ] Read file from: `/Users/goblin/Library/Mobile Documents/iCloud~md~obsidian/Documents/Notes/Topics of Interest.md`
- [ ] Parse paragraphs to extract topics of interest
- [ ] Store parsed topics in keepsync with timestamp
- [ ] Handle file watching for automatic updates

### Phase 2: Query Generation Worker
- [ ] Create worker: `query-generator`
- [ ] Listen to keepsync for new topics from obsidian-note-parser
- [ ] Integrate OpenAI API for query generation
- [ ] Generate 3-5 useful search queries per topic
- [ ] Store queries in keepsync linked to original topics

### Phase 3: Web Search and Content Worker
- [ ] Create worker: `web-search-content`
- [ ] Listen to keepsync for new queries from query-generator
- [ ] Integrate Brave Search API
- [ ] Search each query and collect results
- [ ] Extract and read website content from search results
- [ ] Store raw search results and website content in keepsync

### Phase 4: Content Summarization Worker
- [ ] Create worker: `content-summarizer`
- [ ] Listen to keepsync for new web content from web-search-content
- [ ] Integrate OpenAI API for summarization
- [ ] Generate multi-paragraph summaries with citations
- [ ] Maintain source links for each summary
- [ ] Organize summaries by day in keepsync structure

### Phase 5: Daily Report Aggregation Worker
- [ ] Create worker: `daily-report-aggregator`
- [ ] Listen to keepsync for completed daily summaries
- [ ] Collect all summaries for a given day
- [ ] Use OpenAI API to create comprehensive daily report
- [ ] Store final report in keepsync with metadata

### Phase 6: Report Viewer React Application
- [ ] Create view: `daily-reports-viewer`
- [ ] Display daily reports with navigation by date
- [ ] Show individual summaries within each report
- [ ] Include clickable source links
- [ ] Add search/filter functionality for reports
- [ ] Show processing status and progress
```

Your Responsibilities as the AI Agent 1. Ask the user:
• What is the primary function of the worker?
• What credentials or third-party APIs are needed?
• Should it react to file system events or timers?
• Read and respect existing files, templates, and metadata
• Do not delete or overwrite scaffolded files unless specifically allowed
• Modify configuration files only when necessary and clearly commented

Example Initial Agent Prompt

I’m helping you set up a new Tonk Worker. To scaffold it properly, I need a few details: 1. What is this worker’s main function?

After collecting this information, proceed to:
• Scaffold or adapt the worker
• Populate a spec/checklist at the root of the workspace named checklist.md
• Stub out endpoint or listener handlers
