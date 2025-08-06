const { app, BrowserWindow } = require('electron')
const express = require('express')
const path = require('path')

let server;

const createWindow = () => {
  // Start local server
  const expressApp = express()
  expressApp.use(express.static(path.join(__dirname, 'dist')))
  server = expressApp.listen(0, () => {
    const port = server.address().port
    console.log(`Local server started on port ${port}`)
    
    const win = new BrowserWindow({
      fullscreen: true,
      webPreferences: {
        nodeIntegration: false,
        contextIsolation: true
      }
    })

    win.loadURL(`http://localhost:${port}`)
  })
}

app.whenReady().then(createWindow)

app.on('before-quit', () => {
  if (server) {
    console.log('Closing local server')
    server.close()
  }
})
