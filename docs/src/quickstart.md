# Quickstart guide

> If you haven't yet, start with the [**introduction**](./introduction.md) before reading this quickstart guide.

Tonk apps plug into Tonk stores, which store data in a local-first way. This makes applications especially collaborative and interoperable. It also means that they are private, performant, have offline support and reduce dependency on third parties. Apps on Tonk sidestep traditional database headaches such as caching, migrations and auth.

## Installing Tonk

First, you'll need to install Tonk on your machine:

```bash
npm install -g @tonk/cli && tonk hello
```

This will install the Tonk CLI globally and run the `hello` command, which sets up the Tonk daemon for synchronizing your data.

## Creating a new Tonk app

To create a new Tonk app, use the `create` command:

```bash
tonk create
```

The CLI will guide you through a series of prompts to configure your project:

1. Choose a template (React or Node.js)
2. Enter a project name (or accept the default)
3. Provide a brief description of your project

After answering these questions, Tonk will scaffold a new project with everything you need to get started, including:

- React, TypeScript, and Tailwind CSS
- Keepsync for state management and synchronization
- Development tools and configuration

## Developing your app

Navigate to your project directory and start development:

```bash
cd my-app
pnpm dev
```

This will:

1. Start a development server with hot reloading
2. Set up a sync server for real-time collaboration
3. Open your app in the browser (typically at http://localhost:3000)

## Understanding Tonk Stores

Stores are what make Tonk apps special. They're containers of shareable data that easily plug into any app. Stores are synchronized using the Automerge CRDT library, which enables automatic conflict resolution when multiple users edit the same data.

Unlike traditional databases, Tonk stores:

- Work offline first, then sync when connections are available
- Don't require schema migrations
- Handle synchronization automatically
- Provide real-time updates across all connected clients

### Working with stores in your app

Your Tonk app comes with the `@tonk/keepsync` library already integrated. Here's how to use it:

```typescript
// 1. Import the sync middleware
import { create } from "zustand";
import { sync } from "@tonk/keepsync";

// 2. Create a synced store
const useTodoStore = create(
  sync(
    (set) => ({
      todos: [],
      addTodo: (text) =>
        set((state) => ({
          todos: [...state.todos, { id: Date.now(), text, completed: false }],
        })),
      toggleTodo: (id) =>
        set((state) => ({
          todos: state.todos.map((todo) =>
            todo.id === id ? { ...todo, completed: !todo.completed } : todo
          ),
        })),
    }),
    { docId: "todos" } // This identifies the store for synchronization
  )
);
```

When multiple users access your app, any changes they make to the store will automatically synchronize across all clients in real-time.

## Deploying your app

Once your app is ready for deployment, you can build and serve it:

```bash
# Build the app
pnpm run build

# Push the bundle you've just built
tonk push

```

# Start a service to host your bundle

`tonk start <bundleName>`

The bundleName will likely be the folder of your project. You can see available bundles by running `tonk ls`

```

Usage: tonk start [options] <bundleName>

Start a bundle server

Arguments:
bundleName Name of the bundle to start

Options:
-u, --url <url> URL of the Tonk server (default: "http://localhost:7777")
-p, --port <port> Port for the bundle server (optional)

```

You should see a message like:

```

Bundle server started successfully!
Server ID: 454f91d5-40a9-4892-aca8-f6cfaa3936a5
Running on port: 8000
Status: running
Use 'tonk ps' to see all running servers

```

# Use a reverse-proxy to share your application to external devices

Pinggy makes this simple and free

```

ssh -p 443 -R0:localhost:8000 free.pinggy.io

```

The caveat is of course that free pinggy tunnels last only 60 minutes. If you pay for a dedicated tunnel, then you could serve the application off that.

## Tonk Deploy

For comprehensive deployment options including Docker containerization and cloud deployment, see the [Deployment Guide](./deployment.md). For CLI command reference, see the [Reference](./reference.md) section.
