import superjson from 'superjson';

/**
 * Recursively removes functions from an object while preserving other complex types
 * This is used as a preprocessing step before SuperJSON serialization
 *
 * @param obj The object to process
 * @param visited Set to track visited objects for circular reference detection
 * @returns A new object with functions removed but other types preserved
 */
function removeFunctions<T>(obj: T, visited = new WeakSet()): T {
  if (!obj || typeof obj !== 'object') {
    return obj;
  }

  // Handle Date, RegExp, and other built-in objects - pass through as-is
  // These are immutable and safe to reuse
  if (obj instanceof Date || obj instanceof RegExp || obj instanceof Error) {
    return obj;
  }

  // Handle circular references for mutable objects only
  if (visited.has(obj as any)) {
    return obj; // Return the original object for circular references (let SuperJSON handle it)
  }
  visited.add(obj as any);

  // Handle arrays
  if (Array.isArray(obj)) {
    return obj
      .filter(item => typeof item !== 'function') // Filter out functions from arrays
      .map(item => removeFunctions(item, visited)) as T;
  }

  // Handle Maps
  if (obj instanceof Map) {
    const newMap = new Map();
    for (const [key, value] of obj.entries()) {
      // Only add non-function values
      if (typeof value !== 'function') {
        newMap.set(key, removeFunctions(value, visited));
      }
    }
    return newMap as T;
  }

  // Handle Sets
  if (obj instanceof Set) {
    const newSet = new Set();
    for (const value of obj.values()) {
      // Only add non-function values
      if (typeof value !== 'function') {
        newSet.add(removeFunctions(value, visited));
      }
    }
    return newSet as T;
  }

  // Handle plain objects
  const result: any = {};
  for (const key in obj) {
    if (obj.hasOwnProperty(key)) {
      const value = obj[key];
      // Skip functions
      if (typeof value === 'function') {
        continue;
      }
      // Recursively process other values
      result[key] = removeFunctions(value, visited);
    }
  }

  return result as T;
}

/**
 * Serializes an object using SuperJSON while filtering out functions
 * This preserves Date objects, Maps, Sets, RegExp, BigInt, etc. but removes functions
 *
 * @param obj The object to serialize
 * @returns Serialized object that can be stored in Automerge
 */
export function serializeForSync<T>(obj: T): any {
  if (!obj || typeof obj !== 'object') {
    return obj;
  }

  try {
    // First remove functions, then serialize with SuperJSON
    const functionsRemoved = removeFunctions(obj);
    const serialized = superjson.serialize(functionsRemoved);

    // If there's no metadata (simple objects), just return the json part
    if (!serialized.meta || Object.keys(serialized.meta).length === 0) {
      return serialized.json;
    }

    // Return the full serialized data (SuperJSON returns {json, meta})
    // We store both parts so we can deserialize properly
    return serialized;
  } catch (error) {
    // Fallback to basic function removal if SuperJSON fails
    console.warn(
      'SuperJSON serialization failed, falling back to basic serialization:',
      error
    );
    return removeFunctions(obj);
  }
}

/**
 * Deserializes an object that was serialized with serializeForSync
 * This restores Date objects, Maps, Sets, RegExp, BigInt, etc.
 *
 * @param serializedObj The serialized object from Automerge
 * @returns Deserialized object with complex types restored
 */
export function deserializeFromSync<T>(serializedObj: any): T {
  if (!serializedObj || typeof serializedObj !== 'object') {
    return serializedObj;
  }

  try {
    // Check if this looks like a SuperJSON serialized object
    if (serializedObj.json !== undefined && serializedObj.meta !== undefined) {
      // This is a SuperJSON serialized object, deserialize it
      return superjson.deserialize(serializedObj);
    } else {
      // This might be legacy data or basic object, return as-is
      return serializedObj;
    }
  } catch (error) {
    // If deserialization fails, return the object as-is
    console.warn(
      'SuperJSON deserialization failed, returning object as-is:',
      error
    );
    return serializedObj;
  }
}

/**
 * Checks if two objects are equal after serialization
 * This is used to determine if the store needs to be updated
 *
 * @param obj1 First object to compare
 * @param obj2 Second object to compare
 * @returns true if the objects are equal after serialization
 */
export function areSerializedEqual<T>(obj1: T, obj2: T): boolean {
  try {
    const serialized1 = serializeForSync(obj1);
    const serialized2 = serializeForSync(obj2);

    // Compare the JSON representations
    return JSON.stringify(serialized1) === JSON.stringify(serialized2);
  } catch (error) {
    // Fallback to basic JSON comparison
    console.warn(
      'Serialized comparison failed, falling back to JSON comparison:',
      error
    );
    return JSON.stringify(obj1) === JSON.stringify(obj2);
  }
}
