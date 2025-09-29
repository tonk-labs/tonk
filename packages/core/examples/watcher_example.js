// Example usage of the document watcher API

async function main() {
  // Initialize TonkCore
  const { create_tonk } = await import('../pkg-node/tonk_core.js');
  const tonk = await create_tonk();

  // Get VFS instance
  const vfs = await tonk.getVfs();

  // Watch the root directory for changes
  console.log('Starting document watcher...');
  const watcher = await vfs.watchDirectory('/', docState => {
    console.log('Document changed:', docState);
  });

  // Create a test document
  await vfs.createFile('/test.txt', 'Initial content');

  // Get the document ID being watched
  try {
    const docId = watcher.documentId();
    console.log('Watching document ID:', docId);
  } catch (error) {
    console.error('Error getting document ID:', error);
  }

  // Make a change to the document (in a real app, this might come from another client)
  vfs.updateFile('/test.txt', 'New content!');

  // Stop watching after 5 seconds
  setTimeout(async () => {
    console.log('Stopping watcher...');
    await watcher.stop();
    console.log('Watcher stopped');
    process.exit(0);
  }, 5000);

  // Keep the process alive
  console.log('Waiting for 5 seconds before stopping...');
}

// Directory watcher example
async function directoryExample() {
  const { create_tonk } = await import('../pkg-node/tonk_core.js');
  const tonk = await create_tonk();
  const vfs = await tonk.getVfs();

  // Create a directory
  await vfs.createDirectory('/mydir');

  // Watch the directory
  const watcher = await vfs.watchDirectory('/mydir', dirState => {
    console.log('Directory metadata changed:', dirState);
  });

  // Stop after some time
  setTimeout(async () => {
    await watcher.stop();
  }, 5000);
}

// Run the example
if (typeof window === 'undefined') {
  main().catch(console.error);
}
