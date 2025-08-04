import { DocumentId, Repo, DocHandle } from '@automerge/automerge-repo/slim';

// Reference to a node stored in a parent's children array
export type RefNode = {
  pointer: DocumentId;
  type: 'doc' | 'dir';
  timestamps: {
    create: number;
    modified: number;
  };
  name: string;
};

// The full document node stored at a DocumentId
export type DirNode = {
  type: 'dir';
  name: string;
  timestamps: {
    create: number;
    modified: number;
  };
  children?: RefNode[];
};

export type DocNode = {
  type: 'doc' | 'dir';
  pointer?: DocumentId;
  name: string;
  timestamps: {
    create: number;
    modified: number;
  };
  children?: RefNode[];
};

type TraverseResult = {
  // Handle to the node at the parent path
  nodeHandle: DocHandle<DirNode>;
  // The actual node at the parent path
  node: DirNode;
  // Reference to the target node in the parent's children array (if target is not root)
  targetRef?: RefNode;
  // Path to the parent directory
  parentPath: string;
};

/**
 * Traverses the document tree to find or create nodes along a path
 * @param repo The Automerge repository
 * @param rootId The ID of the root document
 * @param path The path to traverse
 * @param createMissing Whether to create missing directories along the way
 * @returns Information about the target node and its parent
 */
