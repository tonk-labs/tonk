import { initializeWasm, configureSyncEngine, getRepo, readDoc } from "@tonk/keepsync/slim"

import {
  MessageChannelNetworkAdapter,
  WebSocketClientAdapter,
  IndexedDBStorageAdapter,
} from "https://esm.sh/@automerge/vanillajs@2.2.0/slim?bundle-deps"

const CACHE_NAME = "v6"

async function initializeRepo() {
  console.log("Waiting for Automerge WASM initialization...")

  await initializeWasm(fetch("https://esm.sh/@automerge/automerge@3.1.1/dist/automerge.wasm"))
  console.log("Automerge WASM initialized successfully")

  console.log("Creating sync engine")
  const syncEngine = await configureSyncEngine({
    url: "http://localhost:7777",
    storage: new IndexedDBStorageAdapter(),
    network: [new WebSocketClientAdapter("ws://localhost:7777/sync")],
  })

  await syncEngine.whenReady()
  const repo = getRepo()
  self.repo = repo

  return repo
}

console.log("Before registration")
const repoPromise = initializeRepo()
repoPromise.then((r) => {
  self.repo = r
  console.log("Keepsync service worker repo initialized")
})

async function clearOldCaches() {
  const cacheWhitelist = [CACHE_NAME]
  const cacheNames = await caches.keys()
  const deletePromises = cacheNames.map((cacheName) => {
    if (!cacheWhitelist.includes(cacheName)) {
      return caches.delete(cacheName)
    }
  })
  await Promise.all(deletePromises)
}

self.addEventListener("install", (event) => {
  console.log("Installing SW")
  self.skipWaiting()
})

self.addEventListener("message", async (event) => {
  console.log("Client messaged", event.data)

  if (event.data && event.data.type === "INIT_PORT") {
    const clientPort = event.ports[0]
    const repo = getRepo()
    if (repo) {
      repo.networkSubsystem.addNetworkAdapter(
        new MessageChannelNetworkAdapter(clientPort, { useWeakRef: true })
      )
    }
  }

  if (event.data && event.data.type === "TEST_AUTOMERGE") {
    console.log("Received TEST_AUTOMERGE message")

    const respond = (data) => {
      try {
        event.ports[0].postMessage(data)
      } catch (e) {
        console.error("Failed to post message back:", e)
      }
    }

    try {
      console.log("Testing automerge repo in service worker...")

      await repoPromise
      const repo = getRepo()

      if (!repo) {
        throw new Error("Repo not initialized")
      }

      console.log("Repo available, testing document operations...")

      const handle = repo.create()
      handle.change((doc) => {
        doc.text = "Hello from service worker!"
        doc.timestamp = Date.now()
      })

      const docState = handle.docSync()

      respond({
        success: true,
        message: "Automerge repo works in service worker!",
        docId: handle.documentId,
        docContent: docState,
      })
    } catch (error) {
      console.error("Error testing automerge in service worker:", error)
      respond({
        success: false,
        error: error.message,
        stack: error.stack,
      })
    }
  }
})

function addSyncServer(url) {
  const repo = getRepo()
  if (repo) {
    repo.networkSubsystem.addNetworkAdapter(new WebSocketClientAdapter(url))
  }
}
self.addSyncServer = addSyncServer

self.addEventListener("activate", async (event) => {
  console.log("Activating service worker.")
  await clearOldCaches()
  clients.claim()
})

const determinePath = (url) => {
  const serviceWorkerPath = self.location.pathname // Path where the SW is registered
  const registrationScope = serviceWorkerPath.split("/").slice(0, -1).join("/") + "/" // Base scope

  const requestPath = new URL(url).pathname

  // Get the path relative to the service worker's registration scope
  const relativePath = requestPath.startsWith(registrationScope)
    ? requestPath.slice(registrationScope.length)
    : requestPath

  // Special case for expected web server behavior
  const candidatePath = relativePath.split("/")
  if (candidatePath[candidatePath.length - 1] === "") {
    candidatePath[candidatePath.length - 1] = "index.html"
  }

  return candidatePath
}

const targetToResponse = async (target) => {
  if (target.mimeType) {
    return new Response(target.content, {
      headers: { "Content-Type": target.contentType },
    })
  }

  if (typeof target === "string") {
    return new Response(target, {
      headers: { "Content-Type": "text/plain" },
    })
  }

  return new Response(JSON.stringify(target), {
    headers: { "Content-Type": "application/json" },
  })
}

self.addEventListener("fetch", async (event) => {
  const url = new URL(event.request.url)

  if (url.origin === location.origin) {
    event.respondWith(
      (async () => {
        let pathname = url.pathname;
        console.log(pathname);
        if (pathname === "/") {
          pathname = "/index.html";
        }
        const target = await readDoc(pathname);

        if (!target) {
          return new Response(`The path couldn't be resolved to a valid document.`, {
            status: 500,
            headers: { "Content-Type": "text/plain" },
          })
        }

        return targetToResponse(target)
      })()
    )
  }
})
