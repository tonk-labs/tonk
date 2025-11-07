import { defineConfig, devices } from '@playwright/test';

export default defineConfig({
  globalSetup: './global-setup.ts',
  testDir: './tests',
  timeout: 600000, // 10 min per test
  expect: {
    timeout: 10000,
  },
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: 1,

  use: {
    baseURL: 'http://localhost:5173',
    trace: 'on-first-retry',
    video: 'retain-on-failure',
    screenshot: 'only-on-failure',

    // Launch options for better memory monitoring
    launchOptions: {
      args: [
        '--enable-precise-memory-info',
        '--js-flags=--expose-gc',
        '--disable-dev-shm-usage',
        '--no-sandbox',
      ],
    },
  },

  projects: [
    {
      name: 'bundles',
      testMatch: /bundles\/.*\.spec\.ts$/,
      use: {
        ...devices['Desktop Chrome'],
        viewport: { width: 1920, height: 1080 },
      },
    },
    {
      name: 'stress',
      testMatch: /stress\/.*\.spec\.ts$/,
      use: {
        ...devices['Desktop Chrome'],
        viewport: { width: 1920, height: 1080 },
      },
    },
    {
      name: 'sync',
      testMatch: /sync\/.*\.spec\.ts$/,
      use: {
        ...devices['Desktop Chrome'],
        viewport: { width: 1920, height: 1080 },
      },
    },
    {
      name: 'uptime',
      testMatch: /uptime\/.*\.spec\.ts$/,
      timeout: 3600000,
      use: {
        ...devices['Desktop Chrome'],
        viewport: { width: 1920, height: 1080 },
      },
    },
    {
      name: 'distributed',
      testMatch: /distributed\/.*\.spec\.ts$/,
      timeout: 86400000,
      use: {
        ...devices['Desktop Chrome'],
        viewport: { width: 1920, height: 1080 },
      },
    },
  ],

  reporter: [['html'], ['json', { outputFile: 'test-results.json' }], ['list']],

  webServer: {
    command: 'npm run dev',
    port: 5173,
    reuseExistingServer: !process.env.CI,
    timeout: 120000,
  },
});
