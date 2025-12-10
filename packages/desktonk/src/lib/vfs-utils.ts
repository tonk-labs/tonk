// Re-export VFS utilities from local vfs-client
// This maintains backward compatibility for existing imports
export {
  bytesToString,
  stringToBytes,
  uint8ArrayToBase64,
  base64ToUint8Array,
} from '@/vfs-client';
