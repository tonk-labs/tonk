# Worker Keepsync Examples

This section shows examples of how to use keepsync in background workers. These examples demonstrate
data fetching, processing, and storage patterns for Node.js environments.

## API Data Fetching Worker

A complete example of fetching data from an external API and storing it in keepsync:

```typescript
{{#include ../../../../../../packages/create/templates/worker/instructions/keepsync/examples/worker/index.ts}}
```

## Key Concepts Demonstrated

1. **Data Fetching**: Fetching data from external APIs
2. **Document Operations**: Using `readDoc()` and `writeDoc()` for data storage
3. **Data Merging**: Combining existing data with new data
4. **Error Handling**: Proper error handling in worker contexts
5. **Path-based Storage**: Using filesystem-like paths for data organization

## Worker Patterns

Workers typically follow these patterns:

1. **Scheduled Data Fetching**: Periodically fetch data from external sources
2. **Document Storage**: Store processed data in keepsync documents
3. **Data Transformation**: Process raw data into structured formats
4. **Error Recovery**: Handle API failures and network issues gracefully

## Running the Example

To run this example:

1. Create a new Tonk worker: `tonk create` (choose Worker)
2. Replace the generated code with the example above
3. Configure your API endpoints and credentials
4. Start the worker: `bun run dev`

The worker will fetch data and store it in keepsync, making it available to all connected
applications!
