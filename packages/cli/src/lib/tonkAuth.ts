import {RESPONSES} from '../utils/messages.js';
import {TonkAuth} from '@tonk/tonk-auth';
import chalk from 'chalk';
import inquirer from 'inquirer';
import {shutdownAnalytics} from '../utils/analytics.js';

// TonkAuth wrapped in a Singleton class with (non-blocking) async initilization keeps the CLI load snappy 🐢
class TonkAuthManager {
  private instance: any = null;
  private initPromise: Promise<any> | null = null;
  private _isReady = false;

  constructor() {
    this.initialize();
  }

  private async initialize() {
    if (this.initPromise) return this.initPromise;

    this.initPromise = TonkAuth({
      onSubscriptionDisabled: () => {
        console.log(
          chalk.red(
            '🚨 Your subscription is inactive, upgrade your subscription to Deploy to Tonk',
          ),
        );
      },
    })
      .then(auth => {
        this.instance = auth;
        this._isReady = true;
        return auth;
      })
      .catch(error => {
        console.error(chalk.red('Failed to initialize auth:'), error);
        this._isReady = false;
        return null;
      });

    return this.initPromise;
  }

  get isSignedIn(): boolean {
    return this.instance?.isSignedIn ?? false;
  }

  get activeSubscription(): boolean {
    return this.instance?.activeSubscription ?? false;
  }

  get friendlyName(): string {
    return this.instance?.friendlyName ?? '';
  }

  get isReady(): boolean {
    return this._isReady;
  }

  async ensureReady(): Promise<void> {
    if (!this._isReady && this.initPromise) {
      await this.initPromise;
    }
  }

  async login() {
    await this.ensureReady();
    if (!this.instance) return {ok: false, error: 'Auth not initialized'};

    try {
      return await this.instance.login();
    } catch (error) {
      return {ok: false, error: String(error)};
    }
  }

  async logout() {
    await this.ensureReady();
    if (!this.instance) return {ok: false, error: 'Auth not initialized'};

    try {
      return await this.instance.logout();
    } catch (error) {
      return {ok: false, error: String(error)};
    }
  }

  async getAuthToken(): Promise<string | null> {
    await this.ensureReady();
    if (!this.instance?.__session) return null;
    
    try {
      return await this.instance.__session.getToken();
    } catch (error) {
      console.error('Failed to get auth token:', error);
      return null;
    }
  }

  async destroy(): Promise<void> {
    if (this.instance && typeof this.instance.destroy === 'function') {
      this.instance.destroy();
    }
    await shutdownAnalytics();
  }
}

export const tonkAuth = new TonkAuthManager();

export function checkAuth(): boolean {
  return tonkAuth.isSignedIn && tonkAuth.activeSubscription;
}

export const authHook = async () => {
  // Ensure auth is initialized
  await tonkAuth.ensureReady();

  if (checkAuth()) return;

  // Check if user is signed in but doesn't have active subscription
  if (tonkAuth.isSignedIn && !tonkAuth.activeSubscription) {
    console.log(chalk.yellow(RESPONSES.needSubscription));
    console.log(chalk.yellow('Please upgrade your subscription to use Tonk hosting features.'));
    await tonkAuth.destroy();
    process.exit(1);
  }

  // User is not signed in at all
  if (!tonkAuth.isSignedIn) {
    console.log(chalk.yellow('You need to be signed in to use Tonk hosting features.'));
    const {confirm} = await inquirer.prompt([
      {
        type: 'confirm',
        name: 'confirm',
        message: 'Do you want to login?',
        default: true,
      },
    ]);

    if (!confirm) {
      console.log(chalk.yellow('Authentication required. Exiting.'));
      await tonkAuth.destroy();
      process.exit(1);
    }

    console.log("\n🗺️ Great, let's sign you in using your browser!");
    const res = await tonkAuth.login();

    if (!res.ok) {
      console.error(
        chalk.red(`Failed to login to Tonk: ${res.error || 'Unknown error'}`),
      );
      await tonkAuth.destroy();
      process.exit(1);
    }

    console.log(
      chalk.green(
        `🎉 Welcome back to Tonk ${tonkAuth.friendlyName || 'user'}!\n`,
      ),
    );

    // Check subscription status after login
    if (!tonkAuth.activeSubscription) {
      console.log(chalk.yellow('Please upgrade your subscription to use Tonk hosting features.'));
      await tonkAuth.destroy();
      process.exit(1);
    }
  }
};

