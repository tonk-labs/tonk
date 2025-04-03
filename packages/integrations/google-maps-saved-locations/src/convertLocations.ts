import { convertLocations } from "./convert";
import * as dotenv from "dotenv";

dotenv.config();

async function main() {
  try {
    await convertLocations();
  } catch (error) {
    console.error(error);
    process.exit(1);
  }
}

if (require.main === module) {
  main().catch(console.error);
}
