import { configureSyncEngine } from "@tonk/keepsync";
import { createSyncedStore } from "@tonk/keepsync";

// Define the store type
interface CounterStore {
  count: number;
  increment: () => void;
  decrement: () => void;
}

// Optional: Configure the sync engine once at application startup
// This can be done in a separate initialization file
export function initializeSync() {
  configureSyncEngine({
    url: "ws://localhost:4080/sync",
    name: "MyApplication",
    dbName: "test_counter_example_db",
    onSync: (docId) => console.log(`Document ${docId} synced`),
    onError: (error) => console.error("Sync error:", error),
  });
}

// Create the counter store
export const createCounterStore = async () => {
  // Create a synced store with minimal configuration
  return createSyncedStore<CounterStore>(
    {
      docId: "counter-doc",
      initialState: { count: 0, increment: () => {}, decrement: () => {} },
    },
    (set) => ({
      count: 0,
      increment: () => set((state) => ({ count: state.count + 1 })),
      decrement: () => set((state) => ({ count: state.count - 1 })),
    }),
  );
};

// Usage example:
// In your application entry point:
// initializeSync();
//
// In your component:
// const useCounterStore = await createCounterStore();
// function MyComponent() {
//   const { count, increment, decrement } = useCounterStore();
//   return (
//     <div>
//       <p>Count: {count}</p>
//       <button onClick={increment}>+</button>
//       <button onClick={decrement}>-</button>
//     </div>
//   );
// }
