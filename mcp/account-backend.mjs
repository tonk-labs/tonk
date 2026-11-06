import { authoringGuide } from './authoring.mjs';
import { workerPrimitive, primitiveTools } from './primitive.mjs';
import { TonkToolError } from './errors.mjs';

// Account authority is supplied by an authenticated host, independently of
// MCP transport or presentation. Selection is revalidated for every call.
export function accountBackend({ principal, tenants, allowsWrites = true }) {
  const spaceTools = allowsWrites ? primitiveTools : ['tonk_query'];
  const capabilities = ['tonk_guide', 'tonk_list_spaces', ...spaceTools];
  return {
    spaceSelection: true,
    capabilities,
    async call(name, args, signal) {
      if (!capabilities.includes(name)) throw new TonkToolError('Tool unavailable.');
      if (name === 'tonk_guide') return authoringGuide(args);
      if (name === 'tonk_list_spaces') return { spaces: await tenants.list(principal) };
      const { space, ...input } = args;
      const worker = await tenants.resolve(principal, space);
      return workerPrimitive({
        subject: space,
        allowsWrites,
        request: (path, options) => worker.request(path, options),
      }).call(name, input, signal);
    },
  };
}
