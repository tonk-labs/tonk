# Write-View Prompt

## Purpose

You are scaffolding a new Tonk View — a React-based frontend application that integrates with the Tonk ecosystem for building local-first applications with real-time data synchronization. Your primary goal is to help a semi-technical developer define the view's structure by asking clarifying questions and generating a spec document with checklists for any remaining setup tasks.

This is a collaborative process. Ask questions before making assumptions. Always check existing project files and instructions before generating or modifying code.

## Before You Start

1. **Location**: ALWAYS run `tonk create` from inside the `views/` directory
2. **Creation**: Views are created using `tonk create` - this scaffolds the complete structure
3. **Directory**: Ensure the new view is created as a subdirectory within `views/`

## Key Architecture Patterns

### Project Structure

A Tonk view includes:

- `src/` - React application source code
  - `views/` - Page-level components with routing
  - `components/` - Reusable UI elements
  - `stores/` - Zustand state management
  - `services/` - API integration
- `server/` - Express.js backend server
- `instructions/` - LLM-readable documentation
- `public/` - Static assets
- Configuration files: `tonk.config.json`, `vite.config.ts`, `package.json`

### Core Technologies

- **Frontend**: React, TypeScript, Vite, Tailwind CSS, React Router
- **State**: Zustand for global state, KeepSync for sync document state
- **Backend**: Express.js with CORS support
- **Sync**: KeepSync with WebSocket networking and IndexedDB storage
- **Icons**: Lucide React
- **PWA**: Progressive Web App capabilities enabled

## Development Workflow

### Initial Setup

```bash
cd views/  # CRITICAL: Must be in views directory
tonk create  # Select "react" when prompted
cd view-name/
pnpm install
```

### Running the View

```bash
# CRITICAL: Always run from within the view directory
cd views/your-view-name/
pnpm install      # Install dependencies
pnpm dev         # Start both frontend and backend
pnpm dev:client  # Frontend only (port 3000)
pnpm dev:server  # Backend only
pnpm build       # Production build
pnpm serve       # Preview production build
```

### Development Ports

- Frontend: `http://localhost:3000`
- Sync proxy: WebSocket to `ws://localhost:7777/sync`
- API proxy: HTTP to `http://localhost:6080/api`

## Configuration Files

### tonk.config.json

Tonk platform integration:

- View name and description
- Platform type ("react")
- Template reference
- Project metadata

### vite.config.ts

Build configuration defining:

- **Plugins**: React plugin, PWA support, WebAssembly
- **Dev Server**: Proxy configuration for sync and API endpoints
- **Build**: ESNext target with vendor chunking optimization
- **Environment**: Development and production settings

## Critical: KeepSync Integration

### Document Operations

**⭐ ALWAYS use KeepSync for synchronized data state**
**DO NOT use KeepSync for purely local data state**

```typescript
import { useKeepSyncDoc } from "@tonk/keepsync-react";

// 1. Subscribe to documents
const doc = useKeepSyncDoc("document-id");

// 2. Update documents
const updateDoc = (changes) => {
  keepsync.change(doc, (draft) => {
    // Apply changes to draft - automatically synced
    draft.items.push(newItem);
  });
};

// 3. Handle loading states
if (!doc) return <LoadingSpinner />;
```

## Creation of the View Task Checklist

```bash
cd views/view-name
touch task-checklist.md
```

#### Example Checklist

```markdown
## Overview

Create a collaborative task management application that syncs tasks in real-time across multiple users, with offline support and conflict resolution.

## Data Flow
```

User Input → Zustand Store → KeepSync Document → Real-time Sync → Other Users

```

## Outstanding LLMs.txt advice
- Use React.memo for performance optimization
- Implement proper loading states for all async operations

## Implementation Steps

### Phase 1: Basic Task Management
- [ ] Create view: `task-manager`
- [ ] Set up KeepSync document structure for tasks
- [ ] Create task list component with CRUD operations
- [ ] Implement task filtering and sorting
- [ ] Add responsive design for mobile/desktop

### Phase 2: Real-time Collaboration
- [ ] Integrate KeepSync for multi-user sync
- [ ] Handle conflict resolution for simultaneous edits
- [ ] Add user presence indicators
- [ ] Implement optimistic UI updates
- [ ] Show sync status and offline indicators

### Phase 3: Advanced Features
- [ ] Add task categories and tagging system
- [ ] Implement due dates and reminders
- [ ] Create task assignment functionality
- [ ] Add search and filter capabilities
- [ ] Include data export functionality

### Phase 4: UI/UX Polish
- [ ] Implement dark mode support
- [ ] Add keyboard shortcuts
- [ ] Create onboarding flow
- [ ] Add accessibility features (ARIA labels, focus management)
- [ ] Optimize performance with React.memo and lazy loading
```

## Example Initial Agent Prompt

I'm helping you set up a new Tonk View. To scaffold it properly, I need a few details:

1. **What is this view's main purpose?** (e.g., data visualization, user management, content editing)
2. **What data will it display?** Should it connect to existing KeepSync documents or create new ones?

After collecting this information, I'll:

- Run tonk create in /views
- Begin coding the view structure
- Create the implementation checklist at root of workspace project /checklist.md
