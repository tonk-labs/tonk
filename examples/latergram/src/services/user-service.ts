export interface UserData {
  id: string;
  createdAt: number;
  lastSeen: number;
}

class UserService {
  private userData: UserData | null = null;
  private readonly STORAGE_KEY = 'latergram_user_id';
  private readonly METADATA_KEY = 'latergram_user_metadata';

  private generateUserId(): string {
    return crypto.randomUUID();
  }

  private loadMetadata(): Omit<UserData, 'id'> | null {
    try {
      const metadata = localStorage.getItem(this.METADATA_KEY);
      return metadata ? JSON.parse(metadata) : null;
    } catch (error) {
      console.error('Failed to load user metadata:', error);
      return null;
    }
  }

  private saveMetadata(metadata: Omit<UserData, 'id'>): void {
    try {
      localStorage.setItem(this.METADATA_KEY, JSON.stringify(metadata));
    } catch (error) {
      console.error('Failed to save user metadata:', error);
    }
  }

  async initialize(): Promise<UserData> {
    let userId = localStorage.getItem(this.STORAGE_KEY);
    let metadata = this.loadMetadata();

    if (!userId) {
      userId = this.generateUserId();
      localStorage.setItem(this.STORAGE_KEY, userId);
      console.log('New user ID generated:', userId);

      metadata = {
        createdAt: Date.now(),
        lastSeen: Date.now(),
      };
      this.saveMetadata(metadata);
    } else {
      console.log('Existing user ID retrieved:', userId);

      if (!metadata) {
        metadata = {
          createdAt: Date.now(),
          lastSeen: Date.now(),
        };
      } else {
        metadata.lastSeen = Date.now();
      }
      this.saveMetadata(metadata);
    }

    this.userData = {
      id: userId,
      ...metadata,
    };

    (window as any).userData = this.userData;

    return this.userData;
  }

  getUserData(): UserData | null {
    return this.userData;
  }

  getUserId(): string | null {
    return this.userData?.id || null;
  }

  reset(): void {
    localStorage.removeItem(this.STORAGE_KEY);
    localStorage.removeItem(this.METADATA_KEY);
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
