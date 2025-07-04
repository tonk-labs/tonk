import {Command} from 'commander';
import chalk from 'chalk';
import {
  trackCommand,
  trackCommandError,
  trackCommandSuccess,
} from '../utils/analytics.js';

// Lazy-load tonkAuth to prevent initialization on import
async function getTonkAuth() {
  const {tonkAuth} = await import('../lib/tonkAuth.js');
  return tonkAuth;
}

const getAuthDescription = async () => {
  try {
    const tonkAuth = await getTonkAuth();
    if (!tonkAuth.isReady) return 'Log in to your Tonk account';
    if (tonkAuth.isSignedIn && tonkAuth.activeSubscription)
      return '🚀 You have an active subscription!';
    if (tonkAuth.isSignedIn)
      return `🌈 You're signed in to Tonk as ${tonkAuth.friendlyName}`;
    return 'Log in to your Tonk account';
  } catch {
    return 'Log in to your Tonk account';
  }
};

export const authCommand = new Command('auth')
  .description('Manage your Tonk authentication')
  .action(async () => {
    // Update description dynamically when auth command is actually used
    authCommand.description(await getAuthDescription());
    authCommand.help();
  });

authCommand
  .command('login')
  .description('Login to your Tonk account')
  .action(async () => {
    const startTime = Date.now();
    let tonkAuth: any = null;

    try {
      trackCommand('auth-login', {});

      tonkAuth = await getTonkAuth();
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
        await tonkAuth.destroy();
        return;
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
        await tonkAuth.destroy();
        process.exitCode = 1;
        return;
      }

      console.log(`💌 YAAY! Good to have you ${tonkAuth.friendlyName}!`);

      const duration = Date.now() - startTime;
      trackCommandSuccess('auth-login', duration, {
        wasAlreadySignedIn: false,
        hasActiveSubscription: tonkAuth.activeSubscription,
        userName: tonkAuth.friendlyName,
      });
      await tonkAuth.destroy();
    } catch (error) {
      const duration = Date.now() - startTime;
      trackCommandError('auth-login', error as Error, duration, {
        errorType: 'unexpected_error',
      });
      console.error(
        chalk.red('An unexpected error occurred during login:'),
        error,
      );
      if (tonkAuth) {
        await tonkAuth.destroy();
      }
      process.exitCode = 1;
    }
  });

authCommand
  .command('logout')
  .description('Logout from your Tonk account')
  .action(async () => {
    const startTime = Date.now();
    let tonkAuth: any = null;

    try {
      trackCommand('auth-logout', {});

      tonkAuth = await getTonkAuth();
      await tonkAuth.ensureReady();

      if (!tonkAuth.isSignedIn) {
        console.log(`💌 You're already signed out of Tonk`);

        const duration = Date.now() - startTime;
        trackCommandSuccess('auth-logout', duration, {
          wasAlreadySignedOut: true,
        });
        await tonkAuth.destroy();
        return;
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
        await tonkAuth.destroy();
        process.exitCode = 1;
        return;
      }

      console.log(`👋❤️😮‍💨 Ciao ${name}`);

      const duration = Date.now() - startTime;
      trackCommandSuccess('auth-logout', duration, {
        wasAlreadySignedOut: false,
        userName: name,
        hadActiveSubscription,
      });
      await tonkAuth.destroy();
    } catch (error) {
      const duration = Date.now() - startTime;
      trackCommandError('auth-logout', error as Error, duration, {
        errorType: 'unexpected_error',
      });
      console.error(
        chalk.red('An unexpected error occurred during logout:'),
        error,
      );
      if (tonkAuth) {
        await tonkAuth.destroy();
      }
      process.exitCode = 1;
    }
  });

authCommand
  .command('status')
  .description('Show your authentication status')
  .action(async () => {
    const startTime = Date.now();
    let tonkAuth: any = null;

    try {
      trackCommand('auth-status', {});

      tonkAuth = await getTonkAuth();
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

      await tonkAuth.destroy();
    } catch (error) {
      const duration = Date.now() - startTime;
      trackCommandError('auth-status', error as Error, duration, {
        errorType: 'unexpected_error',
      });
      console.error(
        chalk.red('An unexpected error occurred while checking status:'),
        error,
      );
      if (tonkAuth) {
        await tonkAuth.destroy();
      }
      process.exitCode = 1;
    }
  });
