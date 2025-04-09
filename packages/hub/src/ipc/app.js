const { ipcMain, shell } = require("electron");
const { getConfig } = require("../config.js");
const path = require("node:path");
const util = require("node:util");
const child_process = require("node:child_process");

const exec = util.promisify(child_process.exec);

ipcMain.handle("launch-app", async (event, projectPath) => {
    try {
        // Launch the app with the docId as a query parameter
        // You'll need to adjust this based on how your app is actually launched
        // Assuming you're using npm start or similar to launch the app
        let config = getConfig();
        let storesPath = path.join(config.homePath, "stores");
        console.log({ projectPath });

        // Execute the command and wait for it to finish
        const { stdout, stderr } = await exec(
            `cd "${projectPath}" && tonk serve -f ${storesPath}`
        );

        if (stderr) {
            // Handle potential errors written to stderr, even if the command exits successfully
            console.warn(`Command stderr: ${stderr}`);
        }
        console.log(`Command stdout: ${stdout}`);

        shell.openExternal("http://localhost:8080");

        return true;
    } catch (error) {
        // The awaited exec will throw an error if the command fails (non-zero exit code)
        console.error(`Error launching app: ${error.stderr || error.message}`);
        throw new Error(error.stderr || error.message);
    }
});

ipcMain.handle("open-external-link", async (event, link) => {
    shell.openExternal(link);
});