export async function traverseDocTree(
  repo: Repo,
  rootId: DocumentId,
  path: string,
  createMissing: boolean = false
): Promise<TraverseResult | undefined> {
  // Normalize the path and split it into segments
  const normalizedPath = path.startsWith('/') ? path.slice(1) : path;
  const segments = normalizedPath ? normalizedPath.split('/') : [];

  // No segments means root path
  if (segments.length === 0) {
    const rootHandle = await repo.find<DirNode>(rootId);
    const rootDoc = await rootHandle.doc();

    // Initialize root if it doesn't exist and we're creating missing nodes
    if (!rootDoc && createMissing) {
      rootHandle.change((doc: DirNode) => {
        doc.type = 'dir';
        doc.name = '/';
        doc.timestamps = {
          create: Date.now(),
          modified: Date.now(),
        };
        doc.children = [];
      });

      const newDoc = await rootHandle.doc();
      if (!newDoc) {
        return undefined;
      }

      return {
        nodeHandle: rootHandle,
        node: JSON.parse(JSON.stringify(newDoc)) as DirNode,
        parentPath: '',
      };
    } else if (!rootDoc) {
      return undefined;
    }

    return {
      nodeHandle: rootHandle,
      node: JSON.parse(JSON.stringify(rootDoc)) as DirNode,
      parentPath: '',
    };
  }

  // Navigate through the tree
  let currentNodeId = rootId;
  let currentHandle = await repo.find<DirNode>(currentNodeId);
  let currentDoc = await currentHandle.doc();
  let currentPath = '';

  // Initialize root if needed and we're creating missing directories
  if (!currentDoc && createMissing) {
    currentHandle.change((doc: DirNode) => {
      doc.type = 'dir';
      doc.name = '/';
      doc.timestamps = {
        create: Date.now(),
        modified: Date.now(),
      };
      doc.children = [];
    });

    const updatedDoc = await currentHandle.doc();
    if (!updatedDoc) {
      return undefined;
    }
    currentDoc = JSON.parse(JSON.stringify(updatedDoc)) as DirNode;
  } else if (!currentDoc) {
    return undefined;
  } else {
    currentDoc = JSON.parse(JSON.stringify(currentDoc)) as DirNode;
  }

  // Track parent for document creation
  let lastSegmentRef: RefNode | undefined;

  // Process each path segment
  for (let i = 0; i < segments.length; i++) {
    const segment = segments[i];
    const isLastSegment = i === segments.length - 1;

    // Update current path
    currentPath += `/${segment}`;

    // Ensure children array exists and is an array
    if (!currentDoc.children && createMissing) {
      currentHandle.change((doc: DirNode) => {
        doc.children = [];
      });
      const updatedDoc = await currentHandle.doc();
      if (!updatedDoc) {
        return undefined;
      }
      currentDoc = updatedDoc as DirNode;
    } else if (!currentDoc.children) {
      return undefined;
    }

    // At this point we know currentDoc.children exists
    const children = currentDoc.children || [];

    // Find the child with the matching name
    const childRef = children.find(child => child.name === segment);

    if (!childRef && createMissing) {
      // Create a new directory document with repo.create()
      const newNodeHandle = repo.create<DirNode>();
      const newNodeId = newNodeHandle.documentId;

      // Initialize the new node as a directory
      newNodeHandle.change((doc: DirNode) => {
        doc.type = 'dir';
        doc.name = segment;
        doc.timestamps = {
          create: Date.now(),
          modified: Date.now(),
        };
        doc.children = [];
      });

      // Add the new node to the parent's children with duplicate prevention
      currentHandle.change((doc: DirNode) => {
        if (!doc.children) {
          doc.children = [];
        }

        // Check if a directory with this name already exists (prevents race condition duplicates)
        const existingChild = doc.children.find(
          child => child.name === segment
        );
        if (existingChild) {
          // Another concurrent operation already created this directory, skip adding duplicate
          return;
        }

        // Create a reference node that points to the actual directory node
        const newDirRef: RefNode = {
          name: segment,
          type: 'dir',
          pointer: newNodeId,
          timestamps: {
            create: Date.now(),
            modified: Date.now(),
          },
        };

        doc.children.push(newDirRef);

        if (doc.timestamps) {
          doc.timestamps.modified = Date.now();
        }
      });

      // After the change operation, check what actually exists in the parent
      const updatedParentDoc = await currentHandle.doc();
      if (!updatedParentDoc) {
        return undefined;
      }

      // Find the actual child that exists (either ours or one created concurrently)
      const actualChild = updatedParentDoc.children?.find(
        child => child.name === segment
      );
      if (!actualChild || !actualChild.pointer) {
        return undefined;
      }

      // Update for next iteration - use the actual child that exists
      currentNodeId = actualChild.pointer;
      currentHandle = await repo.find<DirNode>(currentNodeId);
      const newDoc = await currentHandle.doc();

      if (!newDoc) {
        return undefined;
      }

      currentDoc = newDoc as DirNode;
      lastSegmentRef = undefined;

      if (isLastSegment) {
        return {
          nodeHandle: currentHandle,
          node: JSON.parse(JSON.stringify(currentDoc)),
          parentPath: currentPath,
        };
      }
    } else if (!childRef) {
      // Node doesn't exist and we're not creating it
      return undefined;
    } else if (childRef.pointer) {
      // Navigate to existing node (whether document or directory)
      lastSegmentRef = childRef;

      // If this is the last segment, we can return without further navigation
      if (isLastSegment) {
        return {
          nodeHandle: currentHandle,
          node: currentDoc,
          targetRef: childRef,
          parentPath: currentPath,
        };
      }

      // Only try to navigate further if it's a directory
      if (childRef.type === 'dir') {
        currentNodeId = childRef.pointer;
        currentHandle = await repo.find<DirNode>(currentNodeId);
        const nextDoc = await currentHandle.doc();

        if (!nextDoc) {
          return undefined;
        }

        // Create a clean copy of the document to avoid "Cannot create a reference to an existing document object" error
        // We need to use a completely new reference to avoid carrying over Automerge metadata
        currentDoc = JSON.parse(JSON.stringify(nextDoc)) as DirNode;
      } else {
        // Found a document when we need to navigate further, so fail
        return undefined;
      }
    } else {
      // Found a node without a pointer
      return undefined;
    }
  }

  // If we get here, we've reached the requested node
  return {
    nodeHandle: currentHandle,
    node: JSON.parse(JSON.stringify(currentDoc)),
    targetRef: lastSegmentRef,
    parentPath: currentPath,
  };
}

/**
 * Finds a document at the specified path
 * @param repo The Automerge repository
 * @param rootId The ID of the root document
 * @param path The path to the document
 * @returns The RefNode if found at the leaf, or the FullNode for the root
 */
