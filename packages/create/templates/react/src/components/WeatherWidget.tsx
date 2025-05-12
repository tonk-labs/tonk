import React, { useState, useEffect } from 'react';
import weatherModule from '../modules/weather';

interface WeatherWidgetProps {
  location: string;
}

const WeatherWidget: React.FC<WeatherWidgetProps> = ({ location }) => {
  const [weather, setWeather] = useState<any>(null);
  const [loading, setLoading] = useState<boolean>(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const fetchWeather = async () => {
      try {
        setLoading(true);
        setError(null);
        const data = await weatherModule.getCurrentWeather(location);
        setWeather(data);
      } catch (err) {
        console.error('Error fetching weather:', err);
        setError('Failed to load weather data. Please try again later.');
      } finally {
        setLoading(false);
      }
    };

    fetchWeather();
  }, [location]);

  if (loading) {
    return <div className="weather-widget loading">Loading weather data...</div>;
  }

  if (error) {
    return <div className="weather-widget error">{error}</div>;
  }

  if (!weather) {
    return null;
  }

  return (
    <div className="weather-widget">
      <h3>Weather for {weather.location.name}</h3>
      <div className="weather-current">
        <div className="weather-temp">
          <span className="temp">{weather.current.temp_c}°C</span>
          <span className="feels-like">Feels like: {weather.current.feelslike_c}°C</span>
        </div>
        <div className="weather-condition">
          <img 
            src={weather.current.condition.icon} 
            alt={weather.current.condition.text} 
          />
          <span>{weather.current.condition.text}</span>
        </div>
      </div>
      <div className="weather-details">
        <div className="weather-detail">
          <span className="label">Humidity:</span>
          <span className="value">{weather.current.humidity}%</span>
        </div>
        <div className="weather-detail">
          <span className="label">Wind:</span>
          <span className="value">
            {weather.current.wind_kph} km/h {weather.current.wind_dir}
          </span>
        </div>
        <div className="weather-detail">
          <span className="label">UV Index:</span>
          <span className="value">{weather.current.uv}</span>
        </div>
      </div>
    </div>
  );
};

export default WeatherWidget;
