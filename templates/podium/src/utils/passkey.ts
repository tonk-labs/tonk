const PASSKEY_STORAGE_KEY = "podium_passkey_credential";
const USER_ID_STORAGE_KEY = "podium_user_id";

export interface PasskeyCredential {
  id: string;
  userId: string;
  publicKey: string;
  createdAt: number;
}

export class PasskeyManager {
  // Check if WebAuthn is supported
  static isSupported(): boolean {
    return !!(navigator.credentials && navigator.credentials.create);
  }

  // Create a new passkey for registration
  static async createPasskey(
    userId: string,
    userName: string,
  ): Promise<PasskeyCredential> {
    if (!this.isSupported()) {
      throw new Error("Passkeys are not supported in this browser");
    }

    const challenge = new Uint8Array(32);
    crypto.getRandomValues(challenge);

    try {
      // Fix RP ID for localhost and tonk domains
      const hostname = window.location.hostname;
      let rpId = hostname;
      
      if (hostname === "localhost") {
        rpId = "localhost";
      } else if (hostname.endsWith(".tonk.xyz")) {
        // For tonk.xyz subdomains, use the parent domain for passkey compatibility
        rpId = "tonk.xyz";
      }

      const credential = (await navigator.credentials.create({
        publicKey: {
          challenge,
          rp: {
            name: "Podium Family App",
            id: rpId,
          },
          user: {
            id: new TextEncoder().encode(userId),
            name: userName,
            displayName: userName,
          },
          pubKeyCredParams: [
            { alg: -7, type: "public-key" }, // ES256
            { alg: -257, type: "public-key" }, // RS256 fallback
          ],
          authenticatorSelection: {
            authenticatorAttachment: "platform",
            userVerification: "preferred",
            requireResidentKey: false,
          },
          timeout: 60000,
          attestation: "none", // Don't require attestation
        },
      })) as PublicKeyCredential;

      if (!credential) {
        throw new Error("Failed to create passkey");
      }

      const response = credential.response as AuthenticatorAttestationResponse;

      // Try to get public key, but don't fail if we can't access it
      let publicKey = "unavailable";
      try {
        if (response.getPublicKey) {
          publicKey = this.arrayBufferToBase64(response.getPublicKey()!);
        }
      } catch (error) {
        console.warn("Could not access public key:", error);
      }

      const passkeyCredential: PasskeyCredential = {
        id: this.arrayBufferToBase64(credential.rawId),
        userId,
        publicKey,
        createdAt: Date.now(),
      };

      // Store the credential locally
      try {
        localStorage.setItem(
          PASSKEY_STORAGE_KEY,
          JSON.stringify(passkeyCredential),
        );
        localStorage.setItem(USER_ID_STORAGE_KEY, userId);
      } catch (storageError) {
        console.error("❌ Failed to store credential:", storageError);
        throw new Error("Failed to store passkey credential");
      }

      return passkeyCredential;
    } catch (error) {
      console.error("WebAuthn creation failed:", error);
      throw error;
    }
  }

  // Authenticate with existing passkey
  static async authenticateWithPasskey(): Promise<string | null> {
    const storedCredential = this.getStoredCredential();
    if (!storedCredential) {
      return null;
    }

    // If it's a fallback credential, just return the userId
    if (storedCredential.publicKey === "fallback") {
      return storedCredential.userId;
    }

    // Try WebAuthn authentication for real passkeys
    if (!this.isSupported()) {
      return null;
    }

    try {
      const challenge = new Uint8Array(32);
      crypto.getRandomValues(challenge);

      // Use same RP ID logic as creation
      const hostname = window.location.hostname;
      let rpId = hostname;
      
      if (hostname === "localhost") {
        rpId = "localhost";
      } else if (hostname.endsWith(".tonk.xyz")) {
        // For tonk.xyz subdomains, use the parent domain for passkey compatibility
        rpId = "tonk.xyz";
      }

      const assertion = await navigator.credentials.get({
        publicKey: {
          challenge,
          rpId,
          allowCredentials: [
            {
              id: this.base64ToArrayBuffer(storedCredential.id),
              type: "public-key",
            },
          ],
          userVerification: "preferred",
          timeout: 60000,
        },
      });

      if (assertion) {
        return storedCredential.userId;
      }
    } catch (error) {
      console.error("❌ WebAuthn authentication failed:", error);
      // Don't clear credential immediately, might be temporary issue
    }

    return null;
  }

  // Get stored credential
  static getStoredCredential(): PasskeyCredential | null {
    const stored = localStorage.getItem(PASSKEY_STORAGE_KEY);
    if (stored) {
      try {
        return JSON.parse(stored);
      } catch {
        return null;
      }
    }
    return null;
  }

  // Get stored user ID
  static getStoredUserId(): string | null {
    return localStorage.getItem(USER_ID_STORAGE_KEY);
  }

  // Clear stored credentials
  static clearCredentials(): void {
    localStorage.removeItem(PASSKEY_STORAGE_KEY);
    localStorage.removeItem(USER_ID_STORAGE_KEY);
  }

  // Check if user has a stored passkey
  static hasStoredPasskey(): boolean {
    const hasCredential = !!this.getStoredCredential();
    return hasCredential;
  }

  // Fallback: Generate simple ID for browsers without passkey support
  static generateFallbackId(): string {
    return crypto.randomUUID();
  }

  // Helper methods
  private static arrayBufferToBase64(buffer: ArrayBuffer): string {
    const bytes = new Uint8Array(buffer);
    let binary = "";
    for (let i = 0; i < bytes.byteLength; i++) {
      binary += String.fromCharCode(bytes[i]);
    }
    return btoa(binary);
  }

  private static base64ToArrayBuffer(base64: string): ArrayBuffer {
    const binary = atob(base64);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) {
      bytes[i] = binary.charCodeAt(i);
    }
    return bytes.buffer;
  }
}
