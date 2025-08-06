// Simple script to receive file contents from Electron main process via IPC
import { configureSyncEngine, writeDoc, readDoc } from "@tonk/keepsync"
import { BrowserWebSocketClientAdapter } from "@automerge/automerge-repo-network-websocket";
import { parseBundle } from "@tonk/spec";

console.log('Load-in script initialized')

async function init() {
  const wsUrl = `http://localhost:7777/sync`;
  const wsAdapter = new BrowserWebSocketClientAdapter(wsUrl);

  const engine = await configureSyncEngine({
    url: `http://localhost:7777`,
    network: [wsAdapter],
  });
  await engine.whenReady();

  console.log("KeepSync Engine", engine);
  return engine;
}

// Listen for file data from main process
if (window.electronAPI && window.electronAPI.onFileReceived) {
  window.electronAPI.onFileReceived((fileData) => {
    console.log('File received via IPC:', fileData.fileName)
    console.log('File path:', fileData.filePath)
    console.log('File content length:', fileData.content ? fileData.content.length : 'undefined')
    
    // Handle the received file data
    handleFileReceived(fileData)
  })
} else {
  console.error('Electron API or onFileReceived listener not available')
}

// Function to handle received file data
async function handleFileReceived(fileData) {
  if (fileData.success) {
    try {
      // fileData.content is now a Uint8Array from IPC, convert to ArrayBuffer for parseBundle
      const arrayBuffer = fileData.content.buffer.slice(
        fileData.content.byteOffset, 
        fileData.content.byteOffset + fileData.content.byteLength
      )
      console.log('Parsing bundle from ArrayBuffer, size:', arrayBuffer.byteLength)
      const parsed = await parseBundle(arrayBuffer)
      console.log('Bundle parsed successfully:', parsed)

      parsed.manifest.files.forEach(async ({path}) => {
        const data = await parsed.getFileData(path);
        const fileMetadata = parsed.getFile(path);
        if (data) {
          const text = new globalThis.TextDecoder().decode(data);
          await writeDoc(path, {
            path: path,
            mimeType: fileMetadata.contentType,
            content: text
          });
          console.log(await readDoc(path));
        }
      })
      
    } catch (error) {
      console.error('Error parsing bundle:', error)
      window.currentFile = fileData
      window.currentBundle = null
    }
  } else {
    console.error('Error in received file:', fileData.error)
    window.currentFile = null
    window.currentBundle = null
  }
}

init();