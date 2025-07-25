# DO NOT EDIT - AUTO-GENERATED FROM docs/src/llms/templates/worker/README.md
# This file is automatically generated from the documentation.
# Edit the source file instead: docs/src/llms/templates/worker/README.md
# 
# Generated on: 2025-07-17T13:11:10.055Z
# 

## File Listening Pattern - IMPORTANT

When implementing file watching functionality, **ALWAYS use the existing FileListener pattern** from `src/listeners/fileListener.ts`. Do NOT create custom file watcher services.

### Proper FileListener Usage:

1. **Import the FileListener**: Use `import { FileListener, createAndStartFileListener } from "./listeners/fileListener";`

2. **Define your data transformation**: Create a transformer function that converts file content to your desired format

3. **Define path transformation**: Create a function that maps file paths to keepsync document paths  

4. **Define data mapper**: Create a mapper function that handles how the transformed data gets written to keepsync

5. **Use the pattern**: Initialize the FileListener with your configuration

### Example Implementation:
```typescript
import { createAndStartFileListener } from "./listeners/fileListener";
// Note: Instead of creating services/ directory, add API routes to the /server proxy in the react template
// The server is located at server/src/index.ts and handles API endpoints

// In your main initialization:
const fileListener = await createAndStartFileListener(
  '/path/to/obsidian/file.md',
  (filePath) => 'obsidian/topics', // keepsync path
  (fileContent, filePath) => parseObsidianContent(fileContent), // transformer
  (existingDoc, newData) => ({ ...existingDoc, ...newData }) // mapper
);
```

### Why This Pattern?

- **Consistency**: All workers use the same file watching approach
- **Reliability**: The FileListener handles edge cases, debouncing, and error recovery
- **Integration**: Built-in keepsync integration with proper data mapping
- **Monitoring**: Standardized logging and error handling

**Remember**: The FileListener already handles chokidar setup, file reading, error handling, and keepsync integration. Focus on your business logic (data transformation) rather than file watching infrastructure.
