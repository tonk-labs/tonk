# LLM Instructions

This section contains all the instructions for AI coding assistants (LLMs) working with the Tonk codebase. The instructions are organized into shared common patterns and template-specific variations.

## Organization

### Shared Instructions
Common patterns and guidelines used across multiple templates:

- **[Keepsync](./shared/keepsync/README.md)** - Data synchronization patterns
  - [React/Browser](./shared/keepsync/react-browser.md) - Browser-based keepsync usage
  - [Worker/Node.js](./shared/keepsync/worker-nodejs.md) - Node.js-based keepsync usage
  - [Examples](./shared/keepsync/examples/README.md) - Code examples for both environments
- **[Components](./shared/components.md)** - Component creation guidelines
- **[Stores](./shared/stores.md)** - State management patterns
- **[Views](./shared/views.md)** - View creation guidelines
- **[Server](./shared/server.md)** - Server endpoint patterns
- **[Instructions](./shared/instructions.md)** - General instruction patterns

### Template-Specific Instructions
Variations and specifics for different template types:

- **[React](./templates/react/README.md)** - React application templates
- **[Worker](./templates/worker/README.md)** - Background worker templates
- **[Workspace](./templates/workspace/README.md)** - Full workspace templates

## Usage

These instructions serve as the single source of truth for LLM guidance. They are automatically distributed to the appropriate template locations using the `copy-llms-from-docs.js` utility.

## Maintenance

When updating LLM instructions:
1. Edit the appropriate file in this `docs/src/llms/` directory
2. Run `node utils/copy-llms-from-docs.js` to distribute changes
3. Changes will be automatically synced to all template locations

This ensures consistency across all AI coding tools (Claude, Cursor, Windsurf) and template types. 