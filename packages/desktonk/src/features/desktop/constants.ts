/**
 * Desktop file system paths.
 * Single source of truth for all desktop-related VFS paths.
 */

/**
 * Directory where user desktop files are stored.
 */
export const DESKTOP_DIRECTORY = '/desktonk';

/**
 * Directory where icon position files are stored.
 */
export const LAYOUT_DIRECTORY = '/var/lib/desktonk/layout';

/**
 * Directory where thumbnail images are stored separately from file documents.
 * This reduces shape size and prevents large writes on drag operations.
 */
export const THUMBNAILS_DIRECTORY = '/var/lib/desktonk/thumbnails';
