# Template-Specific Instructions

This section contains instructions that are specific to individual Tonk templates. These instructions complement the [Shared Instructions](../shared/README.md) with template-specific requirements, patterns, and usage guidelines.

## Available Templates

### React Template
- **[React](./react/README.md)** - React application template
  - Global project setup and configuration
  - Node module management
  - React-specific development patterns
  - Integration with Tonk server deployment

### Worker Template
- **[Worker](./worker/README.md)** - Background worker template
  - Node.js service patterns
  - File watching and processing
  - API integration patterns
  - Background task management

### Workspace Template
- **[Workspace](./workspace/README.md)** - Complete workspace template
  - Multi-component development environment
  - Console application for monitoring
  - View and worker management
  - Agent interaction patterns

## Template Structure

Each template may include:

1. **Root Instructions**: General setup and configuration
2. **Component-specific Instructions**: For templates with UI components
3. **Specialized Instructions**: For unique template features

## Usage

Template-specific instructions are combined with shared instructions to provide complete guidance for each template type. When working with a specific template, you should:

1. Start with the relevant shared instructions
2. Review the template-specific instructions for unique requirements
3. Follow the examples provided in both sections

## Maintenance

Template-specific instructions are automatically distributed to the appropriate template locations. Changes to these files are propagated using the `copy-llms-from-docs.js` utility, ensuring consistency across all AI coding tools. 