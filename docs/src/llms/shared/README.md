# Shared Instructions

This section contains common instructions and guidelines that are used across multiple Tonk templates. These instructions eliminate duplication and ensure consistency across different template types.

## Available Shared Instructions

### Core Technologies
- **[Keepsync](./keepsync/README.md)** - Data synchronization and real-time collaboration
  - Environment-specific implementations for React/Browser and Worker/Node.js
  - Complete code examples for both environments
- **[Components](./components.md)** - React component creation guidelines
- **[Stores](./stores.md)** - Zustand state management patterns
- **[Views](./views.md)** - View creation and organization
- **[Server](./server.md)** - Express server endpoint patterns
- **[Instructions](./instructions.md)** - General instruction and documentation patterns

### Usage

These shared instructions are used as the foundation for all Tonk templates. They provide:

1. **Consistent Patterns**: Common approaches across all template types
2. **Best Practices**: Proven patterns for Tonk development
3. **Complete Examples**: Working code that demonstrates concepts
4. **Environment Specifics**: Tailored instructions for different runtime environments

### Template Integration

These shared instructions are automatically distributed to the appropriate template locations. When you edit these files, the changes are propagated to all relevant templates using the `copy-llms-from-docs.js` utility.

Each template may have additional specific instructions in the [Template-Specific](../templates/README.md) section for unique requirements or variations. 