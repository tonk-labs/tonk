import z from 'zod';
import component_manual from '../prompts/TONK_COMPONENT_PROMPT.md?raw';
import store_manual from '../prompts/TONK_STORE_PROMPT.md?raw';
import page_manual from '../prompts/TONK_PAGE_PROMPT.md?raw';
import { tool } from 'ai';

const availablePages = {
  component: {content:component_manual, description: "ALWAYS read the component guidelines when you are creating a new component"},
  store: {content: store_manual, description: "ALWAYS read the store guidelines when you are creating a new store"},
  page: {content: page_manual, description: "ALWAYS read the page guidelines when you are creating a new page"},
}

const printPages = ():string => {
  let result: string = "";
  Object.entries(availablePages).forEach(([key, value]) => {
    result += `${key}: ${value.description}\n`;
  });
  return result;
}

export const tonkManualTool = tool({
  description: `Man pages for building with Tonk - ${printPages()}`,
  inputSchema: z.object({
    page: z.enum(['component', 'store', 'page']).describe('Type of manual to display'),
  }),
  execute: async ({ page }) => {
    console.log('[TonkTool] MANUAL called:', { page });

    let manual: string;
    if (page in availablePages) {
      manual = availablePages[page].content;
      console.log('[TonkTool] MANUAL success:', { page, manual });
    } else {
      throw new Error('Invalid page');
    }

    return {
      page,
      manual,
      success: true,
    };
  },
});
