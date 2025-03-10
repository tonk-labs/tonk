# create-tinyfoot-app

Bootstraps your tinyfoot app; tinyfoot apps are local-first personal software built to be maximally interoperable.

## Structure of a Tinyfoot application

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
npx @tonk/create-tinyfoot-app my-app
```

## Development

1. Clone the repository
2. Install dependencies: `npm install`
3. Build the package: `npm run build`
4. Link for local testing: `npm link`

## Testing

Run `npm test` to execute the test suite.

## License

Simplicity and freedom.

MIT © Tonk
