import { tool } from 'ai';
import { z } from 'zod';
import { getVFSService } from '../../../services/vfs-service';

export const tonkReadFileTool = tool({
  description: 'Read a file from the Tonk virtual file system.',
  inputSchema: z.object({
    path: z
      .string()
      .min(1, 'Path is required')
      .describe('Absolute Tonk path, e.g. /app/src/index.ts'),
  }),
  execute: async ({ path }) => {
    console.log('[TonkTool] readFile called:', { path });
    const vfs = getVFSService();
    if (!vfs.isInitialized()) {
      throw new Error('VFS service is not initialized');
    }

    try {
      const content = await vfs.readFile(path);
      console.log('[TonkTool] readFile success:', { path, contentLength: content.length });
      return {
        path,
        content,
        success: true,
      };
    } catch (error) {
      console.error('[TonkTool] readFile error:', { path, error });
      return {
        path,
        content: '',
        success: false,
        error: error instanceof Error ? error.message : 'Failed to read file',
      };
    }
  },
});

export const tonkWriteFileTool = tool({
  description:
    'Create or overwrite a file in the Tonk virtual file system with the supplied content.',
  inputSchema: z.object({
    path: z
      .string()
      .min(1, 'Path is required')
      .describe('Absolute Tonk path to write, e.g. /app/src/index.ts'),
    content: z.string().describe('Full file contents to persist.'),
  }),
  execute: async ({ path, content }) => {
    console.log('[TonkTool] writeFile called:', { path, contentLength: content.length });
    const vfs = getVFSService();
    if (!vfs.isInitialized()) {
      throw new Error('VFS service is not initialized');
    }

    try {
      const exists = await vfs.exists(path);
      await vfs.writeFile(path, content, !exists);
      console.log('[TonkTool] writeFile success:', { path, created: !exists });
      return {
        path,
        created: !exists,
        success: true,
      };
    } catch (error) {
      console.error('[TonkTool] writeFile error:', { path, error });
      return {
        path,
        created: false,
        success: false,
        error: error instanceof Error ? error.message : 'Failed to write file',
      };
    }
  },
});

export const tonkDeleteFileTool = tool({
  description: 'Delete a file from the Tonk virtual file system.',
  inputSchema: z.object({
    path: z
      .string()
      .min(1, 'Path is required')
      .describe('Absolute Tonk path to delete.'),
  }),
  execute: async ({ path }) => {
    console.log('[TonkTool] deleteFile called:', { path });
    const vfs = getVFSService();
    if (!vfs.isInitialized()) {
      throw new Error('VFS service is not initialized');
    }

    try {
      await vfs.deleteFile(path);
      console.log('[TonkTool] deleteFile success:', { path });
      return {
        path,
        success: true,
      };
    } catch (error) {
      console.error('[TonkTool] deleteFile error:', { path, error });
      return {
        path,
        success: false,
        error: error instanceof Error ? error.message : 'Failed to delete file',
      };
    }
  },
});

export const tonkListDirectoryTool = tool({
  description: 'List the entries in a Tonk directory.',
  inputSchema: z.object({
    path: z
      .string()
      .min(1, 'Path is required')
      .describe('Directory to list, e.g. /app/src'),
  }),
  execute: async ({ path }) => {
    console.log('[TonkTool] listDirectory called:', { path });
    const vfs = getVFSService();
    if (!vfs.isInitialized()) {
      throw new Error('VFS service is not initialized');
    }

    try {
      const entries = await vfs.listDirectory(path);
      console.log('[TonkTool] listDirectory success:', { path, entriesCount: entries.length });
      return {
        path,
        entries,
        success: true,
      };
    } catch (error) {
      console.error('[TonkTool] listDirectory error:', { path, error });
      return {
        path,
        entries: [],
        success: false,
        error: error instanceof Error ? error.message : 'Failed to list directory',
      };
    }
  },
});

export const tonkExistsTool = tool({
  description: 'Check whether a path exists in the Tonk virtual file system.',
  inputSchema: z.object({
    path: z
      .string()
      .min(1, 'Path is required')
      .describe('Absolute Tonk path to check.'),
  }),
  execute: async ({ path }) => {
    console.log('[TonkTool] exists called:', { path });
    const vfs = getVFSService();
    if (!vfs.isInitialized()) {
      throw new Error('VFS service is not initialized');
    }

    try {
      const exists = await vfs.exists(path);
      console.log('[TonkTool] exists success:', { path, exists });
      return {
        path,
        exists,
        success: true,
      };
    } catch (error) {
      console.error('[TonkTool] exists error:', { path, error });
      return {
        path,
        exists: false,
        success: false,
        error: error instanceof Error ? error.message : 'Failed to check existence',
      };
    }
  },
});

// Finish tool - signals that the agent has completed its task
export const finishTool = tool({
  description: 'Call this tool when you have completed the requested task. This signals that you are done with your work.',
  inputSchema: z.object({
    summary: z
      .string()
      .describe('A brief summary of what was accomplished'),
    files_modified: z
      .array(z.string())
      .optional()
      .describe('List of file paths that were modified or created'),
  }),
  execute: async ({ summary, files_modified }) => {
    console.log('[TonkTool] FINISH called:', { summary, files_modified });
    return {
      completed: true,
      summary,
      files_modified: files_modified || [],
      timestamp: new Date().toISOString(),
    };
  },
});

export const tonkTools = {
  tonkReadFile: tonkReadFileTool,
  tonkWriteFile: tonkWriteFileTool,
  tonkDeleteFile: tonkDeleteFileTool,
  tonkListDirectory: tonkListDirectoryTool,
  tonkExists: tonkExistsTool,
  finish: finishTool,
};