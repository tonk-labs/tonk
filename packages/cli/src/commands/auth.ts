import {Command} from 'commander';
import { tonkAuth } from '../lib/tonkAuth';

export const authCommand = new Command('auth')
  .description(tonkAuth.isSignedIn ? tonkAuth.activeSubscription ? `🚀 You have an active subscription!`: `🌈 You're signed in to Tonk as ${tonkAuth.friendlyName}` : 'Log in to your Tonk account')
  .action(() => {
    authCommand.help();
  });


authCommand.command('login')
  .description('Login to your Tonk account')
  .action(async () => {
    try {
      if (tonkAuth.isSignedIn) {
        console.log(`💌 Welcome back ${tonkAuth.friendlyName}, you're ready to Tonk!`);
        process.exit(0);
      }

      const res = await tonkAuth.login();
      if (!res.ok) {
        throw new Error('Failed to login to Tonk' + res.error);
      }
      console.log(`💌 YAAY! Good to have you ${tonkAuth.friendlyName}!`);
      process.exit(0);
    } catch (error) {
      console.error('Failed to login to Tonk:', error);
      process.exit(1);
    }
  }); 

authCommand.command('logout')
  .description('Logout from your Tonk account')
  .action(async () => {
    try {
      if (!tonkAuth.isSignedIn) {
        console.log(`💌 You're already signed out of Tonk`);
        process.exit(0);
      }
      const name = tonkAuth.friendlyName;
      const res = await tonkAuth.logout();
      if (!res.ok) {
        throw new Error('Failed to logout from Tonk' + res.error);
      }
      console.log(`👋❤️😮‍💨 Ciao ${name}`);
      process.exit(0);
    } catch (error) {
      console.error('Failed to logout from Tonk:', error);
      process.exit(1);
    }
  });

authCommand.command('help')
  .description('Show help for Tonk auth commands')
  .action(() => {
    console.log('Tonk auth commands:');
    console.log('  login - Login to your Tonk account');
    console.log('  logout - Logout from your Tonk account');
  });