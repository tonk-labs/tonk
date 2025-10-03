# Tonk Overview

Welcome to Tonk!

Thank you for taking the time to explore our documentation. We're incredibly grateful that you're
here building the future with us.

If you have any questions, run into issues, or just want to chat about Tonk, please don't hesitate
to reach out to us in our [Discord community](https://discord.com/invite/cHqkYpRE). We'd love to
hear from you and help however we can!

## What is Tonk?

Tonk is a new category of software that creates portable, multiplayer, user-owned digital artifacts.
It's essentially a file that contains both an application and its data, designed to work anywhere,
last forever, and remain under user control.

## Product Philosophy

Tonk represents a "credibly neutral platform" for building and sharing local software - positioned
as an alternative to industrial-scale complexity. It embodies principles of:

- Malleable software - as easy to change as it is to use
- User ownership - people retain control of their tools and data
- Infinite software in the age of AI - human-scale creation without corporate intermediaries

## Core Tech Goals

- A tonk has instructions for how to connect to its peers.
- A tonk works offline.
- A tonk can theoretically run on any device and be served as a web application.
- A tonk can be shared just like a file.
- A tonk is encrypted and only accessed by its group. (coming soon)
- A tonk will synchronize the state of its filesystem with any connected peer.
- A tonk can be forked or remixed by changing its network or membership group and updating its
  state.

## Key Features

### Virtual File System (VFS)

A document-based storage system backed by Automerge CRDTs that provides:

- Real-time synchronization across peers
- File and directory operations
- Watch capabilities for reactive updates
- Binary and text file support

### Bundle Format

Self-contained application packages (.tonk files) that include:

- Application code and assets
- Serialized state documents
- Metadata and configuration
- Network sync endpoints

### Host-Web Runtime Environment

Complete browser runtime for loading and executing .tonk applications:

- **Drag-and-Drop Loading**: Simply drag .tonk files onto the interface
- **Service Worker Architecture**: Intercepts requests and serves content from VFS
- **Multi-Bundle Support**: Load and run multiple applications simultaneously
- **Offline-First Operation**: Applications work without network connectivity
- **URL-based Access**: Applications accessible at `localhost:3000/${project-name}/`
- **Automatic Sync**: Connects to relay servers specified in bundle manifest

### WASM Core

High-performance Rust implementation compiled to WebAssembly:

- Runs in browsers and Node.js
- Consistent behavior across platforms
- Safe memory management
- Efficient CRDT operations

### Real-time Object-level Sync

Automatic synchronization of JSON-like objects powered by Automerge:

- WebSocket-based relay transport
- Peer discovery and room-based connection
- Conflict-free merge semantics
- Offline queue and replay

## Use Cases

### Living Business Experiments

Make your excel spreadsheet interactive then share it with a client.

### Multiplayer Digital Gardens

Shared wikis, blogs, or notebooks that feel cosy like a private chat.

### Canvas Jam

Put TLDR draw in a tonk!

### Collaborative Research Notebooks

Put a Jupyter notebook inside your tonk!

## Next Steps

- [Quickstart Guide](./quickstart.md) - Get up and running quickly
- [Architecture](./architecture.md) - Deep dive into Tonk's design
- [Virtual File System](./vfs.md) - Learn about the VFS layer
- [Bundle Format](./bundles.md) - Understand bundle packaging
