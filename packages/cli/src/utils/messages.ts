import chalk from "chalk";

const randomColor = () => {
  return [
    chalk.redBright,
    chalk.cyanBright,
    chalk.yellowBright,
    chalk.blueBright,
    chalk.magentaBright,
    chalk.greenBright,
  ].sort(() => 0.5 - Math.random())[0];
}

const rainbowTonk =  () => chalk.italic(`${randomColor()("T")}${randomColor()("o")}${randomColor()("n")}${randomColor()("k")}`);

const RESPONSES = {
  rainbowTonk,
  needSubscription: `💬 Hey bestie, you need an active subscription to deploy to ${rainbowTonk()}!`,
}

export { RESPONSES };
