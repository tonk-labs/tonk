import * as Automerge from "@automerge/automerge";
import { OutputFormat, SchemaMapping, DEFAULT_SCHEMA_MAPPING } from "./schema";

/**
 * Sets a nested property in an object using a path array
 * @param obj The object to modify
 * @param path Array of property names forming the path
 * @param value The value to set
 */
function setNestedProperty(obj: any, path: string[], value: any): void {
  let current = obj;
  
  // Navigate to the second-to-last element in the path
  for (let i = 0; i < path.length - 1; i++) {
    const key = path[i];
    if (!current[key]) {
      current[key] = {};
    }
    current = current[key];
  }
  
  // Set the value at the final path element
  const lastKey = path[path.length - 1];
  current[lastKey] = value;
}

/**
 * Gets a nested property from an object using a path array
 * @param obj The object to access
 * @param path Array of property names forming the path
 * @returns The value at the path or undefined if not found
 */
function getNestedProperty(obj: any, path: string[]): any {
  let current = obj;
  
  for (const key of path) {
    if (current === undefined || current === null) {
      return undefined;
    }
    current = current[key];
  }
  
  return current;
}

/**
 * Merges location data into an Automerge document using the specified schema mapping
 * @param doc The Automerge document to update
 * @param output The location data to merge
 * @param mapping Schema mapping configuration
 * @returns Updated Automerge document
 */
export function mergeLocationsIntoDoc(
  doc: Automerge.Doc<any>,
  output: OutputFormat,
  mapping: SchemaMapping = DEFAULT_SCHEMA_MAPPING
): Automerge.Doc<any> {
  return Automerge.change(doc, "Add imported locations", (draft) => {
    // Initialize document structure if needed
    const locationsContainer = ensurePath(draft, mapping.locationsPath);
    const userNamesContainer = ensurePath(draft, mapping.userNamesPath);
    
    // Add user names
    for (const [userId, userName] of Object.entries(output.userNames)) {
      const transformedUserName = mapping.transformUserName 
        ? mapping.transformUserName(userId, userName)
        : userName;
      
      userNamesContainer[userId] = transformedUserName;
    }
    
    // Add locations
    for (const [locationId, location] of Object.entries(output.locations)) {
      const transformedLocation = mapping.transformLocation 
        ? mapping.transformLocation(location)
        : location;
      
      locationsContainer[locationId] = transformedLocation;
    }
  });
}

/**
 * Ensures that a nested path exists in an object, creating it if necessary
 * @param obj The object to modify
 * @param path Array of property names forming the path
 * @returns The object at the end of the path
 */
function ensurePath(obj: any, path: string[]): any {
  let current = obj;
  
  for (const key of path) {
    if (!current[key]) {
      current[key] = {};
    }
    current = current[key];
  }
  
  return current;
}
