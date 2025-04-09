/**
 * Simple test script to verify that the proxy server is working correctly
 * 
 * Usage:
 * 1. Start the proxy server: npm run dev
 * 2. In a separate terminal, run: node test.js
 */

const axios = require('axios');
const dotenv = require('dotenv');
const path = require('path');

// Load environment variables from parent directory
dotenv.config({ path: path.resolve(__dirname, '../.env') });

const GOOGLE_MAPS_API_KEY = process.env.GOOGLE_MAPS_API_KEY;

if (!GOOGLE_MAPS_API_KEY) {
  console.error('GOOGLE_MAPS_API_KEY not found in .env file');
  process.exit(1);
}

// Test place ID (Google HQ)
const placeId = 'ChIJj61dQgK6j4AR4GeTYWZsKWw';

// Google Maps API URL
const googleMapsUrl = `https://maps.googleapis.com/maps/api/place/details/json?place_id=${placeId}&fields=name,opening_hours&key=${GOOGLE_MAPS_API_KEY}`;

// Proxy URL
const proxyUrl = `http://localhost:3001/api/proxy?url=${encodeURIComponent(googleMapsUrl)}`;

console.log('Testing proxy server...');
console.log(`Place ID: ${placeId}`);
console.log(`Proxy URL: ${proxyUrl}`);

// Test the proxy
axios.get(proxyUrl)
  .then(response => {
    console.log('\nProxy server is working correctly!');
    console.log('\nResponse data:');
    console.log(JSON.stringify(response.data, null, 2));

    if (response.data.status === 'OK') {
      console.log('\nPlace details:');
      console.log(`Name: ${response.data.result.name}`);

      if (response.data.result.opening_hours) {
        console.log('\nOpening hours:');
        console.log(`Open now: ${response.data.result.opening_hours.open_now ? 'Yes' : 'No'}`);

        if (response.data.result.opening_hours.weekday_text) {
          console.log('\nWeekday hours:');
          response.data.result.opening_hours.weekday_text.forEach(day => {
            console.log(`  ${day}`);
          });
        }
      } else {
        console.log('\nNo opening hours available for this place.');
      }
    } else {
      console.log(`\nError: ${response.data.status}`);
      console.log(response.data.error_message || 'No error message provided');
    }
  })
  .catch(error => {
    console.error('\nError testing proxy server:');
    if (error.response) {
      console.error(`Status: ${error.response.status}`);
      console.error(error.response.data);
    } else {
      console.error(error.message);
    }
  });
