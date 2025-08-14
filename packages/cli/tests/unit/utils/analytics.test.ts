import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

// Create proper mock for PostHog
const mockPostHogInstance = {
  capture: vi.fn(),
  shutdown: vi.fn().mockResolvedValue(undefined),
};

const MockPostHog = vi.fn(() => mockPostHogInstance);

// Mock PostHog
vi.mock('posthog-node', () => ({
  PostHog: MockPostHog,
}));

// Mock fs
vi.mock('fs', () => ({
  existsSync: vi.fn(),
}));

// Mock os
vi.mock('os', () => ({
  default: {
    hostname: vi.fn(() => 'test-machine'),
    platform: vi.fn(() => 'linux'),
    arch: vi.fn(() => 'x64'),
    userInfo: vi.fn(() => ({ username: 'testuser' })),
  },
}));

// Mock node-machine-id (optional dependency)
vi.mock('node-machine-id', () => ({
  machineIdSync: vi.fn(() => 'test-machine-id'),
}));

// Mock environment config
const mockGetAnalyticsConfig = vi.fn(() => ({
  enabled: true,
  apiKey: 'test-api-key',
  host: 'https://app.posthog.com',
}));

vi.mock('../../../src/config/environment.js', () => ({
  getAnalyticsConfig: mockGetAnalyticsConfig,
}));

// Import after mocks
const {
  trackCommand,
  trackCommandSuccess,
  trackCommandError,
  initAnalytics,
  shutdownAnalytics,
  __resetAnalyticsForTesting,
} = await import('../../../src/utils/analytics.js');

describe('analytics utils', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // Reset the singleton state for clean test isolation
    __resetAnalyticsForTesting();
    // Reset the analytics state
    vi.unstubAllEnvs();
    vi.stubEnv('DISABLE_ANALYTICS', '');
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  describe('initAnalytics', () => {
    it('should initialize PostHog when analytics is enabled', () => {
      const analytics = initAnalytics();
      expect(analytics).toBeDefined();
      expect(MockPostHog).toHaveBeenCalledWith('test-api-key', {
        host: 'https://app.posthog.com',
      });
    });

    it('should return null when analytics is disabled via environment config', () => {
      mockGetAnalyticsConfig.mockReturnValueOnce({
        enabled: false,
        apiKey: 'test-api-key',
        host: 'https://app.posthog.com',
      });

      const analytics = initAnalytics();
      expect(analytics).toBeNull();
    });

    it('should return null when no API key is provided', () => {
      mockGetAnalyticsConfig.mockReturnValueOnce({
        enabled: true,
        apiKey: '',
        host: 'https://app.posthog.com',
      });

      const analytics = initAnalytics();
      expect(analytics).toBeNull();
    });
  });

  describe('trackCommand', () => {
    it('should track command execution with basic properties', () => {
      trackCommand('test-command', { arg1: 'value1' });

      expect(mockPostHogInstance.capture).toHaveBeenCalledWith({
        distinctId: expect.any(String),
        event: 'command_executed',
        properties: expect.objectContaining({
          command: 'test-command',
          options: { arg1: 'value1' },
          platform: 'linux',
          arch: 'x64',
        }),
      });
    });

    it('should handle analytics tracking errors gracefully', () => {
      mockPostHogInstance.capture.mockImplementationOnce(() => {
        throw new Error('Analytics error');
      });

      const consoleSpy = vi
        .spyOn(console, 'debug')
        .mockImplementation(() => {});

      expect(() => trackCommand('test-command')).not.toThrow();
      expect(consoleSpy).toHaveBeenCalledWith(
        'Analytics tracking failed:',
        expect.any(Error)
      );
    });

    it('should not track when analytics is disabled', () => {
      mockGetAnalyticsConfig.mockReturnValueOnce({
        enabled: false,
        apiKey: 'test-api-key',
        host: 'https://app.posthog.com',
      });

      // Clear previous calls
      mockPostHogInstance.capture.mockClear();

      trackCommand('test-command');

      expect(mockPostHogInstance.capture).not.toHaveBeenCalled();
    });
  });

  describe('trackCommandSuccess', () => {
    it('should track command success with duration', () => {
      trackCommandSuccess('test-command', 1500);

      expect(mockPostHogInstance.capture).toHaveBeenCalledWith({
        distinctId: expect.any(String),
        event: 'command_success',
        properties: expect.objectContaining({
          command: 'test-command',
          duration_ms: 1500,
        }),
      });
    });
  });

  describe('trackCommandError', () => {
    it('should track command error with Error object', () => {
      const error = new Error('Test error');
      trackCommandError('test-command', error, 2000);

      expect(mockPostHogInstance.capture).toHaveBeenCalledWith({
        distinctId: expect.any(String),
        event: 'command_error',
        properties: expect.objectContaining({
          command: 'test-command',
          error: 'Test error',
          error_type: 'Error',
          error_stack: expect.any(String),
          duration_ms: 2000,
        }),
      });
    });

    it('should track command error with string message', () => {
      trackCommandError('test-command', 'String error message');

      expect(mockPostHogInstance.capture).toHaveBeenCalledWith({
        distinctId: expect.any(String),
        event: 'command_error',
        properties: expect.objectContaining({
          command: 'test-command',
          error: 'String error message',
          error_type: 'string',
          error_stack: undefined,
        }),
      });
    });
  });

  describe('shutdownAnalytics', () => {
    it('should shutdown PostHog instance gracefully', async () => {
      // Initialize analytics first
      initAnalytics();

      await shutdownAnalytics();

      expect(mockPostHogInstance.shutdown).toHaveBeenCalled();
    });

    it('should handle shutdown errors gracefully', async () => {
      mockPostHogInstance.shutdown.mockRejectedValueOnce(
        new Error('Shutdown error')
      );

      const consoleSpy = vi
        .spyOn(console, 'debug')
        .mockImplementation(() => {});

      // Initialize analytics first
      initAnalytics();

      await expect(shutdownAnalytics()).resolves.not.toThrow();
      expect(consoleSpy).toHaveBeenCalledWith(
        'Analytics shutdown failed:',
        expect.any(Error)
      );
    });
  });
});
