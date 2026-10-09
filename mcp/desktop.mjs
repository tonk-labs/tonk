import { connectRuntime } from './runtime-client.mjs';
import { selectTools } from './catalog.mjs';
import { TonkToolError } from './core.mjs';

export async function connectDesktop(path) {
  const request = await connectRuntime(path);
  const { tools } = await request('/tools', {});
  if (!Array.isArray(tools)) throw new Error('Missing runtime capabilities.');
  const capabilities = tools.map(tool => tool.name);
  selectTools(capabilities);
  return {
    capabilities,
    async call(name, args, signal) {
      if (!capabilities.includes(name)) throw new TonkToolError('Tool unavailable.');
      const response = await request('/call', { name, arguments: args }, signal);
      if (typeof response.error === 'string') throw new TonkToolError(response.error);
      return response.result;
    },
  };
}
