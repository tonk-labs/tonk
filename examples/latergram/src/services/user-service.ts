import { getVFSService } from './vfs-service';

export interface UserData {
  id: string;
  createdAt: number;
  lastSeen: number;
}

class UserService {
  private userData: UserData | null = null;
  private vfs = getVFSService();
  private readonly STORAGE_KEY = 'latergram_user_id';

  private generateUserId(): string {
    return crypto.randomUUID();
  }

  private async ensureUserDirectories(userId: string): Promise<void> {
    if (!this.vfs.isInitialized()) {
      console.warn('VFS not initialized, skipping user directory creation');
      return;
    }

    const userDir = `/users/${userId}`;
    const chatHistoryDir = `${userDir}/chat-history`;

    try {
      const userDirExists = await this.vfs.exists(userDir);
      if (!userDirExists) {
        console.log('Creating user directory:', userDir);
        await this.vfs.writeFile(`${userDir}/.keep`, { content: {} }, true);
        await this.vfs.writeFile(
          `${chatHistoryDir}/.keep`,
          { content: {} },
          true
        );

        // Store user metadata
        const userMetadata = {
          id: userId,
          createdAt: Date.now(),
          lastSeen: Date.now(),
        };
        await this.vfs.writeFile(
          `${userDir}/metadata.json`,
          { content: userMetadata },
          true
        );
      } else {
        // Update last seen
        try {
          const metadataPath = `${userDir}/metadata.json`;
          const metadataContent = await this.vfs.readFile(metadataPath);
          const metadata = metadataContent.content as any;
          metadata.lastSeen = Date.now();
          await this.vfs.writeFile(metadataPath, metadata, false);
        } catch (error) {
          console.warn('Could not update user metadata:', error);
        }
      }
    } catch (error) {
      console.error('Error setting up user directory:', error);
    }
  }

  async initialize(): Promise<UserData> {
    // Check localStorage for existing user ID
    let userId = localStorage.getItem(this.STORAGE_KEY);

    if (!userId) {
      // Generate new user ID
      userId = this.generateUserId();
      localStorage.setItem(this.STORAGE_KEY, userId);
      console.log('New user ID generated:', userId);
    } else {
      console.log('Existing user ID retrieved:', userId);
    }

    // Create user data
    this.userData = {
      id: userId,
      createdAt: Date.now(),
      lastSeen: Date.now(),
    };

    // Make userData available globally
    (window as any).userData = this.userData;

    // Ensure user directories exist in VFS
    await this.ensureUserDirectories(userId);

    return this.userData;
  }

  getUserData(): UserData | null {
    return this.userData;
  }

  getUserId(): string | null {
    return this.userData?.id || null;
  }

  getUserPath(subpath?: string): string | null {
    if (!this.userData) return null;
    const basePath = `/users/${this.userData.id}`;
    return subpath ? `${basePath}/${subpath}` : basePath;
  }

  getChatHistoryPath(): string | null {
    return this.getUserPath('chat-history/chat.json');
  }

  reset(): void {
    localStorage.removeItem(this.STORAGE_KEY);
    this.userData = null;
    (window as any).userData = undefined;
  }
}

// Singleton instance
let userServiceInstance: UserService | null = null;

export function getUserService(): UserService {
  if (!userServiceInstance) {
    userServiceInstance = new UserService();
  }
  return userServiceInstance;
}

export function resetUserService(): void {
  if (userServiceInstance) {
    userServiceInstance.reset();
    userServiceInstance = null;
  }
}
