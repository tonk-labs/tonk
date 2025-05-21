# Modules in Tonk

Modules in Tonk encapsulate business logic and handle IO operations (like API calls, file operations, etc.). They provide a clean way to organize your application's functionality.

## Using the API Proxy

The API proxy allows you to make requests to external APIs without exposing API keys or dealing with CORS issues. All requests to `/api/:endpoint` are automatically proxied from the frontend to the server running on port 6080.

### Example: Weather Module

The Weather Module demonstrates how to use the API proxy to fetch weather data from an external API:

```typescript
// src/modules/weather/index.ts
import { createModule, createFunction } from '../../core/module';

// Module functions
export const getCurrentWeatherFn = createFunction(
  'getCurrentWeather',
  'Get current weather for a location',
  async (location: string): Promise<WeatherResponse> => {
    try {
      const formattedLocation = formatLocation(location);
      // This request is proxied through the server
      const response = await fetch(`/api/weather/current.json?q=${formattedLocation}`);
      
      if (!response.ok) {
        throw new WeatherError(
          `Failed to fetch weather data: ${response.statusText}`,
          response.status
        );
      }
      
      return await response.json();
    } catch (error) {
      // Error handling...
    }
  }
);

// Create and export the module
export default createModule<{
  getCurrentWeather: typeof getCurrentWeatherFn.fn;
  // Other functions...
}>([
  getCurrentWeatherFn,
  // Other functions...
]);
```

### Using the Module in Components

```tsx
// src/components/WeatherWidget.tsx
import React, { useState, useEffect } from 'react';
import weatherModule from '../modules/weather';

const WeatherWidget: React.FC<{ location: string }> = ({ location }) => {
  const [weather, setWeather] = useState(null);
  
  useEffect(() => {
    const fetchWeather = async () => {
      try {
        const data = await weatherModule.getCurrentWeather(location);
        setWeather(data);
      } catch (err) {
        console.error('Error fetching weather:', err);
      }
    };

    fetchWeather();
  }, [location]);
  
  // Render weather data...
};
```

## Adding New API Endpoints

To add a new API endpoint, modify the `server/src/index.ts` file and add a new case to the switch statement in the API proxy handler. See the server README.md for more details.

## Benefits of Using Modules

1. **Separation of concerns** - Keep business logic separate from UI components
2. **Reusability** - Use the same module across different components
3. **Testability** - Test business logic independently from UI
4. **Maintainability** - Easier to understand and maintain focused modules

## Module Structure

Each module should:
- Be focused on a single responsibility
- Export all logic through the module interface
- Include comprehensive type definitions
- Handle errors gracefully
- Include detailed JSDoc documentation

For more details on creating modules, see the [CLAUDE.md](./CLAUDE.md) file.
