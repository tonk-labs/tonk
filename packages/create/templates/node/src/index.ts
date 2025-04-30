import { sync, configureSyncEngine, DocumentId } from "@tonk/keepsync";
import { BrowserWebSocketClientAdapter } from "@automerge/automerge-repo-network-websocket";
import { NodeFSStorageAdapter } from "@automerge/automerge-repo-storage-nodefs";
import { createStore } from "zustand/vanilla";

const wsAdapter = new BrowserWebSocketClientAdapter("ws://localhost:7777/sync");
configureSyncEngine({
  url: "http://localhost:7777",
  network: [wsAdapter as any],
  storage: new NodeFSStorageAdapter(),
});

interface CounterStore {
  count: number;
  increment: () => void;
  decrement: () => void;
}

const createStoreAndRun = () => {
  const store = createStore<CounterStore>(
    sync(
      (set) => ({
        count: 0,
        increment: () => set((state) => ({ count: state.count + 1 })),
        decrement: () => set((state) => ({ count: state.count - 1 })),
      }),
      {
        docId: "counter-doc" as DocumentId,
      }
    )
  );

  const state = store.getState();

  setInterval(() => {
    state.increment();
    console.log(`The current count is: ${store.getState().count}`);
  }, 2000);
};

createStoreAndRun();
