const { ipcMain, shell, BrowserWindow } = require("electron");
const path = require("node:path");
const http = require("http");
const { spawn } = require("child_process");

// Track the app window globally
let appWindow = null;
// Keep a reference to the pinggy process
let pinggyProcess = null;

// Export appWindow for use in other modules
module.exports = { appWindow };

// Helper function to start Pinggy and return the URL
const startPinggy = async () => {
  return new Promise((resolve, reject) => {
    // Kill any existing pinggy process
    if (pinggyProcess) {
      try {
        pinggyProcess.kill();
      } catch (err) {
        console.log("Error killing existing pinggy process:", err);
      }
      pinggyProcess = null;
    }

    console.log("Connecting to pinggy...");

    // Start the ssh command to connect to pinggy
    pinggyProcess = spawn("ssh", [
      "-p",
      "443",
      "-o",
      "StrictHostKeyChecking=no",
      "-R0:localhost:8080",
      "a.pinggy.io",
    ]);

    let url = null;
    const urlRegex = /(https?:\/\/[^\s]+)/;

    // Listen for output to capture the URL
    pinggyProcess.stdout.on("data", (data) => {
      const output = data.toString();
      console.log("Pinggy output:", output);

      // Look for the pinggy tunnel URLs
      const pinggyUrlRegex = /(https?:\/\/[^.]+\.a\.free\.pinggy\.link)/;
      const pinggyMatch = output.match(pinggyUrlRegex);

      if (pinggyMatch && !url) {
        // Prefer HTTPS URL if available
        if (output.includes("https://")) {
          const httpsMatch = output.match(
            /(https:\/\/[^.]+\.a\.free\.pinggy\.link)/,
          );
          if (httpsMatch) {
            url = httpsMatch[1];
          } else {
            url = pinggyMatch[1];
          }
        } else {
          url = pinggyMatch[1];
        }
        console.log("App launched at:", url);
        resolve(url);
      }
    });

    pinggyProcess.stderr.on("data", (data) => {
      console.error("Pinggy error:", data.toString());
    });

    pinggyProcess.on("error", (error) => {
      console.error("Failed to start pinggy process:", error);
      reject(error);
    });

    pinggyProcess.on("close", (code) => {
      console.log(`Pinggy process exited with code ${code}`);
      if (!url) {
        reject(
          new Error(
            `Pinggy process exited with code ${code} before providing URL`,
          ),
        );
      }
    });

    // Set a timeout in case we don't get a URL
    setTimeout(() => {
      if (!url) {
        reject(new Error("Timeout waiting for pinggy URL"));
      }
    }, 30000); // 30 second timeout
  });
};

ipcMain.handle("launch-app", async (_, projectPath) => {
  try {
    console.log("Launching app...");

    // Start Pinggy and get the public URL
    const url = await startPinggy();

    // Launch the app with the pinggy URL
    await launchApp(projectPath, url);

    // Keep the process running until interrupted
    return url;
  } catch (e) {
    console.error("Error in launch-app:", e);
    throw e;
  }
});

