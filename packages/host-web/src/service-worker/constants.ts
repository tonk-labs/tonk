// Build-time constants injected by bundler
declare const TONK_SERVER_URL: string;
declare const TONK_SERVE_LOCAL: boolean;
declare const __SW_BUILD_TIME__: string;
declare const __SW_VERSION__: string;

// Re-export build-time constants
export const SERVER_URL = TONK_SERVER_URL;
export const SERVE_LOCAL = TONK_SERVE_LOCAL;
export const SW_BUILD_TIME = __SW_BUILD_TIME__;
export const SW_VERSION = __SW_VERSION__;

// Configuration constants
export const DEBUG_LOGGING = true;
export const HEALTH_CHECK_INTERVAL = 5000;
export const MAX_RECONNECT_ATTEMPTS = 10;
export const CONTINUOUS_RETRY_ENABLED = true;
export const PATH_INDEX_SYNC_TIMEOUT = 1000;

// Cache API constants
export const CACHE_NAME = 'tonk-sw-state-v1';
export const APP_SLUG_URL = '/tonk-state/appSlug';
export const BUNDLE_BYTES_URL = '/tonk-state/bundleBytes';

// Dev server configuration
export const DEV_SERVER_PORT = 4001;
export const DEV_SERVER_URL = `http://localhost:${DEV_SERVER_PORT}`;
