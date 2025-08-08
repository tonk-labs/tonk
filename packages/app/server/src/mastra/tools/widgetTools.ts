import { createTool } from '@mastra/core/tools';
import { z } from 'zod';
import { WidgetMCPServer } from '../../mcpServer.js';


export async function createWidgetTools(mcpServer: WidgetMCPServer) {
  return {
    // Tool to generate a complete widget from scratch
    generateWidget: createTool({
      id: 'generate_widget',
      description: 'Generate a complete React widget with TypeScript, including component file and widget definition',
      inputSchema: z.object({
        widgetId: z.string().describe('Unique identifier for the widget (kebab-case)'),
        name: z.string().describe('Display name for the widget'),
        description: z.string().describe('Description of what the widget does'),
        category: z.string().default('custom').describe('Widget category (e.g., productivity, entertainment, utility)'),
        icon: z.string().default('🔧').describe('Emoji icon for the widget'),
        width: z.number().default(300).describe('Default width in pixels'),
        height: z.number().default(200).describe('Default height in pixels'),
        componentCode: z.string().describe('Complete React component code for the widget'),
      }),
      execute: async ({ context }) => {
        const { widgetId, name, description, category, icon, width, height, componentCode } = context;

        try {
          // Create widget directory
          const widgetDir = `generated/${widgetId}`;

          // Validate and fix import paths in component code
          let fixedComponentCode = componentCode
            .replace(/import BaseWidget from ['"]\.\.\/BaseWidget['"];?/g, "import BaseWidget from '../../templates/BaseWidget';")
            .replace(/import BaseWidget from ['"]\.\.\/\.\.\/BaseWidget['"];?/g, "import BaseWidget from '../../templates/BaseWidget';")
            .replace(/import \{ WidgetProps \} from ['"]\.\.\/index['"];?/g, "import { WidgetProps } from '../../index';")
            .replace(/import \{ WidgetProps \} from ['"]\.\.\/\.\.\/index['"];?/g, "import { WidgetProps } from '../../index';");

          // Basic React key validation (inline)
          const hasMapWithoutKey = fixedComponentCode.includes('.map(') && !fixedComponentCode.includes('key=');
          const hasArrayWithoutKey = fixedComponentCode.includes('[...Array(') && !fixedComponentCode.includes('key=');
          
          if (hasMapWithoutKey || hasArrayWithoutKey) {
            return {
              success: false,
              error: 'React key compliance issue: Found .map() or Array usage without key props',
              message: `Widget '${name}' has React key issues - all mapped elements must have unique keys`,
              recommendations: [
                'Use: items.map((item, index) => <div key={item.id || `item-${index}`}>...)',
                'Always provide unique keys for array elements',
                'Prefer stable identifiers over array indices when possible'
              ]
            };
          }

          // Write the main component file
          const componentPath = `${widgetDir}/component.tsx`;
          await mcpServer.executeTool('write_widget_file', {
            path: componentPath,
            content: fixedComponentCode
          });

          // Create the widget definition index file
          const indexContent = `import React from 'react';
import { WidgetDefinition } from '../../index';
import ${toPascalCase(widgetId)}Component from './component';

const widgetDefinition: WidgetDefinition = {
  id: '${widgetId}',
  name: '${name}',
  description: '${description}',
  component: ${toPascalCase(widgetId)}Component,
  defaultProps: {
    width: ${width},
    height: ${height},
  },
  icon: '${icon}',
  category: '${category}',
};

export default widgetDefinition;`;

          const indexPath = `${widgetDir}/index.ts`;
          await mcpServer.executeTool('write_widget_file', {
            path: indexPath,
            content: indexContent
          });

          return {
            success: true,
            widgetId,
            message: `Successfully generated widget '${name}' with ID '${widgetId}'`,
            files: [componentPath, indexPath]
          };
        } catch (error) {
          return {
            success: false,
            error: error instanceof Error ? error.message : 'Unknown error',
            message: `Failed to generate widget '${name}'`
          };
        }
      }
    }),

    // Tool to read comprehensive widget templates
    readWidgetTemplate: createTool({
      id: 'read_widget_template',
      description: 'Read comprehensive widget templates with guidelines and import paths',
      inputSchema: z.object({
        templateType: z.enum(['component', 'index', 'all']).default('all').describe('Type of template to read')
      }),
      execute: async ({ context }) => {
        const { templateType } = context;
        
        try {
          const templateResult = await mcpServer.executeTool('read_widget_templates', {
            templateType
          });

          const templateData = JSON.parse(templateResult.content[0].text);

          return {
            success: true,
            templates: templateData.templates,
            importPaths: templateData.importPaths,
            guidelines: templateData.guidelines,
            message: `Successfully read ${templateType} widget template(s) with comprehensive guidelines`
          };
        } catch (error) {
          return {
            success: false,
            error: error instanceof Error ? error.message : 'Unknown error',
            message: 'Failed to read widget templates'
          };
        }
      }
    }),

    // Tool to read BaseWidget component for reference
    readBaseWidget: createTool({
      id: 'read_base_widget',
      description: 'Read the BaseWidget component to understand the base structure and props',
      inputSchema: z.object({}),
      execute: async () => {
        try {
          const baseWidgetResult = await mcpServer.executeTool('read_widget_file', {
            path: 'templates/BaseWidget.tsx'
          });

          const baseWidgetData = JSON.parse(baseWidgetResult.content[0].text);

          return {
            success: true,
            baseWidgetCode: baseWidgetData.content,
            message: 'Successfully read BaseWidget component'
          };
        } catch (error) {
          return {
            success: false,
            error: error instanceof Error ? error.message : 'Unknown error',
            message: 'Failed to read BaseWidget component'
          };
        }
      }
    }),

    // Tool to list existing widgets
    listWidgets: createTool({
      id: 'list_widgets',
      description: 'List all existing widgets in the widgets directory',
      inputSchema: z.object({
        directory: z.string().default('generated').describe('Directory to list (generated, templates, components)')
      }),
      execute: async ({ context }) => {
        const { directory } = context;

        try {
          const listResult = await mcpServer.executeTool('list_widget_directory', {
            path: directory
          });

          const listData = JSON.parse(listResult.content[0].text);

          return {
            success: true,
            directory,
            items: listData.items,
            message: `Successfully listed ${listData.items.length} items in ${directory}`
          };
        } catch (error) {
          return {
            success: false,
            error: error instanceof Error ? error.message : 'Unknown error',
            message: `Failed to list widgets in ${directory}`
          };
        }
      }
    }),

    // Tool to read widget index for type definitions
    readWidgetIndex: createTool({
      id: 'read_widget_index',
      description: 'Read the main widget index file to understand types and interfaces',
      inputSchema: z.object({}),
      execute: async () => {
        try {
          const indexResult = await mcpServer.executeTool('read_widget_file', {
            path: 'index.ts'
          });

          const indexData = JSON.parse(indexResult.content[0].text);

          return {
            success: true,
            indexCode: indexData.content,
            message: 'Successfully read widget index file'
          };
        } catch (error) {
          return {
            success: false,
            error: error instanceof Error ? error.message : 'Unknown error',
            message: 'Failed to read widget index file'
          };
        }
      }
    }),

    // Tool to validate React key compliance in widget code
    validateReactKeys: createTool({
      id: 'validate_react_keys',
      description: 'Validate that widget code follows React key best practices',
      inputSchema: z.object({
        componentCode: z.string().describe('React component code to validate for key compliance')
      }),
      execute: async ({ context }) => {
        const { componentCode } = context;
        
        const issues: string[] = [];
        const warnings: string[] = [];
        
        // Check for common React key issues
        if (componentCode.includes('.map(') && !componentCode.includes('key=')) {
          issues.push('Found .map() usage without key prop - this will cause React warnings');
        }
        
        if (componentCode.includes('[...Array(') && !componentCode.includes('key=')) {
          issues.push('Found Array spread with map without key prop - this will cause React warnings');
        }
        
        // Check for implicit JSX arrays (multiple JSX elements as siblings)
        const jsxArrayPattern = /children:\s*\[[\s\S]*?jsx\(/g;
        if (jsxArrayPattern.test(componentCode)) {
          warnings.push('Potential implicit JSX arrays detected - ensure all elements have keys');
        }
        
        // Check for proper key patterns
        const goodKeyPatterns = [
          /key=\{[^}]*\.id[^}]*\}/,  // key={item.id}
          /key=\{`[^`]*\${[^}]*}\`\}/,  // key={`prefix-${id}`}
          /key=\{[^}]*index[^}]*\}/   // key={index} (acceptable for static lists)
        ];
        
        const hasGoodKeys = goodKeyPatterns.some(pattern => pattern.test(componentCode));
        if (componentCode.includes('key=') && !hasGoodKeys) {
          warnings.push('Keys found but may not be optimal - prefer stable unique identifiers');
        }
        
        return {
          success: true,
          isCompliant: issues.length === 0,
          issues,
          warnings,
          message: issues.length === 0 
            ? 'Code appears to be React key compliant' 
            : `Found ${issues.length} React key issues that must be fixed`,
          recommendations: [
            'Always use unique keys for array elements: items.map((item, index) => <div key={item.id || `item-${index}`}>...)',
            'Prefer stable identifiers over array indices when possible',
            'Avoid creating implicit JSX arrays without proper keys',
            'Use React.Fragment or single container elements to avoid unnecessary arrays'
          ]
        };
      }
    }),
  };
}

// Helper function to convert kebab-case to PascalCase
function toPascalCase(str: string): string {
  return str
    .split('-')
    .map(word => word.charAt(0).toUpperCase() + word.slice(1))
    .join('');
}
