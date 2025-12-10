import { getVFSService } from '@/vfs-client';

interface SampleFile {
  path: string;
  content: string;
  desktopMeta: {
    x: number;
    y: number;
  };
}

const sampleFiles: SampleFile[] = [
  {
    path: '/desktonk/Welcome.txt',
    content: `Welcome to Desktonk!

This is a collaborative desktop environment built on top of Tonk.

Features:
- Real-time file synchronization
- Multiple file type support
- Drag-and-drop desktop interface
- Built-in text and image editors
- Collaborative editing with presence awareness

Try opening the other sample files to explore more!`,
    desktopMeta: {
      x: 50,
      y: 50,
    },
  },
  {
    path: '/desktonk/README.md',
    content: `# Desktonk Desktop Environment

## Overview

Desktonk is a modern, collaborative desktop environment that runs entirely in your browser. It leverages the Tonk real-time synchronization framework to provide seamless multi-user experiences.

## Key Features

### File Management
- Create, edit, and delete files
- Support for multiple file formats (.txt, .md, .json, images)
- Desktop-style icon arrangement with drag-and-drop

### Collaboration
- Real-time presence tracking
- See who's online and what they're working on
- Integrated chat system

### Editors
- Rich text editor with TipTap
- Markdown support
- Image viewer
- JSON editor

## Getting Started

1. Create new files by right-clicking on the desktop
2. Double-click files to open them
3. Drag files to rearrange them on your desktop
4. Use the chat button to communicate with other users

Enjoy exploring Desktonk!`,
    desktopMeta: {
      x: 170,
      y: 50,
    },
  },
  {
    path: '/desktonk/notes.json',
    content: JSON.stringify(
      {
        title: 'Sample Notes',
        version: '1.0.0',
        lastModified: new Date().toISOString(),
        items: [
          {
            id: 1,
            text: 'Welcome to the JSON editor!',
            completed: false,
            priority: 'high',
          },
          {
            id: 2,
            text: 'Try editing this file to see real-time sync',
            completed: false,
            priority: 'medium',
          },
          {
            id: 3,
            text: 'Changes are automatically saved',
            completed: true,
            priority: 'low',
          },
        ],
        metadata: {
          created: new Date().toISOString(),
          tags: ['sample', 'demo', 'desktonk'],
          author: 'Desktonk',
        },
      },
      null,
      2
    ),
    desktopMeta: {
      x: 290,
      y: 50,
    },
  },
];

async function waitForVFSInitialization(maxAttempts = 50, intervalMs = 100): Promise<void> {
  const vfs = getVFSService();

  for (let attempt = 0; attempt < maxAttempts; attempt++) {
    if (vfs.isInitialized()) {
      console.log('[CreateSamples] VFS is initialized');
      return;
    }

    console.log(
      `[CreateSamples] Waiting for VFS initialization (attempt ${attempt + 1}/${maxAttempts})...`
    );
    await new Promise((resolve) => setTimeout(resolve, intervalMs));
  }

  throw new Error('VFS initialization timeout - make sure the VFS service is running');
}

export async function createSampleFiles(): Promise<void> {
  console.log('[CreateSamples] Starting sample files creation...');

  try {
    // Wait for VFS to be initialized
    await waitForVFSInitialization();

    const vfs = getVFSService();

    // Ensure the /desktonk directory exists by checking for it
    const desktopExists = await vfs.exists('/desktonk');
    if (!desktopExists) {
      console.warn(
        '[CreateSamples] /desktonk directory does not exist - files will be created anyway'
      );
    }

    // Create each sample file
    for (const file of sampleFiles) {
      console.log(`[CreateSamples] Creating file: ${file.path}`);

      try {
        // Write file with content and metadata
        await vfs.writeStringAsBytes(file.path, file.content, true);

        // Update metadata separately to preserve desktop position
        const existingDoc = await vfs.readFile(file.path);
        const updatedContent = {
          ...existingDoc.content,
          desktopMeta: file.desktopMeta,
        };

        await vfs.writeFile(
          file.path,
          {
            content: updatedContent,
            bytes: existingDoc.bytes,
          },
          false
        );

        console.log(
          `[CreateSamples] Successfully created: ${file.path} at position (${file.desktopMeta.x}, ${file.desktopMeta.y})`
        );
      } catch (error) {
        console.error(`[CreateSamples] Failed to create ${file.path}:`, error);
        throw error;
      }
    }

    console.log(`[CreateSamples] Successfully created ${sampleFiles.length} sample files!`);
    console.log('[CreateSamples] Sample files:');
    for (const file of sampleFiles) {
      console.log(`  - ${file.path} (${file.desktopMeta.x}, ${file.desktopMeta.y})`);
    }
  } catch (error) {
    console.error('[CreateSamples] Error creating sample files:', error);
    throw error;
  }
}

// Make function available globally in browser console
if (typeof window !== 'undefined') {
  // biome-ignore lint/suspicious/noExplicitAny: Exposing utility function to window for debugging
  (window as any).createSampleFiles = createSampleFiles;
  console.log(
    '[CreateSamples] Sample files utility loaded. Run createSampleFiles() to populate desktop.'
  );
}
