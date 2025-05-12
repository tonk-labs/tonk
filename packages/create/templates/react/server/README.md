# Tonk Server API Proxy

This Express server provides API proxy functionality for your Tonk application.

## API Proxy Functionality

The API proxy allows your frontend to make requests to external APIs without exposing API keys or dealing with CORS issues. All requests to `/api/:endpoint` are automatically proxied from the frontend to this server running on port 6080.

### Benefits of using the API proxy:
1. **CORS handling** - Make cross-origin requests without browser restrictions
2. **Security** - Store API keys and sensitive credentials server-side
3. **Local resource access** - Access local file system or other local resources
4. **Request transformation** - Modify requests or responses before forwarding

## How it works:
- Your frontend makes requests to `/api/your-endpoint`
- The Vite dev server proxies these to `http://localhost:6080/api/your-endpoint`
- Your Express server handles the request, makes any needed external calls, and returns the response

## Setup

1. Copy the `.env.example` file to `.env` and add your API keys:
```
cp .env.example .env
```

2. Install server dependencies:
```
cd server
pnpm install
pnpm build
```

3. Start the server:
```
pnpm start
```

4. Start the development server in the top-level app project:
```
tonk dev
```

## Available Endpoints

### GET /api/hello

Returns a simple hello world message.

Example response:
```
Hello World Api!
```

### GET /api/weather

Proxies requests to a weather API service. Requires a `WEATHER_API_KEY` in your `.env` file.

Example request:
```
GET /api/weather/current.json?q=London
```

### GET /ping

Health check endpoint that returns a 200 OK response. This endpoint is used for health checks and service discovery.

Example response:
```
OK
```

## Adding New API Endpoints

To add a new API endpoint, modify the `server/src/index.ts` file and add a new case to the switch statement in the API proxy handler:

```javascript
// Handle different API endpoints
switch (endpoint) {
  case 'weather':
    // Existing weather API proxy...
    break;
    
  case 'your-new-endpoint':
    // Add your new API proxy configuration here
    const apiKey = process.env.YOUR_API_KEY;
    if (!apiKey) {
      return res.status(500).json({ error: 'API key not configured' });
    }
    
    createProxyMiddleware({
      target: 'https://api.example.com',
      changeOrigin: true,
      pathRewrite: {
        [`^/api/your-new-endpoint`]: '',
      },
      onProxyReq: (proxyReq) => {
        // Add API key to the request
        proxyReq.path = `${proxyReq.path}${proxyReq.path.includes('?') ? '&' : '?'}key=${apiKey}`;
      }
    })(req, res, next);
    break;
    
  // ...
}
```

Don't forget to add the corresponding API key to your `.env` file:
```
YOUR_API_KEY=your_api_key_here
```
