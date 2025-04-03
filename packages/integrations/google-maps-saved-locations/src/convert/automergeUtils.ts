import * as fs from "fs/promises";
import * as Automerge from "@automerge/automerge";

/**
 * Loads an Automerge document from a file, or creates a new one if the file doesn't exist
 * @param filePath Path to the Automerge binary file
 * @returns Loaded Automerge document
 */
export async function loadAutomergeDoc(filePath: string): Promise<Automerge.Doc<any>> {
  try {
    // Check if the file exists
    await fs.access(filePath);

    // Read the binary file
    const binary = await fs.readFile(filePath);

    // Load the document from binary
    return Automerge.load(binary);
  } catch (error) {
    // If file doesn't exist or can't be loaded, create a new document
    console.log(`Creating new Automerge document (${error})`);
    return Automerge.init();
  }
}

/**
 * Saves an Automerge document to a file
 * @param doc The Automerge document to save
 * @param filePath Path where the document should be saved
 */
export async function saveAutomergeDoc(
  doc: Automerge.Doc<any>,
  filePath: string,
): Promise<void> {
  // Convert the document to binary
  const binary = Automerge.save(doc);

  // Write the binary to file
  await fs.writeFile(filePath, binary);
  console.log(`Saved Automerge document to ${filePath}`);
}
