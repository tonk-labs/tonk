# Tonk Worker

A Tonk worker service for building local-first applications.

## Getting Started

1. Install dependencies:
   ```bash
   pnpm install
   ```

2. Start the development server:
   ```bash
   pnpm dev
   ```

3. Build for production:
   ```bash
   pnpm build
   ```

4. Register the worker with Tonk:
   ```bash
   tonk worker register
   ```

## Worker Configuration

The worker is configured using the `worker.yaml` file. You can modify this file to change the worker's behavior.

## Environment Variables

Create a `.env` file based on the `.env.example` file to configure the worker.

## Project Structure

```
my-worker/
├── src/
│   ├── index.ts    # Main entry point
├── worker.yaml     # Worker configuration
└── package.json    # Project configuration
```

## License

MIT © Tonk