const launchApp = async (projectPath, pinggyUrl) => {
  try {
    const distPath = path.join(projectPath, "dist");
    // Set the distPath via API call
    const requestData = JSON.stringify({ distPath });

    return new Promise((resolve, reject) => {
      const req = http.request(
        {
          hostname: "localhost",
          port: 8080,
          path: "/api/toggle-dist-path",
          method: "POST",
          headers: {
            "Content-Type": "application/json",
            "Content-Length": Buffer.byteLength(requestData),
          },
        },
        (res) => {
          if (res.statusCode === 200) {
            // Reuse existing window if it exists, otherwise create a new one
            if (!appWindow || appWindow.isDestroyed()) {
              appWindow = new BrowserWindow({
                width: 1024,
                height: 800,
                title: "",
                webPreferences: {
                  preload: path.join(__dirname, "..", "..", "preload.js"),
                  nodeIntegration: true,
                  contextIsolation: true,
                  webSecurity: false,
                  allowRunningInsecureContent: true,
                  webgl: true,
                  enableRemoteModule: true,
                  // Enable WebAssembly
                  experimentalFeatures: true,
                },
              });

              // Enable @electron/remote for this window
              require("@electron/remote/main").enable(appWindow.webContents);

              // Set the window title to match the page title
              appWindow.webContents.on(
                "page-title-updated",
                (_event, title) => {
                  appWindow.setTitle(title);
                },
              );

              // Clean up the window reference when window is closed
              appWindow.on("closed", () => {
                appWindow = null;
                // Also kill the pinggy process when window is closed
                if (pinggyProcess) {
                  pinggyProcess.kill();
                  pinggyProcess = null;
                }
              });
            }

            appWindow.loadURL("http://localhost:8080");

            // Display the pinggy URL in the corner of the window
            appWindow.webContents.on("did-finish-load", () => {
              appWindow.webContents.executeJavaScript(`
                // Display pinggy URL
                (function() {
                    const urlDisplay = document.createElement('div');
                    urlDisplay.textContent = '${
                      pinggyUrl || "http://localhost:8080"
                    }';
                    urlDisplay.style.position = 'fixed';
                    urlDisplay.style.bottom = '6px';
                    urlDisplay.style.right = '6px';
                    urlDisplay.style.background = 'rgba(0, 0, 0, 0.7)';
                    urlDisplay.style.color = 'white';
                    urlDisplay.style.padding = '5px 10px';
                    urlDisplay.style.borderRadius = '4px';
                    urlDisplay.style.fontSize = '12px';
                    urlDisplay.style.zIndex = '9999';
                    urlDisplay.style.cursor = 'pointer';
                    urlDisplay.title = 'Click to copy URL';
                    
                    urlDisplay.addEventListener('click', function() {
                        const url = this.textContent;
                        navigator.clipboard.writeText(url).then(() => {
                            // Visual feedback
                            const originalText = this.textContent;
                            const originalBg = this.style.background;
                            
                            this.textContent = 'Copied!';
                            this.style.background = 'rgba(0, 128, 0, 0.7)';
                            
                            setTimeout(() => {
                                this.textContent = originalText;
                                this.style.background = originalBg;
                            }, 1000);
                        });
                    });
                    
                    document.body.appendChild(urlDisplay);
                })();
              `);
            });

            resolve(true);
          } else {
            reject(
              new Error(`Failed to set distPath: Status ${res.statusCode}`),
            );
          }
        },
      );

      req.on("error", (error) => {
        console.error("Error configuring server:", error);
        reject(error);
      });

      req.write(requestData);
      req.end();
    });
  } catch (error) {
    console.error("Error launching app:", error);
    throw error;
  }
};

ipcMain.handle("open-external-link", async (_event, link) => {
  // Open links in the user's default browser
  await shell.openExternal(link);
  return true;
});

ipcMain.handle("open-url-in-electron", async (_event, url) => {
  try {
    // Create a new browser window
    const newWindow = new BrowserWindow({
      width: 1024,
      height: 800,
      title: url,
      webPreferences: {
        nodeIntegration: false,
        contextIsolation: true,
        webSecurity: true,
      },
    });

    // Load the URL
    await newWindow.loadURL(url);

    // Display the URL in the corner of the window
    newWindow.webContents.on("did-finish-load", () => {
      newWindow.webContents.executeJavaScript(`
        // Display URL
        (function() {
            const urlDisplay = document.createElement('div');
            urlDisplay.textContent = '${url || "http://localhost:8080"}';
            urlDisplay.style.position = 'fixed';
            urlDisplay.style.bottom = '6px';
            urlDisplay.style.right = '6px';
            urlDisplay.style.background = 'rgba(0, 0, 0, 0.7)';
            urlDisplay.style.color = 'white';
            urlDisplay.style.padding = '5px 10px';
            urlDisplay.style.borderRadius = '4px';
            urlDisplay.style.fontSize = '12px';
            urlDisplay.style.zIndex = '9999';
            urlDisplay.style.cursor = 'pointer';
            urlDisplay.title = 'Click to copy URL';
            
            urlDisplay.addEventListener('click', function() {
                const url = this.textContent;
                navigator.clipboard.writeText(url).then(() => {
                    // Visual feedback
                    const originalText = this.textContent;
                    const originalBg = this.style.background;
                    
                    this.textContent = 'Copied!';
                    this.style.background = 'rgba(0, 128, 0, 0.7)';
                    
                    setTimeout(() => {
                        this.textContent = originalText;
                        this.style.background = originalBg;
                    }, 1000);
                });
            });
            
            document.body.appendChild(urlDisplay);
        })();
      `);
    });

    // Return success
    return true;
  } catch (error) {
    console.error("Error opening URL in new window:", error);
    throw error;
  }
});
