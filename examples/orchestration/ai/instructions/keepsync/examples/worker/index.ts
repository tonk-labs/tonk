// Example showing how to fetch data from an API and store it in keepsync
import { writeDoc, readDoc } from "@tonk/keepsync";

// Path where we'll store our data in keepsync
const KEEPSYNC_DOC_PATH = "api-data/weather";

// Function to fetch data from a weather API
async function fetchWeatherData() {
  const response = await fetch(
    "https://api.weather.com/current?location=london",
  );
  const data = await response.json();

  console.log("Fetching data from weather API...");

  // This is just example data that would come from the API
  return {
    temperature: data.temp,
    condition: data.condition,
    humidity: data.humidity,
    windSpeed: data.windSpeed,
    location: data.city,
    timestamp: new Date().toISOString(),
  };
}

// Main function to demonstrate the concept
async function main() {
  try {
    // 1. Fetch data from the API
    const weatherData = await fetchWeatherData();
    console.log("Received weather data:", weatherData);

    // 2. Store the data in keepsync
    const existingData = await readDoc(KEEPSYNC_DOC_PATH);
    const updatedData = { ...existingData, ...weatherData };
    await writeDoc(KEEPSYNC_DOC_PATH, updatedData);

    console.log(
      `Successfully stored weather data in keepsync at ${KEEPSYNC_DOC_PATH}`,
    );
  } catch (error) {
    console.error("Error:", error);
  }
}

main();
