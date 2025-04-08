const { ipcMain } = require("electron");
const { getConfig } = require("../config.js");
const path = require("node:path");
const { spawn } = require("child_process");
const fs = require("fs-extra");
const which = require("which");

ipcMain.handle("install-integration", async (event, integrationName) => {
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
            const npmProcess = spawn(npmPath, ["install"], {
                cwd: integrationsPath,
                stdio: ["ignore", "pipe", "pipe"],
                env: process.env,
                shell: false,
            });

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
            return { success: true, data: [] };
        }

        // Get all directories in the integrations folder
        const integrationDirs = await fs.readdir(integrationsPath);
        const installedIntegrations = [];

        // Check each directory for a package.json
        for (const dir of integrationDirs) {
            const packageJsonPath = path.join(
                integrationsPath,
                dir,
                "package.json"
            );

            if (await fs.pathExists(packageJsonPath)) {
                try {
                    const packageJson = await fs.readJson(packageJsonPath);
                    installedIntegrations.push({
                        name: dir,
                        version: packageJson.version,
                        description: packageJson.description,
                        dependencies: packageJson.dependencies || {},
                        devDependencies: packageJson.devDependencies || {},
                    });
                } catch (err) {
                    console.warn(`Error reading package.json for ${dir}:`, err);
                }
            }
        }

        return { success: true, data: installedIntegrations };
    } catch (error) {
        console.error("Failed to get installed integrations:", error);
        return { success: false, error: error.message };
    }
});
