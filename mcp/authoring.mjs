import { guides, instructions } from './authoring-content.mjs';
import { TonkToolError } from './errors.mjs';
export { guides, instructions };
export function authoringGuide(args = {}) {
  if (
    !args ||
    Array.isArray(args) ||
    Object.keys(args).some((key) => key !== 'topic') ||
    (args.topic !== undefined && typeof args.topic !== 'string')
  )
    throw new TonkToolError('Provide only an optional guide topic.');
  if (args.topic === undefined) return { topics: Object.keys(guides), usage: instructions };
  if (!Object.hasOwn(guides, args.topic))
    throw new TonkToolError('Unknown guide; omit topic to list available manuals.');
  return { topic: args.topic, source: 'Canonical Tonk manual', text: guides[args.topic] };
}
