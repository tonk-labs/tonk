import {describe, it, expect, beforeAll} from 'vitest';
import {Repo, DocumentId} from '@automerge/automerge-repo';
import {findDocument, createDocument} from '../../src/documents/addressing';
import {DummyNetworkAdapter} from '../dummy/DummyNetworkAdapter';
import {DummyStorageAdapter} from '../dummy/DummyStorageAdapter';

type DocNode = {
  type: 'dir';
  timestamps: {
    create: number;
    modified: number;
  };
  children: any[];
};

describe('Document Addressing', () => {
  async function initRoot(repo: Repo) {
    const docHandle = repo.create<any>();
    docHandle.change((_doc: any) => {
      Object.assign(_doc, {
        type: 'dir',
        timestamps: {
          create: Date.now(),
          modified: Date.now(),
        },
        children: [],
      } as DocNode);
    });
    const doc = await docHandle.doc();
    return docHandle.documentId;
  }

  const setup = async ({startReady = true} = {}) => {
    const storageAdapter = new DummyStorageAdapter();
    const networkAdapter = new DummyNetworkAdapter({startReady});

    const repo = new Repo({
      storage: storageAdapter,
      network: [networkAdapter],
    });

    const rootId = await initRoot(repo);
    return {repo, rootId, storageAdapter, networkAdapter};
  };

  // Test cases will go here
  it('test setup has a valid root document', async () => {
    // Wait a moment to ensure all operations have settled
    const {repo, rootId} = await setup();

    // const root = await findDocument(repo, '/');
    // expect(root).toBeDefined();

    const otherRootHandle = repo.find(rootId).docSync();
    // const rootDoc = await otherRootHandle.doc();
    expect(otherRootHandle).toBeDefined();
  });

  it('it can get and set a document', async () => {
    const {repo, rootId} = await setup();
    const newDoc = repo.create();
    newDoc.change((doc: any) => {
      doc.text = 'HI!';
    });
    await createDocument(repo, rootId, '/test', newDoc);
    const docNode = await findDocument(repo, rootId, '/test');
    expect(docNode).toBeDefined();
    if (docNode) {
      // DocNode should have a pointer property that references the actual document
      expect(docNode.pointer).toBeDefined();
      // Use the pointer to get the actual document
      const docHandle = repo.find(docNode.pointer!);
      const doc = (await docHandle.doc()) as any;
      expect(doc.text).toBe('HI!');
    }
  });

  it('it can get and set a deeply nested document', async () => {
    const {repo, rootId} = await setup();
    const newDoc = repo.create();
    newDoc.change((doc: any) => {
      doc.text = 'HI!';
    });
    await createDocument(repo, rootId, '/test/this/deeply/nested/doc', newDoc);
    const docNode = await findDocument(
      repo,
      rootId,
      '/test/this/deeply/nested/doc',
    );
    expect(docNode).toBeDefined();
    if (docNode) {
      // DocNode should have a pointer property that references the actual document
      expect(docNode.pointer).toBeDefined();
      // Use the pointer to get the actual document
      const docHandle = repo.find(docNode.pointer!);
      const doc = (await docHandle.doc()) as any;
      expect(doc.text).toBe('HI!');
    }
  });

  it('it can get the root document', async () => {
    const {repo, rootId} = await setup();
    const newDoc = repo.create();
    newDoc.change((doc: any) => {
      doc.text = 'HI!';
    });
    await createDocument(repo, rootId, '/test/this/deeply/nested/doc', newDoc);
    const docNode = await findDocument(repo, rootId, '/');
    expect(docNode).toBeDefined();
    if (docNode) {
      // DocNode should have a pointer property that references the actual document
      expect(docNode.children).toBeDefined();
      expect(docNode.children!.length).toBe(1);
    }
  });
});
