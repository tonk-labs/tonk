const { ipcMain, shell, BrowserWindow } = require("electron");
const { getConfig } = require("../config.js");
const path = require("node:path");
const http = require("http");

ipcMain.handle("launch-app", async (event, projectPath) => {
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
                        // Create a new browser window instead of opening externally
                        const appWindow = new BrowserWindow({
                            width: 1024,
                            height: 800,
                            title: "",
                            webPreferences: {
                                nodeIntegration: false,
                                contextIsolation: true,
                            },
                        });

                        appWindow.loadURL("http://localhost:8080");

                        // Set the window title to match the page title
                        appWindow.webContents.on(
                            "page-title-updated",
                            (event, title) => {
                                appWindow.setTitle(title);
                            }
                        );

                        // Clean up the window reference when window is closed
                        appWindow.on("closed", () => {
                            // Optional: track window state if needed
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
});

ipcMain.handle("open-external-link", async (event, link) => {
    // Also modify this handler to offer opening in a new window
    const appWindow = new BrowserWindow({
        width: 1200,
        height: 800,
        title: "",
        webPreferences: {
            nodeIntegration: false,
            contextIsolation: true,
        },
    });

    appWindow.loadURL(link);

    // Set the window title to match the page title
    appWindow.webContents.on("page-title-updated", (event, title) => {
        appWindow.setTitle(title);
    });

    return true;
});
