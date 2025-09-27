import z from 'zod';
import component_manual from '../prompts/TONK_COMPONENT_PROMPT.md';
import store_manual from '../prompts/TONK_STORE_PROMPT.md';
import { tool } from 'ai';

export const tonkManualTool = tool({
  description: 'Man pages for building with Tonk',
  inputSchema: z.object({
    type: z.enum(['component', 'store']).describe('Type of manual to display'),
  }),
  execute: async ({ type }) => {
    console.log('[TonkTool] MANUAL called:', { type });

    let manual: string;
    if (type === 'component') {
      manual = component_manual;
    } else if (type === 'store') {
      manual = store_manual;
    } else {
      throw new Error('Invalid type');
    }

    return {
      type,
      manual,
      success: true,
    };
  },
});
