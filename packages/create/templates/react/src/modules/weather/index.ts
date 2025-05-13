/**
 * Weather Module for fetching weather data using the API proxy
 */
import { createModule, createFunction } from "../core/module";

// Type definitions
interface WeatherConfig {
  /** Units for temperature (default: metric) */
  units?: "metric" | "imperial";
  /** Language for weather descriptions */
  language?: string;
}

interface WeatherCondition {
  /** Weather condition code */
  code: number;
  /** Weather condition text */
  text: string;
  /** Icon URL */
  icon: string;
}

interface CurrentWeather {
  /** Temperature in requested units */
  temp_c: number;
  /** Temperature in Fahrenheit */
  temp_f: number;
  /** Weather condition */
  condition: WeatherCondition;
  /** Wind speed in kph */
  wind_kph: number;
  /** Wind speed in mph */
  wind_mph: number;
  /** Wind direction */
  wind_dir: string;
  /** Humidity percentage */
  humidity: number;
  /** UV Index */
  uv: number;
}

interface WeatherResponse {
  /** Location information */
  location: {
    /** Location name */
    name: string;
    /** Region */
    region: string;
    /** Country */
    country: string;
    /** Latitude */
    lat: number;
    /** Longitude */
    lon: number;
    /** Local time */
    localtime: string;
  };
  /** Current weather data */
  current: CurrentWeather;
}

// Error handling
class WeatherError extends Error {
  constructor(
    message: string,
    public statusCode?: number,
    public rawError?: unknown,
  ) {
    super(message);
    this.name = "WeatherError";
  }
}

// Module state
let config: WeatherConfig = {
  units: "metric",
  language: "en",
};

// Helper functions
const formatLocation = (location: string): string => {
  return encodeURIComponent(location.trim());
};

/**
 * Configure the weather module
 */
export const configureWeatherFn = createFunction(
  "configureWeather",
  "Configure the weather module settings",
  (newConfig: Partial<WeatherConfig>): void => {
    config = { ...config, ...newConfig };
  },
);

/**
 * Get current weather for a location
 *
 * @param location - City name, postal code, or coordinates
 * @returns Current weather data
 * @throws {WeatherError} When the API request fails
 */
export const getCurrentWeatherFn = createFunction(
  "getCurrentWeather",
  "Get current weather for a location",
  async (location: string): Promise<WeatherResponse> => {
    try {
      const formattedLocation = formatLocation(location);
      const response = await fetch(
        `/api/weather/current.json?q=${formattedLocation}`,
      );

      if (!response.ok) {
        throw new WeatherError(
          `Failed to fetch weather data: ${response.statusText}`,
          response.status,
        );
      }

      return await response.json();
    } catch (error) {
      if (error instanceof WeatherError) {
        throw error;
      }

      throw new WeatherError(
        "Failed to fetch weather data",
        error instanceof Response ? error.status : undefined,
        error,
      );
    }
  },
);

/**
 * Get weather forecast for a location
 *
 * @param location - City name, postal code, or coordinates
 * @param days - Number of days for forecast (1-10)
 * @returns Forecast weather data
 * @throws {WeatherError} When the API request fails
 */
export const getForecastFn = createFunction(
  "getForecast",
  "Get weather forecast for a location",
  async (location: string, days: number = 3): Promise<any> => {
    try {
      const formattedLocation = formatLocation(location);
      const response = await fetch(
        `/api/weather/forecast.json?q=${formattedLocation}&days=${days}`,
      );

      if (!response.ok) {
        throw new WeatherError(
          `Failed to fetch forecast data: ${response.statusText}`,
          response.status,
        );
      }

      return await response.json();
    } catch (error) {
      if (error instanceof WeatherError) {
        throw error;
      }

      throw new WeatherError(
        "Failed to fetch forecast data",
        error instanceof Response ? error.status : undefined,
        error,
      );
    }
  },
);

// Create and export the module
export default createModule<{
  configureWeather: typeof configureWeatherFn.fn;
  getCurrentWeather: typeof getCurrentWeatherFn.fn;
  getForecast: typeof getForecastFn.fn;
}>([configureWeatherFn, getCurrentWeatherFn, getForecastFn]);

// Usage examples:
/*
// Configure the weather module (optional)
weather.configureWeather({
  units: 'imperial',
  language: 'en'
});

// Get current weather for London
try {
  const weatherData = await weather.getCurrentWeather('London');
  console.log(`Current temperature in ${weatherData.location.name}: ${weatherData.current.temp_c}°C`);
} catch (error) {
  console.error('Failed to get weather:', error);
}

// Get 5-day forecast for New York
try {
  const forecastData = await weather.getForecast('New York', 5);
  console.log('5-day forecast for New York:', forecastData);
} catch (error) {
  console.error('Failed to get forecast:', error);
}
*/
