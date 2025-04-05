import { configureSyncEngine } from "@tonk/keepsync";
import runWorkers from "./workers";

configureSyncEngine({
  url: "ws://localhost:4080",
  name: "integrations-starter-template",
});

export const run = async () => {
  runWorkers();
};
