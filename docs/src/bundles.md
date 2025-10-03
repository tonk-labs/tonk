# Bundle Format

Bundles are self-contained packages that encapsulate Tonk applications, their data, and metadata
into a single distributable file with the `.tonk` extension.

## Overview

A bundle is essentially a ZIP archive containing:

- Application code and assets as serialized automerge documents
- Manifest with metadata and configuration

## Bundle Structure

````
my-app.tonk (ZIP archive)
├── manifest.json           # Bundle metadata and configuration
├── storage/               # Serialized Automerge documents
    ├── root.automerge    # Root document (always present)
    ├── [doc-id-1].automerge
    ├── [doc-id-2].automerge
    └── ...

## Manifest Format

The `manifest.json` file contains essential metadata about the bundle:

```json
{
  "manifest_version": 1,
  "version": {
    "major": 1,
    "minor": 0
  },
  "root_id": "automerge:3qF8x9...",
  "entrypoints": ["index.html"],
  "network_uris": ["wss://sync.example.com"],
  "x_notes": "Optional human-readable notes",
  "x_vendor": {
    "app_name": "My Tonk App",
    "author": "John Doe",
    "custom_field": "any value"
  }
}
````

### Manifest Fields

| Field              | Type     | Required | Description                             |
| ------------------ | -------- | -------- | --------------------------------------- |
| `manifest_version` | number   | Yes      | Bundle format version (currently 1)     |
| `version`          | object   | Yes      | Bundle version with major/minor         |
| `root_id`          | string   | Yes      | Automerge document ID of root directory |
| `entrypoints`      | string[] | No       | Entry files for the application         |
| `network_uris`     | string[] | No       | WebSocket URIs for sync                 |
| `x_notes`          | string   | No       | Human-readable notes                    |
| `x_vendor`         | object   | No       | Custom vendor-specific metadata         |
