import { z } from 'zod';
import path from 'path';
import { configureSyncEngine, ls, mkDir, readDoc, writeDoc } from '@tonk/keepsync';
import { NetworkAdapterInterface } from "@automerge/automerge-repo";
import { BrowserWebSocketClientAdapter } from "@automerge/automerge-repo-network-websocket";
import { NodeFSStorageAdapter } from "@automerge/automerge-repo-storage-nodefs";

interface MCPTool {
  name: string;
  description: string;
  inputSchema: z.ZodSchema;
  handler: (args: any) => Promise<{ content: Array<{ type: 'text'; text: string }> }>;
}

export class WidgetMCPServer {
  private tools: Map<string, MCPTool> = new Map();
  private readonly widgetsBasePath = '/widgets';

  constructor() {
    this.setupTools();
  }

  private addTool(tool: MCPTool) {
    this.tools.set(tool.name, tool);
  }

  private setupTools() {
    // Tool to write widget files
    this.addTool({
      name: 'write_widget_file',
      description: 'Write a file to the widgets directory',
      inputSchema: z.object({
        path: z.string().describe('Relative path within widgets directory'),
        content: z.string().describe('File content to write'),
        encoding: z.enum(['utf8', 'base64']).default('utf8').describe('File encoding'),
      }),
      handler: async (args: { path: string; content: string; encoding: 'utf8' | 'base64' }) => {
        const { path: filePath, content, encoding } = args;

        // Security: Normalize and validate path
        const normalizedPath = filePath.startsWith('/') ? filePath.slice(1) : filePath;
        const fullPath = `${this.widgetsBasePath}/${normalizedPath}`;

        try {
          // Write file
          // await fs.writeFile(absolutePath, content, encoding as BufferEncoding);
          writeDoc(`/widgets/${normalizedPath}`, {
            content,
            encoding,
            timestamp: Date.now(),
          });

          // Get file stats for response
          // const stats = await fs.stat(absolutePath);

          return {
            content: [{
              type: 'text' as const,
              text: JSON.stringify({
                path: filePath,
                fullPath,
                size: content.length,
                message: `Successfully wrote ${content.length} bytes to ${filePath}`
              })
            }]
          };
        } catch (error) {
          throw new Error(`Failed to write file: ${error instanceof Error ? error.message : 'Unknown error'}`);
        }
      }
    });

    // Tool to read widget files
    this.addTool({
      name: 'read_widget_file',
      description: 'Read a file from the widgets directory',
      inputSchema: z.object({
        path: z.string().describe('Relative path within widgets directory'),
        encoding: z.enum(['utf8', 'base64']).default('utf8').describe('File encoding'),
      }),
      handler: async (args: { path: string; encoding: 'utf8' | 'base64' }) => {
        const { path: filePath } = args;

        // Security: Normalize and validate path
        const normalizedPath = filePath.startsWith('/') ? filePath.slice(1) : filePath;
        const fullPath = `${this.widgetsBasePath}/${normalizedPath}`;

        try {
          // const content = await fs.readFile(absolutePath, encoding as BufferEncoding);
          const doc = await readDoc<{ content: string; encoding: string; timestamp: number }>(fullPath);
          if (!doc) throw new Error(`File not found: ${filePath}`);
          // const stats = await fs.stat(absolutePath);

          return {
            content: [{
              type: 'text' as const,
              text: JSON.stringify({
                path: filePath,
                content: doc.content,
                size: doc.content.length,
                modified: new Date(doc.timestamp || Date.now()).toISOString()
              })
            }]
          };
        } catch (error) {
          throw new Error(`Failed to read file: ${error instanceof Error ? error.message : 'Unknown error'}`);
        }
      }
    });

    // Tool to list widget directory contents
    this.addTool({
      name: 'list_widget_directory',
      description: 'List contents of a directory within widgets',
      inputSchema: z.object({
        path: z.string().default('').describe('Relative path within widgets directory (empty for root)'),
      }),
      handler: async (args: { path: string }) => {
        const { path: dirPath } = args;

        // Security: Normalize and validate path
        const normalizedPath = (dirPath.startsWith('/') ? dirPath.slice(1) : dirPath) || '';
        const fullPath = normalizedPath ? `${this.widgetsBasePath}/${normalizedPath}` : this.widgetsBasePath;

        try {
          // const entries = await fs.readdir(absolutePath, { withFileTypes: true });
          const docNode = await ls(fullPath);

          if (!docNode) throw new Error(`Directory not found: ${normalizedPath}`);

          const items = (docNode.children || []).map(child => ({
            name: child.name,
            type: child.type === 'dir' ? 'directory' : 'file',
            path: path.join(dirPath, child.name)
          }));

          return {
            content: [{
              type: 'text' as const,
              text: JSON.stringify({
                path: dirPath,
                items
              })
            }]
          };
        } catch (error) {
          throw new Error(`Failed to list directory: ${error instanceof Error ? error.message : 'Unknown error'}`);
        }
      }
    });
  }

  async executeTool(name: string, args: any): Promise<{ content: Array<{ type: 'text'; text: string }> }> {
    const tool = this.tools.get(name);
    if (!tool) {
      throw new Error(`Tool ${name} not found`);
    }

    // Validate input
    const validatedArgs = tool.inputSchema.parse(args);

    // Execute tool
    return await tool.handler(validatedArgs);
  }

  getTools(): Record<string, { description: string; inputSchema: any }> {
    const result: Record<string, { description: string; inputSchema: any }> = {};

    for (const [name, tool] of this.tools) {
      result[name] = {
        description: tool.description,
        inputSchema: tool.inputSchema
      };
    }

    return result;
  }

  async configureKeepsync() {
    // Configure sync engine
    const SYNC_WS_URL = process.env.SYNC_WS_URL || "ws://localhost:7777/sync";
    const SYNC_URL = process.env.SYNC_URL || "http://localhost:7777";

    const wsAdapter = new BrowserWebSocketClientAdapter(SYNC_WS_URL);
    const engine = await configureSyncEngine({
      url: SYNC_URL,
      network: [wsAdapter as any as NetworkAdapterInterface],
      storage: new NodeFSStorageAdapter(),
    });

    await engine.whenReady();
  }

  async start() {
    // Ensure widgets directory exists
    try {
      await this.configureKeepsync();
      // await fs.mkdir(widgetsDir, { recursive: true });
      await mkDir(this.widgetsBasePath);
      console.log(`Widget MCP Server initialized.`);
    } catch (error) {
      console.error('Failed to create widgets directory:', error);
      throw error;
    }
  }
}
