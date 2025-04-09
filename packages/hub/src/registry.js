const https = require("https");

async function fetchRegistry() {
    return new Promise((resolve, reject) => {
        const options = {
            hostname: "raw.githubusercontent.com",
            path: "/tonk-labs/registry/main/registry.json",
            method: "GET",
            headers: {
                "User-Agent": "Tonk-Hub",
            },
        };

        const req = https.request(options, (res) => {
            let data = "";

            res.on("data", (chunk) => {
                data += chunk;
            });

            res.on("end", () => {
                try {
                    const registry = JSON.parse(data);
                    resolve(registry);
                } catch (err) {
                    reject(new Error("Failed to parse registry data"));
                }
            });
        });

        req.on("error", (error) => {
            reject(error);
        });

        req.end();
    });
}

module.exports = {
    fetchRegistry,
};
