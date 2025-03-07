const express = require('express');
const path = require('path');
const port = process.env.PORT || 9999;
const app = express();

// Serve static files from the dist directory
app.use(express.static(path.join(__dirname, 'dist')));

// Send all requests to index.html so that client-side routing works
app.get('*', function(req, res) {
  res.sendFile(path.join(__dirname, 'dist', 'index.html'));
});

app.listen(port);
console.log(`Server started on port ${port}`);
console.log(`Open http://localhost:${port} to view your PWA`);
console.log('On your phone, connect to this server using your computer\'s local IP address');
console.log('For example: http://192.168.1.x:9999'); 