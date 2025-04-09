const { ipcMain, shell } = require("electron");
const { getConfig } = require("../config.js");
const path = require("node:path");
const util = require("node:util");
const child_process = require("node:child_process");
const http = require("node:http");

const exec = util.promisify(child_process.exec);

ipcMain.handle("launch-app", async (event, projectPath) => {
    try {
        const isAppRunning = await checkAppStatus();
        if (isAppRunning) {
            await shell.openExternal("http://localhost:8080");
            return true;
        }
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

        await shell.openExternal("http://localhost:8080");

        return true;
    } catch (error) {
        // The awaited exec will throw an error if the command fails (non-zero exit code)
        console.error(`Error launching app: ${error.stderr || error.message}`);
        throw new Error(error.stderr || error.message);
    }
});

ipcMain.handle("stop-and-reset", async () => {
    await exec(`pkill -f "tonk serve"`);
});

ipcMain.handle("open-external-link", async (event, link) => {
    await shell.openExternal(link);
});

ipcMain.handle("is-app-running", async () => {
    return await checkAppStatus();
});

const checkAppStatus = async () => {
    return new Promise((resolve) => {
        const req = http.get("http://localhost:8080", (res) => {
            // any response signifies the server is active
            resolve(true);
            res.resume(); // ensure response data is consumed
        });

        req.on("error", (err) => {
            // typically econnrefused if the server is down
            // console.warn(`app status check failed: ${err.message}`); // optional: more detailed logging
            resolve(false);
        });

        // prevent indefinite hangs
        req.setTimeout(1000, () => {
            req.destroy();
            // console.warn('app status check timed out.'); // optional: timeout logging
            resolve(false);
        });
    });
};
