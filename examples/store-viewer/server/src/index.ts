import express from "express";

const app = express();
const PORT = 6080;

// Basic route for hello world
app.get("/api/hello", (req, res) => {
  res.send("Hello World Api!");
});

app.get("/ping", (req, res) => {
  res.send("pong!");
});

// Start the server
app.listen(PORT, () => {
  console.log(`Server running on port ${PORT}`);
});
