import catalog from './tools.json' with { type: 'json' };

export const toolDefinitions = catalog.tools;
export const toolByName = new Map(toolDefinitions.map((tool) => [tool.name, tool]));

// The runtime selects capabilities; it cannot introduce a second tool dialect.
export function selectTools(names) {
  if (
    !Array.isArray(names) ||
    new Set(names).size !== names.length ||
    names.some((name) => !toolByName.has(name))
  ) {
    throw new Error('Unsupported Tonk tool capabilities.');
  }
  return names.map((name) => structuredClone(toolByName.get(name)));
}

// These tools operate without an already-selected space. Keep target policy
// separate from tool registration so account and pinned hosts share schemas.
const unscopedTools = new Set(['tonk_list_spaces', 'tonk_guide', 'tonk_create_space']);

export function inputSchemaFor(tool, { spaceSelection = false, agentLinkSelection = false } = {}) {
  const schema = structuredClone(tool.inputSchema);
  if (spaceSelection && agentLinkSelection) throw Error('Choose one authority mode.');
  if (agentLinkSelection && !unscopedTools.has(tool.name)) {
    return {
      ...schema,
      properties: {
        ...schema.properties,
        agent_link: {
          type: 'string',
          minLength: 1,
          maxLength: 160000,
          description:
            'Complete Copy agent link URL, including its private fragment. Required on every call. Never repeat it in responses.',
        },
      },
      required: [...(schema.required ?? []), 'agent_link'],
    };
  }
  if (!spaceSelection || unscopedTools.has(tool.name)) return schema;
  return {
    ...schema,
    properties: {
      ...schema.properties,
      space: {
        type: 'string',
        pattern: '^did:key:[A-Za-z0-9]+$',
        description: 'Exact subject returned by tonk_list_spaces.',
      },
    },
    required: [...(schema.required ?? []), 'space'],
  };
}
