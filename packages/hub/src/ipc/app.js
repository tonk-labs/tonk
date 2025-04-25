const { ipcMain, shell, BrowserWindow } = require("electron");
const path = require("node:path");
const http = require("http");
const { spawn } = require("child_process");
const fs = require("fs");

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
      "-R0:localhost:7777",
      "a.pinggy.io",
    ]);

    let url = null;

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

const runCommandAsync = (fn) => {
  return new Promise((resolve, _reject) => {
    const r = fn();
    r.on("error", () => {
      resolve();
    });
    r.on("close", () => {
      resolve();
    });
    r.on("exit", () => {
      resolve();
    });
  });
};

const launchApp = async (projectPath, pinggyUrl) => {
  try {
    const distPath = path.join(projectPath, "dist");

    // Set the distPath via API call
    const requestData = JSON.stringify({ distPath });

    const serverPath = path.join(projectPath, "server");
    if (fs.existsSync(serverPath)) {
      const installEverything = () => {
        return spawn("pnpm", ["install"], {
          cwd: path.join(projectPath, "server"),
        });
      };
      await runCommandAsync(installEverything);
      const buildTheServer = () => {
        return spawn("pnpm", ["build"], {
          cwd: path.join(projectPath, "server"),
        });
      };
      await runCommandAsync(buildTheServer);
    }

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

              let serverProcess;
              const serverPath = path.join(
                projectPath,
                "server",
                "dist",
                "index.js",
              );
              try {
                if (fs.existsSync(serverPath)) {
                  serverProcess = spawn("node", [serverPath], {
                    cwd: projectPath,
                    stdio: "inherit",
                    shell: true,
                    env: {
                      ...process.env,
                    },
                  });

                  serverProcess.on("exit", (code) => {
                    if (code !== 0 && code !== null) {
                      console.error(
                        chalk.red(`Webpack process exited with code ${code}`),
                      );
                    }
                    process.exit(code || 0);
                  });
                }
              } catch (e) {
                console.error(e);
              }
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
                if (serverProcess) {
                  serverProcess.kill();
                }
                // Also kill the pinggy process when window is closed
                if (pinggyProcess) {
                  pinggyProcess.kill();
                  pinggyProcess = null;
                }
              });
            }

            appWindow.loadURL("http://localhost:7777");

            // Display the pinggy URL in the corner of the window
            appWindow.webContents.on("did-finish-load", () => {
              appWindow.webContents.executeJavaScript(`
                // Display pinggy URL
                (function() {
                    const urlDisplay = document.createElement('div');
                    urlDisplay.textContent = '${
                      pinggyUrl || "http://localhost:7777"
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

const launchAppDev = async (projectPath) => {
  try {
    console.log("Launching app in development mode...");

    // Set default port
    const frontendPort = 3000;

    console.log("Starting Tonk development environment...");

    // Start the tonk dev command
    const devProcess = spawn("tonk", ["dev"], {
      cwd: projectPath,
      env: {
        ...process.env,
      },
    });

    // Log output from the dev process
    devProcess.stdout.on("data", (data) => {
      console.log(`Tonk dev: ${data}`);
    });

    devProcess.stderr.on("data", (data) => {
      console.error(`Tonk dev error: ${data}`);
    });

    // Wait for the dev server to start
    return new Promise((resolve, reject) => {
      let isReady = false;
      const timeout = setTimeout(() => {
        if (!isReady) {
          reject(new Error("Timeout waiting for dev server to start"));
        }
      }, 60000); // 60 second timeout

      devProcess.stdout.on("data", (data) => {
        const output = data.toString();
        // Check for webpack dev server ready message
        if (
          output.includes("compiled successfully") ||
          output.includes("Compiled successfully")
        ) {
          clearTimeout(timeout);
          isReady = true;

          const newWindow = new BrowserWindow({
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
          require("@electron/remote/main").enable(newWindow.webContents);

          // Set the window title to match the page title
          newWindow.webContents.on("page-title-updated", (_event, title) => {
            newWindow.setTitle(title);
          });

          // Store a reference to the process for this window
          newWindow.devProcess = devProcess;

          // Clean up when window is closed
          newWindow.on("closed", () => {
            // Kill the dev process when window is closed
            if (newWindow.devProcess) {
              newWindow.devProcess.kill();
            }
          });

          // Update the appWindow reference to the new window
          appWindow = newWindow;

          // Load the dev server URL
          newWindow.loadURL(`http://localhost:${frontendPort}`);

          resolve(true);
        }
      });

      devProcess.on("error", (error) => {
        clearTimeout(timeout);
        console.error("Failed to start tonk dev process:", error);
        reject(error);
      });

      devProcess.on("close", (code) => {
        clearTimeout(timeout);
        if (!isReady) {
          console.log(`Tonk dev process exited with code ${code} before ready`);
          reject(
            new Error(`Tonk dev process exited with code ${code} before ready`),
          );
        }
      });
    });
  } catch (error) {
    console.error("Error launching app in dev mode:", error);
    throw error;
  }
};

ipcMain.handle("launch-app-dev", async (_, projectPath) => {
  try {
    console.log("Launching app in development mode...");

    // Launch the app in development mode
    await launchAppDev(projectPath);
  } catch (e) {
    console.error("Error in launch-app-dev:", e);
    throw e;
  }
});

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
            urlDisplay.textContent = '${url || "http://localhost:7777"}';
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
