// import * as Automerge from '@automerge/automerge';
// import {logger} from '../utils/logger';

// Patch the Automerge document from Zustand state
// export function patchAutomergeDoc<T>(
//   doc: Automerge.Doc<T>,
//   state: T,
// ): Automerge.Doc<T> {
//   try {
//     // Remove functions and other non-serializable data before storing in Automerge
//     const serializableState = removeNonSerializable(state);

//     return Automerge.change(doc, (d: any) => {
//       // Clear existing properties first
//       Object.keys(d).forEach(key => {
//         // Don't delete internal Automerge properties
//         if (!key.startsWith('_')) {
//           delete d[key];
//         }
//       });

//       // Copy all properties from state
//       Object.assign(d, serializableState);
//     });
//   } catch (error) {
//     logger.error('Error patching Automerge document:', error);
//     throw error;
//   }
// }

// Update the Zustand store from Automerge document
export const patchStore = <T>(
  api: {
    setState: (fn: (state: T) => T) => void;
    getState: () => T;
  },
  docData: T,
) => {
  const currentState = api.getState();
  const currentStateJson = JSON.stringify(removeNonSerializable(currentState));
  const docDataJson = JSON.stringify(removeNonSerializable(docData));

  // If the serialized states are identical, no need to update
  if (currentStateJson === docDataJson) {
    return;
  }

  // Only update the state with properties from docData, preserving other state
  api.setState(state => {
    // Create a new state object to avoid mutating the current state
    const newState = {...state};

    // Only copy properties that exist in docData
    // This preserves any non-synced properties in the Zustand store
    Object.keys(docData as object).forEach(key => {
      if (key in (docData as object)) {
        (newState as any)[key] = (docData as any)[key];
      }
    });

    return newState;
  });
};

/**
 * Removes functions and other non-serializable data from an object
 * @param obj The object to process
 * @returns A new object with only serializable data
 */
export function removeNonSerializable<T>(obj: T): Partial<T> {
  if (!obj || typeof obj !== 'object') {
    return obj as any;
  }

  const result: Partial<T> = Array.isArray(obj)
    ? ([] as any)
    : ({} as Partial<T>);

  for (const key in obj) {
    const value = obj[key];
    // Skip functions
    if (typeof value === 'function') {
      continue;
    }
    // Recursively process objects
    else if (value && typeof value === 'object') {
      result[key] = removeNonSerializable(value) as any;
    }
    // Keep primitive values
    else {
      result[key] = value;
    }
  }

  return result;
}
