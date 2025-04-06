import { configureSyncEngine } from "@tonk/keepsync";
import runWorkers from "./workers";

configureSyncEngine({
  url: "ws://localhost:8080/sync",
  name: "integrations-starter-template",
});

export const run = async () => {
  runWorkers();
};

run();
