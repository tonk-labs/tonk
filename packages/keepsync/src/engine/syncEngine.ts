import {
  RepoConfig,
  Repo,
  StorageAdapterInterface,
  PeerId,
  NetworkAdapterInterface,
  DocumentId,
} from '@automerge/automerge-repo';
import { logger } from '../utils/logger';
import axios from 'axios';

/**
 * Options for configuring the SyncEngine
 */
export interface SyncEngineOptions {
  /** url of the service to resolve root document */
  url: string;
  /** Storage adapter for persistence */
  storage?: StorageAdapterInterface;
  /** Additional network adapters */
  network?: NetworkAdapterInterface[];
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
  #repo: Repo;
  #root: DocumentId | undefined;
  #ready: boolean = false;

  /**
   * Create a new SyncEngine instance
   * @param options Configuration options for the Repo
   */
  constructor(options: SyncEngineOptions = { url: '' }) {
    // Generate a default peerId if not provided
    const peerId =
      options.peerId ||
      ((crypto.randomUUID
        ? crypto.randomUUID()
        : Math.random().toString(36).substring(2, 15) +
          Math.random().toString(36).substring(2, 15)) as PeerId);

    // Combine primary network adapter with additional adapters if provided
    const networkAdapters = options.network || [];

    // Configure the Repo with all available options
    this.#repo = new Repo({
      network: networkAdapters.length > 0 ? networkAdapters : undefined,
      storage: options.storage,
      peerId,
      sharePolicy: options.sharePolicy,
      isEphemeral: options.ephemeral,
    });

    this.#root = undefined;

    // Initialize root document
    this.#getRoot(options.url, this.#repo)
      .then(rootId => {
        this.#root = rootId;
        logger.info('Root document initialized');
      })
      .catch(err => {
        logger.error('Failed to initialize root document:', err);
      });

    logger.info('SyncEngine created successfully');
  }

  #hasRoot = async () => {
    let counter = 0;
    return new Promise(async (resolve, reject) => {
      const interval = setInterval(async () => {
        counter++;
        if (this.#root) {
          clearInterval(interval);
          resolve(true);
        }
        if (counter > 100) {
          reject();
        }
      }, 200);
    });
  };

  #getRoot = async (url: string, repo: Repo): Promise<DocumentId> => {
    // If URL is empty, create a new root document
    if (!url || url.trim() === '') {
      logger.warn('Creating new root document (no URL provided)');
      const docHandle = repo.create();
      return docHandle.documentId;
    }

    try {
      // Fetch the root document ID from URL using axios instead of fetch
      const response = await axios.get(`${url}/.well-known/root.json`);

      if (!response.data) {
        throw new Error('No data was returned from the url provided');
      }

      const rootId = response.data.rootId as DocumentId;

      if (!rootId) {
        throw new Error('No root was returned from the url provided');
      }

      // Check if document exists in the repo
      const rootDoc = (await repo.find(rootId)).doc();
      if (!rootDoc) {
        logger.warn('Creating new root document (url did not return a rootId)');
        const docHandle = repo.create();
        return docHandle.documentId;
      }

      return rootId;
    } catch (error) {
      console.error(error);
      throw new Error('unexpected error when fetching root document');
    }
  };

  whenReady(): Promise<void> {
    return new Promise(async resolve => {
      await this.#repo.networkSubsystem.whenReady();
      await this.#hasRoot();
      this.#ready = true;
      resolve();
    });
  }

  /**
   * Get the Automerge Repo instance
   * @returns The configured Repo instance
   */
  getRepo(): Repo {
    if (!this.#ready) {
      logger.warn(
        'getRepo() should not be called yet. Please await on SyncEngine.whenReady() to make sure initialization is complete.'
      );
    }
    return this.#repo;
  }

  getRootId(): DocumentId | undefined {
    if (!this.#ready) {
      logger.warn(
        'getRootId() should not be called yet. Please await on SyncEngine.whenReady() to make sure initialization is complete.'
      );
    }
    return this.#root;
  }
}
