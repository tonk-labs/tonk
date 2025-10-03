import { tool } from 'ai';
import z from 'zod';
import component_core from '../prompts/TONK_COMPONENT_CORE.md?raw';
import component_errors from '../prompts/TONK_ERROR_PATTERNS.md?raw';
import page_manual from '../prompts/TONK_PAGE_PROMPT.md?raw';
import store_manual from '../prompts/TONK_STORE_PROMPT.md?raw';

interface PageConfig {
  content: string;
  description: string;
  errorPatterns?: string;
}

const availablePages: Record<string, PageConfig> = {
  component: {
    content: component_core,
    errorPatterns: component_errors,
    description:
      'ALWAYS read the component guidelines when you are creating a new component',
  },
  store: {
    content: store_manual,
    description:
      'ALWAYS read the store guidelines when you are creating a new store',
  },
  page: {
    content: page_manual,
    description:
      'ALWAYS read the page guidelines when you are creating a new page',
  },
};

const printPages = (): string => {
  let result: string = '';
  Object.entries(availablePages).forEach(([key, value]) => {
    result += `${key}: ${value.description}\n`;
  });
  return result;
};

export const tonkManualTool = tool({
  description: `Man pages for building with Tonk - ${printPages()}`,
  inputSchema: z.object({
    page: z
      .enum(['component', 'store', 'page'])
      .describe('Type of manual to display'),
    includeErrorPatterns: z
      .boolean()
      .optional()
      .describe('Include error patterns and anti-patterns (use when fixing errors)'),
  }),
  execute: async ({ page, includeErrorPatterns = false }) => {
    console.log('[TonkTool] MANUAL called:', { page, includeErrorPatterns });

    let manual: string;
    if (page in availablePages) {
      const pageConfig = availablePages[page];
      manual = pageConfig.content;
      
      if (includeErrorPatterns && pageConfig.errorPatterns) {
        manual += `\n\n---\n\n# ERROR PATTERNS\n\nThe following section contains common error patterns and how to fix them. Review these carefully to avoid repeating mistakes.\n\n${pageConfig.errorPatterns}`;
        console.log('[TonkTool] MANUAL with error patterns:', { page });
      }
      
      console.log('[TonkTool] MANUAL success:', { page, length: manual.length });
    } else {
      throw new Error('Invalid page');
    }

    return {
      page,
      manual,
      includeErrorPatterns,
      success: true,
    };
  },
});
