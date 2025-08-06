import { z } from 'zod';
import fs from 'fs/promises';
import path from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const projectRoot = path.join(__dirname, '..', '..');
const widgetsDir = path.join(projectRoot, 'widgets');

interface MCPTool {
  name: string;
  description: string;
  inputSchema: z.ZodSchema;
  handler: (args: any) => Promise<{ content: Array<{ type: 'text'; text: string }> }>;
}

export class WidgetMCPServer {
  private tools: Map<string, MCPTool> = new Map();

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
      description: 'Write a file in the widgets directory with security restrictions',
      inputSchema: z.object({
        path: z.string().describe('Relative path within widgets directory'),
        content: z.string().describe('File content to write'),
        encoding: z.enum(['utf8', 'base64']).default('utf8').describe('File encoding'),
      }),
      handler: async (args: { path: string; content: string; encoding: 'utf8' | 'base64' }) => {
        const { path: filePath, content, encoding } = args;
        
        // Security: Normalize and validate path
        const normalizedPath = path.normalize(filePath);
        if (normalizedPath.includes('..') || path.isAbsolute(normalizedPath)) {
          throw new Error('Path traversal not allowed. Use relative paths within widgets directory only.');
        }

        const absolutePath = path.join(widgetsDir, normalizedPath);
        
        // Ensure we're still within widgets directory after normalization
        if (!absolutePath.startsWith(widgetsDir)) {
          throw new Error('Access denied: Path must be within widgets directory');
        }

        try {
          // Auto-create directories
          await fs.mkdir(path.dirname(absolutePath), { recursive: true });
          
          // Write file
          await fs.writeFile(absolutePath, content, encoding as BufferEncoding);
          
          // Get file stats for response
          const stats = await fs.stat(absolutePath);
          
          return {
            content: [{
              type: 'text' as const,
              text: JSON.stringify({
                path: filePath,
                absolutePath,
                size: stats.size,
                message: `Successfully wrote ${stats.size} bytes to ${filePath}`
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
        const { path: filePath, encoding } = args;
        
        // Security: Normalize and validate path
        const normalizedPath = path.normalize(filePath);
        if (normalizedPath.includes('..') || path.isAbsolute(normalizedPath)) {
          throw new Error('Path traversal not allowed. Use relative paths within widgets directory only.');
        }

        const absolutePath = path.join(widgetsDir, normalizedPath);
        
        // Ensure we're still within widgets directory
        if (!absolutePath.startsWith(widgetsDir)) {
          throw new Error('Access denied: Path must be within widgets directory');
        }

        try {
          const content = await fs.readFile(absolutePath, encoding as BufferEncoding);
          const stats = await fs.stat(absolutePath);
          
          return {
            content: [{
              type: 'text' as const,
              text: JSON.stringify({
                path: filePath,
                content: content.toString(),
                size: stats.size,
                modified: stats.mtime.toISOString()
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
        const normalizedPath = path.normalize(dirPath || '.');
        if (normalizedPath.includes('..') || path.isAbsolute(normalizedPath)) {
          throw new Error('Path traversal not allowed. Use relative paths within widgets directory only.');
        }

        const absolutePath = path.join(widgetsDir, normalizedPath);
        
        // Ensure we're still within widgets directory
        if (!absolutePath.startsWith(widgetsDir)) {
          throw new Error('Access denied: Path must be within widgets directory');
        }

        try {
          const entries = await fs.readdir(absolutePath, { withFileTypes: true });
          const items = entries.map(entry => ({
            name: entry.name,
            type: entry.isDirectory() ? 'directory' : 'file',
            path: path.join(dirPath, entry.name)
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

  async start() {
    // Ensure widgets directory exists
    try {
      await fs.mkdir(widgetsDir, { recursive: true });
      console.log(`Widget MCP Server initialized. Widgets directory: ${widgetsDir}`);
    } catch (error) {
      console.error('Failed to create widgets directory:', error);
      throw error;
    }
  }
}