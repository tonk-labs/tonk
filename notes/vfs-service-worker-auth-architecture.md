# VFS Service & Service Worker Architecture Report

**Date:** December 2, 2025 **Purpose:** Comprehensive analysis of the service worker and VFS
service, their roles, relationships, and considerations for introducing DID/UCAN-based
authentication.

---

## Table of Contents

1. [Executive Summary](#executive-summary)
2. [Background Context](#background-context)
3. [Current Architecture](#current-architecture)
4. [VFS Service Analysis](#vfs-service-analysis)
5. [Service Worker Analysis](#service-worker-analysis)
6. [Problems & Technical Debt](#problems--technical-debt)
7. [Auth/Identity Integration Approaches](#authidentity-integration-approaches)
8. [Tradeoffs Matrix](#tradeoffs-matrix)
9. [Recommendations](#recommendations)
10. [Further Questions](#further-questions)
11. [Appendix: File References](#appendix-file-references)

---

## Executive Summary

The Tonk stack uses a **service worker** as the central runtime for Tonk applications, mediating all
VFS operations and network synchronization. The **VFSService** is a client-side abstraction that
communicates with the service worker via `postMessage()`. Both components have grown organically and
need consolidation before adding authentication.

**Key Findings:**

1. **VFS Service duplication:** The VFSService is copy-pasted across multiple Tonk example projects
   and should be moved into `@tonk/core` or `@tonk/host-web`.

2. **Service Worker complexity:** At 1749 lines, the service worker mixes too many concerns: VFS
   operations, WebSocket management, caching, path resolution, dev-mode proxying, and lifecycle
   management.

3. **Auth integration points:** Authentication can be added at three levels:
   - **Relay-level:** Network gate where devices authenticate to access sync
   - **Service worker-level:** All VFS/sync operations gated by auth
   - **App-level:** Developer-accessible identity for app logic

4. **Recommendation:** A layered approach—relay for network access control and enforcing
   capabilities, service worker for exposing identity to apps.

---

## Background Context

### What is Tonk?

Tonk is a local-first application platform where:

- **Applications are files** (`.tonk` bundles containing code + data)
- **Data syncs via CRDTs** (Automerge documents)
- **No server required** for basic functionality
- **Optional relay servers** for real-time sync

### The Stack

```
┌────────────────────────────────────────────────────┐
│ Launcher (React + TypeScript)                      │
│ - Desktop UI, Text Editor, Presence                │
│ - Uses VFSService for file operations              │
└───────────────────────┬────────────────────────────┘
                        │ postMessage()
                        ▼
┌────────────────────────────────────────────────────┐
│ Service Worker (host-web)                          │
│ - Intercepts HTTP requests                         │
│ - Manages TonkCore (WASM)                          │
│ - WebSocket sync to relay                          │
│ - Serves files from VFS                            │
└───────────────────────┬────────────────────────────┘
                        │ WebSocket
                        ▼
┌────────────────────────────────────────────────────┐
│ Relay Server (Rust/Axum)                           │
│ - WebSocket sync broker                            │
│ - Bundle storage (S3)                              │
│ - No auth currently                                │
└────────────────────────────────────────────────────┘
```

### Key Packages

| Package          | Location            | Purpose                               |
| ---------------- | ------------------- | ------------------------------------- |
| `@tonk/core`     | `packages/core-js`  | TypeScript wrapper around WASM        |
| `@tonk/host-web` | `packages/host-web` | Browser runtime (service worker + UI) |
| `relay`          | `packages/relay`    | Rust WebSocket sync server            |
| `desktonk`       | `packages/desktonk` | Launcher React application            |
| `launcher`       | `packages/launcher` | Bundle manager with Discord-like UI   |

---

## Current Architecture

### Data Flow: File Write Operation

```
1. App calls: vfs.writeFile('/desktonk/notes.txt', { content: {...} })

2. VFSService:
   - Generates message ID
   - Stores pending promise
   - Posts message to service worker controller

3. Service Worker:
   - Receives message
   - Extracts TonkCore instance
   - Calls tonk.updateFile() or tonk.createFile()
   - Posts response with same ID

4. VFSService:
   - Matches response ID to pending promise
   - Resolves promise
   - App continues

5. Background Sync:
   - TonkCore broadcasts changes via WebSocket
   - Relay forwards to other connected clients
   - Other clients receive via watchFile callbacks
```

### Data Flow: HTTP Request Interception

```
1. Browser requests: http://localhost:5173/app/index.html

2. Service Worker fetch handler:
   - Checks if origin matches
   - Checks if appSlug is set
   - Determines VFS path: /app/index.html
   - Reads from TonkCore VFS
   - Returns Response with content

3. Browser receives file as if from server
```

---

## VFS Service Analysis

**Location:** `packages/desktonk/src/lib/vfs-service.ts` (470 lines)

### Purpose

The VFSService is a **client-side abstraction** that:

- Provides async file operations via promises
- Manages pending request/response matching
- Tracks file and directory watchers
- Handles connection state and reconnection
- Re-establishes watchers after service worker restarts

### Core API

```typescript
class VFSService {
  // Lifecycle
  connect(): Promise<void>;
  initialize(manifestUrl: string, wsUrl: string): Promise<void>;
  destroy(): void;

  // File Operations
  readFile(path: string): Promise<DocumentData>;
  writeFile(path: string, content: DocumentContent, create?: boolean): Promise<void>;
  deleteFile(path: string): Promise<void>;
  renameFile(oldPath: string, newPath: string): Promise<void>;
  exists(path: string): Promise<boolean>;

  // Binary Operations
  writeFileWithBytes(path: string, content: JsonValue, bytes: Uint8Array): Promise<void>;
  readBytesAsString(path: string): Promise<string>;
  writeStringAsBytes(path: string, stringData: string): Promise<void>;

  // Directory Operations
  listDirectory(path: string): Promise<unknown[]>;

  // Watch Operations
  watchFile(path: string, callback: (doc: DocumentData) => void): Promise<string>;
  unwatchFile(watchId: string): Promise<void>;
  watchDirectory(path: string, callback: (data: any) => void): Promise<string>;
  unwatchDirectory(watchId: string): Promise<void>;

  // State
  isInitialized(): boolean;
  getConnectionState(): ConnectionState;
  onConnectionStateChange(listener: (state: ConnectionState) => void): () => void;
}
```

### Message Protocol

Messages between VFSService and the service worker follow a typed protocol:

**Request Messages:**

```typescript
type VFSWorkerMessage =
  | { type: 'readFile'; id: string; path: string }
  | { type: 'writeFile'; id: string; path: string; content: DocumentContent; create: boolean }
  | { type: 'watchFile'; id: string; path: string }
  | { type: 'initializeFromUrl'; id: string; manifestUrl?: string; wsUrl?: string };
// ... 16 total message types
```

**Response Messages:**

```typescript
type VFSWorkerResponse =
  | { type: 'readFile'; id: string; success: boolean; data?: DocumentData; error?: string }
  | { type: 'fileChanged'; watchId: string; documentData: DocumentData }
  | { type: 'disconnected' }
  | { type: 'reconnected' };
// ... 18 total response types
```

### Duplication Problem

The VFSService exists in **multiple locations**:

1. `packages/desktonk/src/lib/vfs-service.ts`
2. Any other Tonk example app (would need to copy)

The types file (`types.ts`) is also duplicated between:

- `packages/host-web/src/types.ts`
- `packages/desktonk/src/lib/types.ts`

**These are nearly identical but have drifted slightly** (e.g., `handshake` message type only in
desktonk version).

### How Components Use VFS

| Component        | Usage                                    | File                                             |
| ---------------- | ---------------------------------------- | ------------------------------------------------ |
| DesktopService   | File CRUD, directory watching, positions | `features/desktop/services/DesktopService.ts`    |
| useEditorVFSSave | Debounced auto-save, thumbnails          | `features/text-editor/hooks/useEditorVFSSave.ts` |
| presenceStore    | Zustand sync middleware                  | `features/presence/stores/presenceStore.ts`      |
| useFileDrop      | File uploads                             | `features/desktop/hooks/useFileDrop.ts`          |
| StoreBuilder     | VFS persistence middleware               | `lib/storeBuilder.ts`                            |

---

## Service Worker Analysis

**Location:** `packages/host-web/src/service-worker.ts` (1749 lines)

### Responsibilities

The service worker currently handles **too many concerns**:

1. **TonkCore Lifecycle**
   - WASM initialization
   - Bundle loading from URL or bytes
   - State persistence via Cache API

2. **VFS Operations**
   - File CRUD (create, read, update, delete)
   - Directory operations
   - File/directory watching

3. **HTTP Request Interception**
   - Serve files from VFS
   - Path resolution (appSlug mapping)
   - Fallback to index.html (SPA routing)
   - Dev mode: proxy to Vite

4. **WebSocket Management**
   - Connect to relay
   - Health monitoring
   - Reconnection with exponential backoff

5. **State Persistence**
   - appSlug in Cache API
   - bundleBytes in Cache API

6. **Message Handling**
   - 18 message types
   - Request/response correlation

### State Machine

```typescript
type TonkState =
  | { status: 'uninitialized' }
  | { status: 'loading'; promise: Promise<{ tonk: TonkCore; manifest: Manifest }> }
  | { status: 'ready'; tonk: TonkCore; manifest: Manifest }
  | { status: 'failed'; error: Error };
```

### Initialization Paths

The service worker supports **four initialization paths**:

1. **Auto-initialize from cache** (`autoInitializeFromCache`)
   - On SW startup, check for cached bundleBytes
   - If found, initialize TonkCore without user interaction

2. **Initialize from URL** (`initializeFromUrl`)
   - Fetch manifest from URL
   - Initialize TonkCore
   - Connect WebSocket

3. **Initialize from bytes** (`initializeFromBytes`)
   - Receive bundle bytes via postMessage
   - Initialize TonkCore
   - Connect WebSocket

4. **Load bundle** (`loadBundle`)
   - Similar to initializeFromBytes
   - Used when switching bundles

### Path Resolution

The service worker uses a complex path resolution algorithm:

```typescript
const determinePath = (url: URL): string => {
  // 1. Get scope path
  const scopePath = new URL(self.registration?.scope ?? self.location.href).pathname;

  // 2. Strip scope from pathname
  const strippedPath = url.pathname.startsWith(scopePath)
    ? url.pathname.slice(scopePath.length)
    : url.pathname;

  // 3. Handle appSlug
  const segments = strippedPath.replace(/^\/+/, '').split('/').filter(Boolean);

  // 4. Default to index.html if needed
  if (pathSegments.length === 0 || url.pathname.endsWith('/')) {
    return `${appSlug}/index.html`;
  }

  return `${appSlug}/${pathSegments.join('/')}`;
};
```

### Dev Mode Handling

When `TONK_SERVE_LOCAL` is true:

- Vite HMR assets proxied to `http://localhost:4001`
- Cache-control headers set to prevent caching
- WebSocket upgrade requests pass through

---

## Problems & Technical Debt

### 1. VFSService Duplication

**Problem:** VFSService is copied across projects instead of being part of the Tonk stack.

**Impact:**

- Bug fixes don't propagate
- Types drift out of sync
- Developers must understand internal details
- No official API surface

**Evidence:**

- `packages/desktonk/src/lib/vfs-service.ts`
- Types in `host-web/src/types.ts` vs `desktonk/src/lib/types.ts` have diverged

### 2. Service Worker Complexity

**Problem:** 1749 lines mixing unrelated concerns.

**Concerns mixed together:**

- TonkCore lifecycle
- VFS operations
- HTTP interception
- WebSocket management
- Dev mode proxying
- State persistence

**Impact:**

- Hard to understand
- Hard to test
- Hard to modify
- Likely contains bugs

### 3. Redundant Initialization Code

**Problem:** Four different initialization paths with duplicated logic.

```typescript
// All of these do similar things:
autoInitializeFromCache();
handleMessage('init');
handleMessage('initializeFromUrl');
handleMessage('initializeFromBytes');
handleMessage('loadBundle');
```

### 4. No Error Recovery

**Problem:** Limited error handling and recovery.

**Examples:**

- If WASM fetch fails, no retry
- If WebSocket disconnects, reconnection can fail silently
- If auto-init fails, user must manually reinitialize

### 5. Verbose Logging

**Problem:** `DEBUG_LOGGING = true` with console.log everywhere.

**Impact:**

- Console flooded with logs
- Hard to find actual errors
- Performance overhead

### 6. Type Safety Gaps

**Problem:** Many `any` types and type assertions.

**Examples:**

```typescript
const watchers = new Map(); // Should be typed
(response as any).type === 'ready'; // Type narrowing issues
```

### 7. No Auth/Identity

**Problem:** No authentication or identity system.

**Impact:**

- Anyone can connect to relay
- No access control on files
- No audit trail
- No capability-based permissions

---

## Auth/Identity Integration Approaches

### Approach 1: Relay-Level Authentication

**Description:** Authenticate at the WebSocket connection level. The relay verifies identity before
allowing sync.

```
┌──────────────────┐       ┌──────────────────┐
│ Service Worker   │       │ Relay Server     │
│                  │       │                  │
│ Connect with:    │──────▶│ Verify:          │
│ - UCAN token     │       │ - Signature      │
│ - Peer ID        │       │ - Expiration     │
│                  │       │ - Capabilities   │
└──────────────────┘       └──────────────────┘
```

**Implementation:**

1. Service worker holds UCAN delegation
2. On WebSocket connect, send auth header/message
3. Relay verifies signature and capabilities
4. Relay allows/denies sync based on space/room

**Pros:**

- Network-level security
- Relay can enforce access control
- No changes needed to TonkCore WASM
- Works with existing apps

**Cons:**

- Apps don't know user identity
- All-or-nothing access
- Relay becomes security boundary
- Offline-first philosophy tension

### Approach 2: Service Worker-Level Authentication

**Description:** Service worker gates all operations. Auth state managed in SW, exposed to apps.

```
┌──────────────────┐       ┌──────────────────┐       ┌──────────────────┐
│ App              │       │ Service Worker   │       │ Relay            │
│                  │──────▶│                  │──────▶│                  │
│ VFS operations   │       │ Check auth       │       │ (No auth)        │
│ Identity queries │       │ Enforce caps     │       │                  │
│                  │◀──────│ Provide identity │◀──────│                  │
└──────────────────┘       └──────────────────┘       └──────────────────┘
```

**Implementation:**

1. Add `authenticate()` message to service worker
2. Store delegation in service worker state
3. Gate VFS operations on valid delegation
4. Expose `getIdentity()` message for apps
5. Include identity in sync messages

**Pros:**

- Apps can access identity
- Fine-grained capability control
- Works offline
- Centralized enforcement

**Cons:**

- Service worker complexity increases
- All Tonk apps must handle auth
- Migration path for existing apps
- SW restart loses auth state (must re-auth)

### Approach 3: App-Level Authentication (Minimal Core)

**Description:** Auth is purely app concern. Core provides identity primitives but no enforcement.

```
┌──────────────────┐       ┌──────────────────┐       ┌──────────────────┐
│ App              │       │ Service Worker   │       │ Relay            │
│                  │       │                  │       │                  │
│ Auth logic       │       │ No auth          │       │ No auth          │
│ Capability check │──────▶│ Pure VFS ops     │──────▶│ Pure sync        │
│ Identity UI      │       │                  │       │                  │
└──────────────────┘       └──────────────────┘       └──────────────────┘
```

**Implementation:**

1. Provide `@tonk/auth` library with WebAuthn + UCAN
2. Apps use library to get identity
3. Apps check capabilities before operations
4. No enforcement at core level

**Pros:**

- Simple core
- Flexible for apps
- No breaking changes
- Developers have full control

**Cons:**

- Security is opt-in
- Each app implements differently
- No protection for relay
- Easy to make mistakes

### Approach 4: Layered Authentication (Recommended)

**Description:** Auth at multiple levels with different responsibilities.

```
┌──────────────────────────────────────────────────────────────────────┐
│ Application Layer                                                    │
│ - User-facing auth UI                                                │
│ - Business logic capabilities                                        │
│ - Identity display (names, avatars)                                  │
├──────────────────────────────────────────────────────────────────────┤
│ Service Worker Layer                                                 │
│ - Holds operator keypair + delegation                                │
│ - Gates VFS operations on valid delegation                           │
│ - Provides identity to apps via message                              │
│ - Signs sync messages with operator key                              │
├──────────────────────────────────────────────────────────────────────┤
│ Relay Layer                                                          │
│ - Verifies sync messages are signed                                  │
│ - Enforces space/room access based on rootId                         │
│ - Rate limiting per identity                                         │
│ - Audit logging                                                      │
└──────────────────────────────────────────────────────────────────────┘
```

**Implementation:**

1. **Authority derivation (app layer):**

   ```typescript
   const authority = await webAuthnAuth.deriveAuthority();
   // did:key:z6Mk... derived from biometrics
   ```

2. **Operator registration (service worker):**

   ```typescript
   // SW generates or loads operator keypair
   const operator = await getOrCreateOperator();
   // did:key:z6Mk... stored in IndexedDB
   ```

3. **Delegation flow:**

   ```typescript
   // Authority delegates to operator
   const delegation = {
     iss: authority.did,
     aud: operator.did,
     cmd: '/', // Full access
     sub: null, // Powerline delegation
     exp: Date.now() + 30 * 24 * 60 * 60 * 1000,
   };
   const signed = await sign(delegation, authority.privateKey);

   // SW stores delegation
   await serviceWorker.postMessage({ type: 'setDelegation', delegation: signed });
   ```

4. **Relay verification:**

   ```rust
   // On each sync message
   fn verify_sync(msg: &SyncMessage) -> Result<()> {
       let delegation = &msg.delegation;

       // Verify signature
       verify_ed25519(&delegation.signature, &delegation.payload, &delegation.iss)?;

       // Check expiration
       ensure!(delegation.exp > now(), "Delegation expired");

       // Check room access (rootId in sub or wildcard)
       ensure!(can_access(&delegation, &msg.room_id), "Access denied");

       Ok(())
   }
   ```

5. **App identity access:**

   ```typescript
   // App can query identity
   const identity = await vfs.getIdentity();
   // { operatorDid: "did:key:...", authorityDid: "did:key:..." }

   // Use for presence, attribution, etc.
   await vfs.writeFile('/presence/me.json', {
     did: identity.operatorDid,
     name: generateName(identity.operatorDid),
     lastSeen: Date.now(),
   });
   ```

**Pros:**

- Defense in depth
- Apps get identity
- Relay has access control
- Offline still works (with cached delegation)
- Graceful degradation

**Cons:**

- Most complex to implement
- All layers need changes
- Migration path complexity

---

## Tradeoffs Matrix

| Aspect                        | Relay-Level | SW-Level  | App-Level | Layered   |
| ----------------------------- | ----------- | --------- | --------- | --------- |
| **Implementation Complexity** | Low         | Medium    | Low       | High      |
| **Security Strength**         | Medium      | High      | Low       | High      |
| **App Developer Experience**  | Poor        | Medium    | Good      | Good      |
| **Offline Support**           | Poor        | Good      | Good      | Good      |
| **Breaking Changes**          | Low         | Medium    | None      | Medium    |
| **Flexibility**               | Low         | Medium    | High      | High      |
| **Audit Trail**               | Good        | Medium    | Poor      | Good      |
| **Time to Implement**         | 1-2 weeks   | 3-4 weeks | 1 week    | 6-8 weeks |

---

## Recommendations

### Immediate Actions

1. **Consolidate VFSService into `@tonk/host-web`**
   - Move `vfs-service.ts` and `types.ts` into host-web package
   - Export from `@tonk/host-web/client` or similar
   - Update Launcher to import from package

2. **Refactor Service Worker**
   - Split into modules:
     - `tonk-lifecycle.ts` (init, state machine)
     - `vfs-handlers.ts` (file operations)
     - `fetch-handler.ts` (HTTP interception)
     - `websocket-manager.ts` (connection, health)
   - Reduce to ~500-600 lines per module
   - Add proper TypeScript types

3. **Unify Initialization Paths**
   - Single `initialize(config)` method
   - Config object with source: 'url' | 'bytes' | 'cache'
   - Remove redundant code paths

### Auth Integration Plan

**Phase 1: Foundation (2-3 weeks)**

- Add `@tonk/auth` library with WebAuthn + Ed25519
- Operator keypair in IndexedDB
- Delegation creation and verification
- No enforcement yet

**Phase 2: Service Worker Auth (2-3 weeks)**

- Add `setDelegation` message type
- Store delegation in SW state (+ Cache API backup)
- Add `getIdentity` message type
- Optional: gate VFS operations

**Phase 3: Relay Auth (2-3 weeks)**

- Add signature to sync messages
- Relay verifies signatures
- Room access based on delegation `sub` field
- Reject unsigned/invalid messages

**Phase 4: App Integration (1-2 weeks)**

- Update Launcher to use auth
- Login flow with WebAuthn
- Identity in presence system
- Capability checks for actions

---

## Further Questions

### Architecture Questions

1. **Should VFSService live in `@tonk/core` or `@tonk/host-web`?**
   - Core: More accessible, but couples to service worker pattern
   - Host-web: More appropriate, but requires separate package for types

2. **Should auth be mandatory or opt-in?**
   - Mandatory: Better security, but breaking change
   - Opt-in: Easier migration, but security gaps

3. **How should offline-first and auth interact?**
   - Cached delegation allows offline writes
   - What about expired delegations?
   - Should there be "offline mode" with reduced capabilities?

4. **Multi-device identity: same authority or separate?**
   - Same: Requires syncing private key (dangerous)
   - Separate: Each device is separate authority delegating to same operator
   - Implications for revocation and audit

### Implementation Questions

5. **Should the service worker be replaced with SharedWorker?**
   - SharedWorker: More natural for multi-tab, but SW needed for fetch interception
   - Hybrid: SharedWorker for state, SW for fetch?
   - Current approach: SW handles everything

6. **How should delegation storage work?**
   - Cache API: Survives SW restart, but cleared on cache clear
   - IndexedDB: More durable, but needs async access in SW
   - Both: Redundancy

7. **What happens when delegation expires mid-session?**
   - Block operations and prompt re-auth?
   - Grace period?
   - Queue operations for later?

8. **How do we handle bundle-specific capabilities?**
   - Delegation with `sub: "did:tonk:bundle:{rootId}"`
   - Wildcard patterns?
   - Inheritance from parent delegations?

### Security Questions

9. **How do we handle compromised operator keys?**
   - Revocation mechanism?
   - Key rotation?
   - Authority can revoke by creating empty delegation?

10. **Should file changes be signed?**
    - Every write signed by operator?
    - Performance implications?
    - Storage overhead?

11. **How do we prevent replay attacks?**
    - Nonces in messages?
    - Timestamp windows?
    - Sequence numbers per room?

---

## Appendix: File References

### Service Worker

- **Main file:** `packages/host-web/src/service-worker.ts` (1749 lines)
- **Types:** `packages/host-web/src/types.ts` (132 lines)

### VFS Service

- **App version:** `packages/desktonk/src/lib/vfs-service.ts` (470 lines)
- **App types:** `packages/desktonk/src/lib/types.ts` (125 lines)

### TonkCore

- **TypeScript wrapper:** `packages/core-js/src/core.ts` (1102 lines)
- **WASM source:** `packages/core/src/` (Rust)

### Relay Server

- **Server:** `packages/relay/src/server.rs` (382 lines)
- **Network handler:** `packages/relay/src/network.rs`

### Bundle Format

- **Specification:** `TONK_FORMAT.md`

---

## Conclusion

The VFS service and service worker form the critical infrastructure for Tonk applications. Before
adding authentication, the codebase needs consolidation:

1. **VFSService** should become a first-class citizen of the Tonk stack
2. **Service worker** should be modularized and simplified
3. **Auth** should be added in layers, with the service worker as the enforcement point and relay as
   the network gate

The recommended **Layered Authentication** approach provides the best balance of security, developer
experience, and offline-first philosophy, but requires the most implementation effort. Starting with
foundation work (cleanup + `@tonk/auth` library) allows incremental progress toward the full vision.
