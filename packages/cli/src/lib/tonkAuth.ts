import { RESPONSES } from "@/utils/messages";
import { TonkAuth } from "@tonk/tonk-auth";
import chalk from "chalk";
import inquirer from "inquirer";

const tonkAuth = await TonkAuth({
  onSubscriptionDisabled: () => {
    console.log(chalk.red('🚨 Your subscription is inactive, upgrade your subscription to Deploy to Tonk'));
  }
});


/**
 * Checks if the user is logged in and has an active subscription
 */
export function checkAuth(): boolean {
    if (!tonkAuth.isSignedIn) {
      return false;
    }

    const authOk = tonkAuth.isSignedIn && tonkAuth.activeSubscription;
    if (!authOk) {
      return false;
    }
    return true;
}

export const authHook = async() => {
  if (!checkAuth()) {
      console.log(chalk.yellow(RESPONSES.needSubscription));
      const {confirm} = await inquirer.prompt([
            {
              type: 'confirm',
              name: 'confirm',
              message: 'Do you want to login?',
              default: true,
            }
          ]);
      if (!confirm) {
        process.exit(1);
      }
      console.log("\n🗺️ Great, let's sign you in using your browser!")
      const res = await tonkAuth.login();
      if (!res.ok) {
        throw new Error('Failed to login to Tonk' + res.error);
      }
      console.log(chalk.green(`🎉 Welcome back to Tonk ${tonkAuth.friendlyName}\n`));
    }
}

export { tonkAuth };