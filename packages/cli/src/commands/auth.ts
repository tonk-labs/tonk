import {Command} from 'commander';
import {tonkAuth} from '../lib/tonkAuth.js';
import chalk from 'chalk';
import {
  trackCommand,
  trackCommandError,
  trackCommandSuccess,
  shutdownAnalytics,
} from '../utils/analytics.js';

const getAuthDescription = () => {
  if (!tonkAuth.isReady) return 'Log in to your Tonk account';
  if (tonkAuth.isSignedIn && tonkAuth.activeSubscription)
    return '🚀 You have an active subscription!';
  if (tonkAuth.isSignedIn)
    return `🌈 You're signed in to Tonk as ${tonkAuth.friendlyName}`;
  return 'Log in to your Tonk account';
};

export const authCommand = new Command('auth')
  .description(getAuthDescription())
  .action(() => {
    authCommand.help();
  });

authCommand
  .command('login')
  .description('Login to your Tonk account')
  .action(async () => {
    const startTime = Date.now();

    try {
      trackCommand('auth-login', {});

      await tonkAuth.ensureReady();

      if (tonkAuth.isSignedIn) {
        console.log(
          `💌 Welcome back ${tonkAuth.friendlyName}, you're ready to Tonk!`,
        );

        const duration = Date.now() - startTime;
        trackCommandSuccess('auth-login', duration, {
          wasAlreadySignedIn: true,
          hasActiveSubscription: tonkAuth.activeSubscription,
          userName: tonkAuth.friendlyName,
        });
        await shutdownAnalytics();
        process.exit(0);
      }

      console.log("\n🗺️ Great, let's sign you in using your browser!");
      const res = await tonkAuth.login();

      if (!res.ok) {
        const duration = Date.now() - startTime;
        trackCommandError(
          'auth-login',
          new Error(res.error || 'Unknown login error'),
          duration,
          {
            wasAlreadySignedIn: false,
            errorType: 'login_failed',
          },
        );
        console.error(
          chalk.red(`Failed to login to Tonk: ${res.error || 'Unknown error'}`),
        );
        await shutdownAnalytics();
        process.exit(1);
      }

      console.log(`💌 YAAY! Good to have you ${tonkAuth.friendlyName}!`);

      const duration = Date.now() - startTime;
      trackCommandSuccess('auth-login', duration, {
        wasAlreadySignedIn: false,
        hasActiveSubscription: tonkAuth.activeSubscription,
        userName: tonkAuth.friendlyName,
      });
      await shutdownAnalytics();
      process.exit(0);
    } catch (error) {
      const duration = Date.now() - startTime;
      trackCommandError('auth-login', error as Error, duration, {
        errorType: 'unexpected_error',
      });
      console.error(
        chalk.red('An unexpected error occurred during login:'),
        error,
      );
      await shutdownAnalytics();
      process.exit(1);
    }
  });

authCommand
  .command('logout')
  .description('Logout from your Tonk account')
  .action(async () => {
    const startTime = Date.now();

    try {
      trackCommand('auth-logout', {});

      await tonkAuth.ensureReady();

      if (!tonkAuth.isSignedIn) {
        console.log(`💌 You're already signed out of Tonk`);

        const duration = Date.now() - startTime;
        trackCommandSuccess('auth-logout', duration, {
          wasAlreadySignedOut: true,
        });
        await shutdownAnalytics();
        process.exit(0);
      }

      const name = tonkAuth.friendlyName;
      const hadActiveSubscription = tonkAuth.activeSubscription;
      const res = await tonkAuth.logout();

      if (!res.ok) {
        const duration = Date.now() - startTime;
        trackCommandError(
          'auth-logout',
          new Error(res.error || 'Unknown logout error'),
          duration,
          {
            wasSignedIn: true,
            userName: name,
            hadActiveSubscription,
            errorType: 'logout_failed',
          },
        );
        console.error(
          chalk.red(
            `Failed to logout from Tonk: ${res.error || 'Unknown error'}`,
          ),
        );
        await shutdownAnalytics();
        process.exit(1);
      }

      console.log(`👋❤️😮‍💨 Ciao ${name}`);

      const duration = Date.now() - startTime;
      trackCommandSuccess('auth-logout', duration, {
        wasAlreadySignedOut: false,
        userName: name,
        hadActiveSubscription,
      });
      await shutdownAnalytics();
      process.exit(0);
    } catch (error) {
      const duration = Date.now() - startTime;
      trackCommandError('auth-logout', error as Error, duration, {
        errorType: 'unexpected_error',
      });
      console.error(
        chalk.red('An unexpected error occurred during logout:'),
        error,
      );
      await shutdownAnalytics();
      process.exit(1);
    }
  });

authCommand
  .command('status')
  .description('Show your authentication status')
  .action(async () => {
    const startTime = Date.now();

    try {
      trackCommand('auth-status', {});

      await tonkAuth.ensureReady();

      if (tonkAuth.isSignedIn) {
        console.log(`✅ Signed in as ${tonkAuth.friendlyName}`);
        console.log(
          `📦 Subscription: ${tonkAuth.activeSubscription ? 'Active' : 'Inactive'}`,
        );

        const duration = Date.now() - startTime;
        trackCommandSuccess('auth-status', duration, {
          isSignedIn: true,
          hasActiveSubscription: tonkAuth.activeSubscription,
          userName: tonkAuth.friendlyName,
        });
      } else {
        console.log('❌ Not signed in');

        const duration = Date.now() - startTime;
        trackCommandSuccess('auth-status', duration, {
          isSignedIn: false,
          hasActiveSubscription: false,
        });
      }

      await shutdownAnalytics();
      process.exit(0);
    } catch (error) {
      const duration = Date.now() - startTime;
      trackCommandError('auth-status', error as Error, duration, {
        errorType: 'unexpected_error',
      });
      console.error(
        chalk.red('An unexpected error occurred while checking status:'),
        error,
      );
      await shutdownAnalytics();
      process.exit(1);
    }
  });
