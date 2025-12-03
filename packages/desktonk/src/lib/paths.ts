/**
 * Centralized configuration for Filesystem Hierarchy Standard (FHS) compliance.
 * This replaces hardcoded paths like '/desktonk', '/opt/', and '/src/stores/'.
 */
export const FHS = {
  // User Data
  // TODO: Migrate to /home/user/Desktop or /srv/desktop when bundling logic is updated
  DESKTOP: '/desktonk',

  // Application State (variable data)
  // Replaces /opt/ and /src/stores/ usage
  STATE_ROOT: '/var/lib/desktonk',

  // Helpers
  getStorePath: (name: string) => `${FHS.STATE_ROOT}/stores/${name}.json`,

  getServicePath: (service: string, file: string) =>
    `${FHS.STATE_ROOT}/${service}/${file}`,
} as const;
