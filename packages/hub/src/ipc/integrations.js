const { ipcMain } = require("electron");
const { getConfig } = require("../config.js");
const path = require("node:path");
const { spawn } = require("child_process");
const fs = require("fs-extra");
const which = require("which");

ipcMain.handle("install-integration", async (event, integrationLink) => {
    try {
        const config = getConfig();
        const integrationsPath = path.join(config.homePath, "integrations");

        // Ensure the integration directory exists
        if (!fs.existsSync(integrationsPath)) {
            throw new Error(
                `Integration directory not found: ${integrationsPath}`
            );
        }

        console.log("Installing in directory:", integrationsPath);
        console.log("Current PATH:", process.env.PATH);

        // Get npm location
        const npmPath =
            process.platform === "win32"
                ? which.sync("npm.cmd", { nothrow: true })
                : which.sync("npm", { nothrow: true });

        if (!npmPath) {
            throw new Error("npm executable not found in PATH");
        }

        console.log("Using npm from:", npmPath);

        // Create a promise to handle the npm install process
        return new Promise((resolve, reject) => {
            const npmProcess = spawn(
                npmPath,
                ["install", "-y", integrationLink],
                {
                    cwd: integrationsPath,
                    stdio: ["ignore", "pipe", "pipe"],
                    env: process.env,
                    shell: false,
                }
            );

            let stdout = "";
            let stderr = "";

            npmProcess.stdout.on("data", (data) => {
                const output = data.toString();
                console.log("npm stdout:", output);
                stdout += output;
            });

            npmProcess.stderr.on("data", (data) => {
                const output = data.toString();
                console.error("npm stderr:", output);
                stderr += output;
            });

            npmProcess.on("error", (error) => {
                console.error("Failed to start npm process:", error);
                reject({
                    success: false,
                    error: `Failed to start npm: ${error.message}`,
                });
            });

            npmProcess.on("close", (code) => {
                if (code === 0) {
                    console.log("Installation successful");
                    resolve({ success: true, data: stdout });
                } else {
                    console.error("Installation failed with code:", code);
                    console.error("stderr:", stderr);
                    reject({
                        success: false,
                        error:
                            stderr || `Installation failed with code ${code}`,
                    });
                }
            });
        });
    } catch (error) {
        console.error("Failed to install integration:", error);
        return { success: false, error: error.message };
    }
});

ipcMain.handle("get-installed-integrations", async () => {
    try {
        const config = getConfig();
        const integrationsPath = path.join(config.homePath, "integrations");

        // Ensure integrations directory exists
        if (!fs.existsSync(integrationsPath)) {
            return { success: false, data: [] };
        }
        const installedIntegrations = [];

        const packageJsonPath = path.join(integrationsPath, "package.json");
        // Check each directory for a package.json

        if (!(await fs.pathExists(packageJsonPath))) {
            return { success: false, data: [] };
        }
        const dependencies =
            (await fs.readJson(packageJsonPath)).dependencies ?? {};
        for (const [name, version] of Object.entries(dependencies)) {
            installedIntegrations.push({
                name,
                version,
            });
        }

        return { success: true, data: installedIntegrations };
    } catch (error) {
        console.error("Failed to get installed integrations:", error);
        return { success: false, error: error.message, data: [] };
    }
});