export async function findDocument<T>(
  repo: Repo,
  rootId: DocumentId,
  path: string
): Promise<DocHandle<T> | undefined> {
  const result = await traverseDocTree(repo, rootId, path);

  if (!result) {
    return undefined;
  }

  // If this is a path to a document/directory in a parent directory, return the target reference
  if (!result.targetRef || result.targetRef?.type === 'dir') {
    return undefined;
  }
  // Otherwise, return the node itself (for root or created nodes)
  return await repo.find(result.targetRef!.pointer);
}

/**
 * Creates a document at the specified path
 * @param repo The Automerge repository
 * @param rootId The ID of the root document
 * @param path The path where the document should be created
 * @param document The document to create
 * @returns The document handle for the newly created document
 */
export async function createDocument<T>(
  repo: Repo,
  rootId: DocumentId,
  path: string,
  docHandle: DocHandle<T>
): Promise<void> {
  const normalizedPath = path.startsWith('/') ? path.slice(1) : path;
  const segments = normalizedPath.split('/');

  if (segments.length === 0 || (segments.length === 1 && segments[0] === '')) {
    throw new Error('Cannot create document at root path');
  }

  const fileName = segments[segments.length - 1];
  // Get the parent directory path
  const dirPath = segments.slice(0, -1).join('/');

  // Traverse the tree, creating directories as needed
  const result = await traverseDocTree(repo, rootId, dirPath, true);

  if (!result) {
    throw new Error(`Failed to create directory structure for ${path}`);
  }

  let parentHandle;
  if (result.targetRef) {
    parentHandle = await repo.find<DirNode>(result.targetRef.pointer);
  } else {
    parentHandle = result.nodeHandle;
  }

  // Add the document to its parent directory
  parentHandle.change((doc: DirNode) => {
    if (!doc.children) {
      doc.children = [];
    }

    // Check if document already exists
    const existingIndex = doc.children.findIndex(
      (child: RefNode) => child.name === fileName
    );

    const docRef: RefNode = {
      name: fileName,
      type: 'doc',
      pointer: docHandle.documentId,
      timestamps: {
        create: Date.now(),
        modified: Date.now(),
      },
    };

    if (existingIndex >= 0) {
      // Update existing entry
      doc.children[existingIndex] = docRef;
    } else {
      // Add new entry
      doc.children.push(docRef);
    }

    if (doc.timestamps) {
      doc.timestamps.modified = Date.now();
    }
  });
}

/**
 * Removes a document at the specified path
 * @param repo The Automerge repository
 * @param rootId The ID of the root document
 * @param path The path to the document to remove
 * @returns Whether the document was successfully removed
 */
export async function removeDocument(
  repo: Repo,
  rootId: DocumentId,
  path: string
): Promise<boolean> {
  // Find the document and its parent
  const result = await traverseDocTree(repo, rootId, path);

  if (!result || !result.targetRef) {
    return false; // Document or directory not found
  }

  // Get the document ID to delete
  const docId = result.targetRef.pointer;
  if (!docId) {
    return false; // No pointer to document
  }

  // Handle directory: recursively remove all children first
  if (result.targetRef.type === 'dir') {
    const dirHandle = await repo.find<DirNode>(docId);
    const dirDoc = await dirHandle.doc();

    if (dirDoc && dirDoc.children && dirDoc.children.length > 0) {
      // Process all children recursively
      for (const child of [...dirDoc.children]) {
        // Create a copy of the array to iterate over
        const childPath = `${path}/${child.name}`;
        await removeDocument(repo, rootId, childPath);
      }
    }
  }

  // Remove from parent's children array
  result.nodeHandle.change((doc: DirNode) => {
    if (!doc.children) return;

    const index = doc.children.findIndex(
      child =>
        child.name === result.targetRef!.name &&
        child.pointer === result.targetRef!.pointer
    );

    if (index >= 0) {
      doc.children.splice(index, 1);

      // Update modified timestamp
      if (doc.timestamps) {
        doc.timestamps.modified = Date.now();
      }
    }
  });

  // Delete the document from the repo
  repo.delete(docId);

  return true;
}
