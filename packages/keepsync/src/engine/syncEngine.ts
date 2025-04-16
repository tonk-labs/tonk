import {
  RepoConfig,
  Repo,
  NetworkAdapter,
  StorageAdapterInterface,
  PeerId,
} from '@automerge/automerge-repo';
import {logger} from '../utils/logger';

/**
 * Options for configuring the SyncEngine
 */
export interface SyncEngineOptions {
  /** Storage adapter for persistence */
  storage?: StorageAdapterInterface;
  /** Additional network adapters */
  networkAdapters?: NetworkAdapter[];
  /** Peer ID for this client */
  peerId?: PeerId;
  /** Share policy */
  sharePolicy?: RepoConfig['sharePolicy'];
  /** Ephemeral flag */
  ephemeral?: boolean;
}

/**
 * SyncEngine class for creating and configuring an Automerge Repo
 */
export class SyncEngine {
  private repo: Repo;

  /**
   * Create a new SyncEngine instance
   * @param options Configuration options for the Repo
   */
  constructor(options: SyncEngineOptions = {}) {
    // Generate a default peerId if not provided
    const peerId =
      options.peerId ||
      ((crypto.randomUUID
        ? crypto.randomUUID()
        : Math.random().toString(36).substring(2, 15) +
          Math.random().toString(36).substring(2, 15)) as PeerId);

    // Combine primary network adapter with additional adapters if provided
    const networkAdapters = options.networkAdapters || [];

    // Configure the Repo with all available options
    this.repo = new Repo({
      network: networkAdapters.length > 0 ? networkAdapters : undefined,
      storage: options.storage,
      peerId,
      sharePolicy: options.sharePolicy,
      isEphemeral: options.ephemeral,
    });

    logger.info('SyncEngine created successfully');
  }

  /**
   * Get the Automerge Repo instance
   * @returns The configured Repo instance
   */
  getRepo(): Repo {
    return this.repo;
  }
}
