const { ipcMain, shell, BrowserWindow } = require("electron");
const { getConfig } = require("../config.js");
const path = require("node:path");
const http = require("http");
const ngrok = require("ngrok");

// Track the app window globally
let appWindow = null;

// Export appWindow for use in other modules
module.exports = { appWindow };

ipcMain.handle("launch-app", async (_, projectPath) => {
    try {
        console.log("Launching app...");

        // Check if ngrok is already connected to port 8080
        let url;
        try {
            // Get all active tunnels and find one for port 8080
            const dummyUrl = await ngrok.connect(0);
            const api = ngrok.getApi();
            const tunnels = await api.listTunnels();

            const existingTunnel = tunnels.tunnels?.find(
                (tunnel) => tunnel.config?.addr === "http://localhost:8080"
            );
            await ngrok.disconnect(dummyUrl);

            if (existingTunnel) {
                url = existingTunnel.public_url;
                console.log("Using existing ngrok connection:", url);
            } else {
                // No active connection for port 8080, create a new one
                console.log("Connecting to ngrok...");
                url = await ngrok.connect(8080);
                console.log("App launched at:", url);
            }
        } catch (error) {
            // Failed to get tunnels or no tunnels exist, create a new one
            console.log("Connecting to ngrok...");
            url = await ngrok.connect(8080);
            console.log("App launched at:", url);
        }

        // Launch the app with the ngrok URL
        await launchApp(projectPath, url);

        // Keep the process running until interrupted
        return url;
    } catch (e) {
        throw e;
    }
});

const launchApp = async (projectPath, ngrokUrl) => {
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
                                    nodeIntegration: false,
                                    contextIsolation: true,
                                },
                            });

                            // Set the window title to match the page title
                            appWindow.webContents.on(
                                "page-title-updated",
                                (event, title) => {
                                    appWindow.setTitle(title);
                                }
                            );

                            // Clean up the window reference when window is closed
                            appWindow.on("closed", () => {
                                appWindow = null;
                            });
                        }

                        appWindow.loadURL("http://localhost:8080");

                        // Display the ngrok URL in the corner of the window
                        appWindow.webContents.on("did-finish-load", () => {
                            appWindow.webContents.executeJavaScript(`
                                (function() {
                                    const urlDisplay = document.createElement('div');
                                    urlDisplay.textContent = '${
                                        ngrokUrl || "http://localhost:8080"
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
                            new Error(
                                `Failed to set distPath: Status ${res.statusCode}`
                            )
                        );
                    }
                }
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
ipcMain.handle("open-external-link", async (event, link) => {
    // Open links in the user's default browser
    await shell.openExternal(link);
    return true;
});
