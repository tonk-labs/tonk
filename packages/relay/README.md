# Tonk Relay Server

A relay server for Tonk bundles that provides WebSocket sync via Automerge and HTTP API endpoints
for bundle storage using AWS S3.

## Features

- **WebSocket Sync**: Real-time collaboration via Automerge
- **Persistent Storage**: Filesystem-based storage for document changes
- **Bundle Loading**: Load initial state from `.tonk` bundle files
- **Bundle Storage**: Upload and download Tonk bundles via S3
- **Manifest Serving**: Serve slim bundles (manifest + root document only)
- **CORS Support**: Configurable cross-origin resource sharing

## Setup

1. Install dependencies:

   ```bash
   pnpm install
   ```

2. Start the server:

   ```bash
   # Development mode with a local bundle
   pnpm dev

   # Or start with custom port, bundle, and storage directory
   tsx src/index.ts 8081 path/to/bundle.tonk ./storage-data
   
   # With defaults (storage-data defaults to 'automerge-repo-data')
   tsx src/index.ts 8081 path/to/bundle.tonk
   ```

## Storage Architecture

The server uses a **composite storage strategy** that combines two storage adapters:

1. **Bundle Storage** (read-only): Loads initial document state from the `.tonk` bundle file
2. **Filesystem Storage** (read-write): Persists all document changes to disk

**How it works:**
- **Reads**: Check filesystem first for latest changes, fall back to bundle for initial state
- **Writes**: All changes are written to the filesystem storage directory
- **Persistence**: Changes survive server restarts and are preserved independently from the bundle

This architecture allows bundles to serve as immutable distribution packages while runtime state evolves in the filesystem storage.

## API Endpoints

### Bundle Management

#### Upload Bundle

Upload a Tonk bundle to S3 storage.

```http
POST /api/bundles
Content-Type: application/octet-stream

<binary bundle data>
```

**Response:**

```json
{
  "id": "abc123...",
  "message": "Bundle uploaded successfully"
}
```

The `id` is extracted from the bundle's manifest `rootId` field.

#### Download Full Bundle

Download a complete Tonk bundle from S3.

```http
GET /api/bundles/:id
```

**Response:**

- Content-Type: `application/octet-stream`
- Binary bundle data

#### Download Bundle Manifest (Slim Bundle)

Download a slim bundle containing only the manifest and root document storage.

```http
GET /api/bundles/:id/manifest
```

**Response:**

- Content-Type: `application/zip`
- Slim bundle containing:
  - `manifest.json`
  - Storage files for the root document (under `storage/{prefix}/`)

This is the preferred endpoint for loading bundles in the bootloader.

### Legacy Endpoints

#### Get WASM Module

```http
GET /tonk_core_bg.wasm
```

Serves the Tonk Core WASM module for browser initialization.

#### Get Server Manifest

```http
GET /.manifest.tonk
```

Serves a slim bundle of the server's loaded bundle.

## Usage with Host-Web

The host-web bootloader can load bundles using short bundle IDs:

```
https://tonk.app/?b=abc123
```

The bootloader will resolve this to the full manifest URL:

```
http://localhost:8080/api/bundles/abc123/manifest
```

Configure the server URL in host-web's `.env`:

```bash
VITE_TONK_SERVER_URL=https://relay.tonk.xyz
```

## Development

The server runs with hot-reload in development mode:

```bash
pnpm dev
```

## Architecture

- **Express**: HTTP server and routing
- **WebSocket**: Real-time sync via `ws` library
- **Automerge**: CRDT-based document sync
- **Composite Storage**: Bundle (read-only) + Filesystem (persistent)
- **S3**: Bundle storage via AWS SDK
- **JSZip**: Bundle manipulation (zip files)
