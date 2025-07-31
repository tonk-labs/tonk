// Update the Zustand store from Automerge document
export const patchStore = <T>(
  api: {
    setState: (fn: (state: T) => T) => void;
    getState: () => T;
  },
  docData: T
) => {
  const currentState = api.getState();

  // Use the new serialization comparison method
  if (areSerializedEqual(currentState, docData)) {
    return;
  }

  // Deserialize the document data to restore complex types (Date, Map, Set, etc.)
  const deserializedDocData = deserializeFromSync(docData);

  // Only update the state with properties from docData, preserving other state
  api.setState(state => {
    // Create a new state object to avoid mutating the current state
    const newState = { ...state };

    // Only copy properties that exist in deserializedDocData
    // This preserves any non-synced properties in the Zustand store
    Object.keys(deserializedDocData as object).forEach(key => {
      if (key in (deserializedDocData as object)) {
        (newState as any)[key] = (deserializedDocData as any)[key];
      }
    });

    return newState;
  });
};

/**
 * Import enhanced serialization utilities
 */
import {
  deserializeFromSync,
  areSerializedEqual,
} from '../utils/serialization.js';
