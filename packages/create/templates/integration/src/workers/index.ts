import { default as macroFinance } from "./macroFinance";

const runWorkers = async () => {
  // we run all the necessary workers to update the state
  await macroFinance();
};

export default runWorkers;
