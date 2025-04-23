const express = require('express');
const cors = require('cors');
const axios = require('axios');
const dotenv = require('dotenv');
const path = require('path');

// Load environment variables
dotenv.config({ path: path.resolve(__dirname, '../.env') });

const app = express();
const PORT = process.env.PORT || 3001;

// Enable CORS for the client
app.use(cors());

// Parse JSON request body
app.use(express.json());

// Proxy endpoint for Google Maps API
app.get('/api/proxy', async (req, res) => {
  try {
    const url = req.query.url;

    if (!url) {
      return res.status(400).json({ error: 'URL parameter is required' });
    }

    // Make the request to the Google Maps API
    const response = await axios.get(url);

    // Return the data from the Google Maps API
    res.json(response.data);
  } catch (error) {
    console.error('Proxy error:', error.message);

    // Return a more detailed error message
    res.status(500).json({
      error: 'Failed to fetch data from Google Maps API',
      details: error.message,
      status: error.response?.status,
      data: error.response?.data
    });
  }
});

// Serve static files from the React app in production
if (process.env.NODE_ENV === 'production') {
  app.use(express.static(path.join(__dirname, '../build')));

  app.get('*', (req, res) => {
    res.sendFile(path.join(__dirname, '../build', 'index.html'));
  });
}

// Start the server
app.listen(PORT, () => {
  console.log(`Proxy server running on port ${PORT}`);
});
