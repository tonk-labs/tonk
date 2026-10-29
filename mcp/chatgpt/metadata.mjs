// Optional compatibility metadata; standard MCP Apps lives in ../apps/.
export function openAIResourceMetadata(origin) {
  return {
    'openai/widgetCSP': {
      connect_domains: [origin],
      frame_domains: [],
      resource_domains: [],
    },
  };
}
