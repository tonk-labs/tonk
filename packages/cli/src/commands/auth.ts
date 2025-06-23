import {Command} from 'commander';
import { tonkAuth } from '../lib/tonkAuth';
import chalk from 'chalk';

const getAuthDescription = () => {
  if (!tonkAuth.isReady) return 'Log in to your Tonk account';
  if (tonkAuth.isSignedIn && tonkAuth.activeSubscription) return '🚀 You have an active subscription!';
  if (tonkAuth.isSignedIn) return `🌈 You're signed in to Tonk as ${tonkAuth.friendlyName}`;
  return 'Log in to your Tonk account';
};

export const authCommand = new Command('auth')
  .description(getAuthDescription())
  .action(() => {
    authCommand.help();
  });

authCommand.command('login')
  .description('Login to your Tonk account')
  .action(async () => {
    await tonkAuth.ensureReady();
    
    if (tonkAuth.isSignedIn) {
      console.log(`💌 Welcome back ${tonkAuth.friendlyName}, you're ready to Tonk!`);
      process.exit(0);
    }
    
    console.log("\n🗺️ Great, let's sign you in using your browser!");
    const res = await tonkAuth.login();
    if (!res.ok) {
      console.error(chalk.red(`Failed to login to Tonk: ${res.error || 'Unknown error'}`));
      process.exit(1);
    }
    console.log(`💌 YAAY! Good to have you ${tonkAuth.friendlyName}!`);
    process.exit(0);
  }); 

authCommand.command('logout')
  .description('Logout from your Tonk account')
  .action(async () => {
    await tonkAuth.ensureReady();
    
    if (!tonkAuth.isSignedIn) {
      console.log(`💌 You're already signed out of Tonk`);
      process.exit(0);
    }
    
    const name = tonkAuth.friendlyName;
    const res = await tonkAuth.logout();
    if (!res.ok) {
      console.error(chalk.red(`Failed to logout from Tonk: ${res.error || 'Unknown error'}`));
      process.exit(1);
    }
    console.log(`👋❤️😮‍💨 Ciao ${name}`);
    process.exit(0);
  });

authCommand.command('status')
  .description('Show your authentication status')
  .action(async () => {
    await tonkAuth.ensureReady();
    
    if (tonkAuth.isSignedIn) {
      console.log(`✅ Signed in as ${tonkAuth.friendlyName}`);
      console.log(`📦 Subscription: ${tonkAuth.activeSubscription ? 'Active' : 'Inactive'}`);
    } else {
      console.log('❌ Not signed in');
    }
    process.exit(0);
  });