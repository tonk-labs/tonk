import catalog from './tools.json' with { type: 'json' };

export const toolDefinitions = catalog.tools;
export const toolByName = new Map(toolDefinitions.map(tool => [tool.name, tool]));

// The runtime selects capabilities; it cannot introduce a second tool dialect.
export function selectTools(names) {
  if (!Array.isArray(names) || new Set(names).size !== names.length ||
      names.some(name => !toolByName.has(name))) {
    throw new Error('Unsupported Tonk tool capabilities.');
  }
  return names.map(name => structuredClone(toolByName.get(name)));
}
