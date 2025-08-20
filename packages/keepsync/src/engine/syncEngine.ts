import {
  RepoConfig,
  Repo,
  StorageAdapterInterface,
  PeerId,
  NetworkAdapterInterface,
  DocumentId,
} from '@automerge/automerge-repo/slim';
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
    // If URL is empty, use a hardcoded initial document that all peers share
    if (!url || url.trim() === '') {
      logger.info('Using hardcoded shared root document for P2P sync');
      return this.#createSharedRootDocument(repo);
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
        return this.#createSharedRootDocument(repo);
      }

      return rootId;
    } catch (error) {
      console.error(error);
      throw new Error('unexpected error when fetching root document');
    }
  };

  #createSharedRootDocument = async (repo: Repo): Promise<DocumentId> => {
    // Hardcoded initial document bytes that all peers will share
    // This ensures all peers start with the same root document for sync
    const initialDocBytes = new Uint8Array([
      133, 111, 74, 131, 61, 157, 231, 85, 0, 118, 1, 16, 120, 107, 104, 47,
      215, 9, 76, 32, 132, 136, 60, 124, 152, 120, 144, 182, 1, 143, 164, 31,
      13, 102, 61, 139, 125, 246, 189, 135, 97, 16, 167, 63, 30, 215, 249, 60,
      227, 113, 111, 61, 55, 138, 234, 94, 30, 142, 166, 78, 250, 6, 1, 2, 3, 2,
      19, 2, 35, 2, 64, 2, 86, 2, 7, 21, 14, 33, 2, 35, 2, 52, 1, 66, 2, 86, 2,
      128, 1, 2, 127, 0, 127, 1, 127, 2, 127, 0, 127, 0, 127, 7, 126, 5, 102,
      105, 108, 101, 115, 6, 115, 116, 97, 116, 101, 115, 2, 0, 2, 1, 2, 2, 0,
      2, 0, 2, 0, 0,
    ]);

    try {
      // Import the hardcoded document into the repo
      const docHandle = repo.import(initialDocBytes);
      logger.info('Successfully imported shared root document');
      return docHandle.documentId;
    } catch (error) {
      logger.error(
        'Failed to import hardcoded root document, falling back to creating new one:',
        error
      );
      // Fallback: create a new document with the expected structure
      const docHandle = repo.create({ files: {}, states: {} });
      return docHandle.documentId;
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
