// Shared by the native host and Cloudflare proxy. Development checkpoint
// includes separate replicas from reconnects, not only the active replica.
export const checkpointLimit = 64 * 1024 * 1024;

// Expanded JSON is bounded independently of the compressed R2 upload.
export const checkpointExpandedLimit = 256 * 1024 * 1024;
