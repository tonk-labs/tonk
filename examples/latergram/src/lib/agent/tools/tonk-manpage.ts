import { tool } from 'ai';
import z from 'zod';
import unified_manual from '../prompts/TONK_UNIFIED_MANUAL.md?raw';

export const tonkManualTool = tool({
  description:
    'Read the Tonk development manual - ALWAYS read this before implementing components, pages, or stores',
  inputSchema: z.object({}),
  execute: async () => {
    console.log('[TonkTool] MANUAL called');
    console.log('[TonkTool] MANUAL success:', {
      length: unified_manual.length,
    });

    return {
      manual: unified_manual,
      success: true,
    };
  },
});
