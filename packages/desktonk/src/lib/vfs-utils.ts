// Re-export VFS utilities from local vfs-client
// This maintains backward compatibility for existing imports
export {
  base64ToUint8Array,
  bytesToString,
  stringToBytes,
  uint8ArrayToBase64,
} from '@/vfs-client';
