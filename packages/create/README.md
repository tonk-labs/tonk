# create

Bootstraps your Tonk app; Tonk apps are local-first personal software built to be maximally
interoperable.

## Structure of a Tonk application

```
my-app/
├── src/
│   ├── components/   # Reusable UI components
│   ├── modules/      # Any other functionality
│   ├── stores/       # State management
│   ├── views/        # Page components
│   ├── App.tsx       # Root component
│   └── index.tsx     # Entry point
├── public/           # Static assets
└── package.json      # Project configuration
```

## Usage

The CLI requires:

- Node.js >= 18

```bash
npx @tonk/create [app name]
```

The CLI will guide you through the project creation process with interactive prompts for:

- Project name
- Project type
- Platform selection
- Project description

## Development

1. Clone the repository
2. Install dependencies: `pnpm install`
3. Build the package: `pnpm build`
4. Link for local testing: `npm link`

## Testing

Run `pnpm test` to execute the test suite.

## License

Simplicity and freedom.

MIT © Tonk
