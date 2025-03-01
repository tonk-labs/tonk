# create-tinyfoot-app

Create local-first personal software with one command.


## Structure of a Tinyfoot application 

```
my-app/
├── src/
│   ├── components/   # Reusable UI components
│   ├── hooks/        # Custom React hooks
│   ├── lib/          # Core utilities and sync engine
│   ├── services/     # External service integrations
│   ├── stores/       # State management
│   ├── views/        # Page components
│   ├── App.tsx       # Root component
│   └── index.tsx     # Entry point
├── public/           # Static assets
└── package.json      # Project configuration
```

## Prerequisites

### 1. Install Ollama
First, you'll need to install Ollama on your system:

**macOS or Linux:**
```bash
curl -fsSL https://ollama.com/install.sh | sh
```

**Windows:**
- Download the installer from [Ollama.com](https://ollama.com)

### 2. Start Ollama Server
```bash
ollama serve
```

### 3. Pull Required Model
```bash
ollama pull deepseek-r1:8b
```

### Verify Setup
You can verify everything is working by running:
```bash
curl http://localhost:11434/api/health
```

## Usage

The CLI requires:
- Node.js >= 18
- Ollama running locally
- deepseek-r1:8b model installed

```bash
npx create-tinyfoot-app my-app
```

## Features

- Interactive project setup
- AI-assisted project planning
- Customizable templates
- Built-in best practices

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
