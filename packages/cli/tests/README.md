# CLI Tests

This directory contains comprehensive tests for the Tonk CLI package.

## Structure

```
tests/
├── setup.ts              # Global test setup
├── fixtures/              # Test data and fixtures
│   ├── configs/           # Sample configuration files
│   ├── projects/          # Sample project structures
│   └── workers/           # Sample worker implementations
├── mocks/                 # Mock implementations
│   ├── fs.ts             # File system mocks
│   ├── child_process.ts  # Process execution mocks
│   └── network.ts        # Network request mocks
├── helpers/               # Test utilities
│   ├── cli.ts            # CLI test helper
│   ├── filesystem.ts     # File system helpers
│   └── assertions.ts     # Custom assertions
├── unit/                  # Unit tests
│   ├── commands/         # Command handler tests
│   ├── utils/            # Utility function tests
│   └── lib/              # Library tests
└── integration/           # Integration tests
    ├── cli.test.ts       # Full CLI command tests
    ├── init-flow.test.ts # Init command flow
    └── ...
```

## Running Tests

### All Tests

```bash
npm test
```

### Unit Tests Only

```bash
npm run test:unit
```

### Integration Tests Only

```bash
npm run test:integration
```

### Watch Mode

```bash
npm run test:watch
```

### With Coverage

```bash
npm run test:coverage
```

### UI Mode

```bash
npm run test:ui
```

## Test Categories

### Unit Tests

- Test individual functions and modules in isolation
- Mock external dependencies
- Fast execution
- High code coverage

### Integration Tests

- Test complete command flows
- Use real file system operations in temporary directories
- Test CLI interactions end-to-end
- Slower execution but higher confidence

## Test Helpers

### CLITestHelper

Provides utilities for running CLI commands in tests:

```typescript
const cli = new CLITestHelper();
const result = await cli.run(['init', 'my-project']);
```

### TempDirectoryHelper

Manages temporary directories for tests:

```typescript
const tempDir = new TempDirectoryHelper();
await tempDir.create();
// ... use tempDir.getPath() for operations
await tempDir.cleanup();
```

### Custom Assertions

Provides domain-specific assertions:

```typescript
await expectFileExists(filePath);
expectCLISuccess(result);
expectCLIOutput(result, 'expected output');
```

## Test Environment

Tests run with the following environment:

- `NODE_ENV=test`
- `TONK_TEST_MODE=true`
- `DISABLE_ANALYTICS=true` (to prevent analytics calls)

## Mocking Strategy

- **File System**: Mock fs-extra operations for unit tests, use real FS for integration tests
- **Child Processes**: Mock process execution for predictable behavior
- **Network**: Mock HTTP requests and responses
- **Analytics**: Mock PostHog calls to prevent external requests

## Coverage Goals

- Minimum 80% line coverage
- Minimum 80% function coverage
- Minimum 75% branch coverage
- Minimum 80% statement coverage

## Best Practices

1. **Isolation**: Each test should be independent and not rely on other tests
2. **Cleanup**: Always clean up temporary files and directories
3. **Deterministic**: Tests should produce consistent results
4. **Fast Unit Tests**: Keep unit tests fast by mocking external dependencies
5. **Realistic Integration Tests**: Integration tests should simulate real usage
6. **Clear Naming**: Test names should clearly describe what is being tested
7. **Error Cases**: Test both success and failure scenarios

## Debugging Tests

### Run Specific Test

```bash
npx vitest run tests/unit/utils/version.test.ts
```

### Debug Mode

```bash
npx vitest --reporter=verbose
```

### UI Debug

```bash
npm run test:ui
```

The Vitest UI provides excellent debugging capabilities including:

- Interactive test runner
- Code coverage visualization
- Test output inspection
- File watching
