import { z } from 'zod';
import path from 'path';
import { ls, mkDir, readDoc, writeDoc } from '@tonk/keepsync';

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
          writeDoc(`/widgets/${normalizedPath}`, {
            content,
            encoding,
            timestamp: Date.now(),
          });

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
          const doc = await readDoc<{ content: string; encoding: string; timestamp: number }>(fullPath);
          if (!doc) throw new Error(`File not found: ${filePath}`);

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

    // Tool to read comprehensive widget templates
    this.addTool({
      name: 'read_widget_templates',
      description: 'Read all widget templates including component and index templates with guidelines',
      inputSchema: z.object({
        templateType: z.enum(['component', 'index', 'all']).default('all').describe('Type of template to read'),
      }),
      handler: async (args: { templateType: 'component' | 'index' | 'all' }) => {
        const { templateType } = args;
        
        try {
          const templates: any = {};
          
          if (templateType === 'component' || templateType === 'all') {
            const componentDoc = await readDoc<{ content: string; encoding: string; timestamp: number }>('/widgets/templates/component-template.tsx');
            if (componentDoc) {
              templates.component = componentDoc.content;
            }
          }
          
          if (templateType === 'index' || templateType === 'all') {
            const indexDoc = await readDoc<{ content: string; encoding: string; timestamp: number }>('/widgets/templates/index-template.ts');
            if (indexDoc) {
              templates.index = indexDoc.content;
            }
          }
          
          return {
            content: [{
              type: 'text' as const,
              text: JSON.stringify({
                templates,
                importPaths: {
                  BaseWidget: "import BaseWidget from '../../templates/BaseWidget';",
                  WidgetProps: "import { WidgetProps } from '../../index';",
                  React: "import React, { useState, useEffect } from 'react';"
                },
                guidelines: [
                  "Always use the exact import paths shown above",
                  "Extend WidgetProps interface for custom props", 
                  "Use BaseWidget as the root component",
                  "Follow the component structure in the template",
                  "Include proper TypeScript typing",
                  "Use Tailwind CSS for styling",
                  "Replace MyWidget with your actual widget name",
                  "Update the widget ID, name, and description in index.ts",
                  "Set appropriate default width and height",
                  "Include proper state management with useState",
                  "Add useEffect for initialization and cleanup",
                  "Use theme and size props for customization",
                  "CRITICAL: Always provide unique 'key' props when rendering arrays or lists",
                  "CRITICAL: Use map() with proper keys: items.map((item, index) => <div key={item.id || index}>...)",
                  "CRITICAL: Avoid creating implicit JSX arrays - use single elements or proper keyed arrays",
                  "CRITICAL: When using conditional rendering with multiple elements, wrap in fragments or single containers",
                  "CRITICAL: For dynamic lists, use stable unique identifiers as keys, not just array indices when possible"
                ]
              })
            }]
          };
        } catch (error) {
          throw new Error(`Failed to read widget templates: ${error instanceof Error ? error.message : 'Unknown error'}`);
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
      await mkDir(this.widgetsBasePath);
      console.log(`Widget MCP Server initialized.`);
    } catch (error) {
      console.error('Failed to create widgets directory:', error);
      throw error;
    }
  }
}
