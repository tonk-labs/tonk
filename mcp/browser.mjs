import { workerPrimitive } from './primitive.mjs';
import catalog from './tools.json' with { type: 'json' };
import { authoringGuide, instructions, guides } from './authoring.mjs';
export { instructions, guides };

export function createAgentTools(options) {
  const worker = workerPrimitive(options);
  const names = [...worker.capabilities, 'tonk_guide'];
  return {
    instructions,
    tools: names.map((name) => structuredClone(catalog.tools.find((tool) => tool.name === name))),
    async call(name, args, signal) {
      return name === 'tonk_guide' ? authoringGuide(args) : worker.call(name, args, signal);
    },
  };
}
