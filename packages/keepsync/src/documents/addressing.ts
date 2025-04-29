import {DocumentId, Repo, DocHandle, Doc} from '@automerge/automerge-repo';

export type DocNode = {
  pointer?: DocumentId;
  type: 'doc' | 'dir';
  timestamps: {
    create: number;
    modified: number;
  };
  name?: string;
  children?: DocNode[];
};

type TraverseResult = {
  handle: DocHandle<DocNode>;
  doc: DocNode;
  childNode?: DocNode;
  parentPath: string;
};

/**
 * Traverses the document tree to find or create nodes along a path
 * @param repo The Automerge repository
 * @param rootId The ID of the root document
 * @param path The path to traverse
 * @param createMissing Whether to create missing directories along the way
 * @returns Information about the final node and its parent
 */
async function traverseDocTree(
  repo: Repo,
  rootId: DocumentId,
  path: string,
  createMissing: boolean = false,
): Promise<TraverseResult | undefined> {
  // Normalize the path and split it into segments
  const normalizedPath = path.startsWith('/') ? path.slice(1) : path;
  const segments = normalizedPath ? normalizedPath.split('/') : [];

  // No segments means root path
  if (segments.length === 0) {
    const rootHandle = repo.find<DocNode>(rootId);
    const rootDoc = await rootHandle.doc();

    // Initialize root if it doesn't exist and we're creating missing nodes
    if (!rootDoc && createMissing) {
      rootHandle.change((doc: DocNode) => {
        Object.assign(doc, {
          type: 'dir',
          timestamps: {
            create: Date.now(),
            modified: Date.now(),
          },
          children: [],
        } as DocNode);
      });

      const newDoc = await rootHandle.doc();
      if (!newDoc) {
        return undefined;
      }

      return {
        handle: rootHandle,
        doc: newDoc as DocNode,
        parentPath: '',
      };
    } else if (!rootDoc) {
      return undefined;
    }

    return {
      handle: rootHandle,
      doc: rootDoc as DocNode,
      parentPath: '',
    };
  }

  // Navigate through the tree
  let currentNodeId = rootId;
  let currentHandle = repo.find<DocNode>(currentNodeId);
  let currentDoc = await currentHandle.doc();
  let currentPath = '';

  // Initialize root if needed and we're creating missing directories
  if (!currentDoc && createMissing) {
    currentHandle.change((doc: DocNode) => {
      Object.assign(doc, {
        type: 'dir',
        timestamps: {
          create: Date.now(),
          modified: Date.now(),
        },
        children: [],
      } as DocNode);
    });

    const updatedDoc = await currentHandle.doc();
    if (!updatedDoc) {
      return undefined;
    }
    currentDoc = updatedDoc as DocNode;
  } else if (!currentDoc) {
    return undefined;
  } else {
    currentDoc = currentDoc as DocNode;
  }

  // Track parent for document creation
  let parentHandle = currentHandle;
  let parentDoc = currentDoc;
  let lastSegmentNode: DocNode | undefined;

  // Process each path segment
  for (let i = 0; i < segments.length; i++) {
    const segment = segments[i];
    const isLastSegment = i === segments.length - 1;

    // Update current path
    currentPath += `/${segment}`;

    // Ensure children array exists and is an array
    if (!currentDoc.children && createMissing) {
      currentHandle.change((doc: DocNode) => {
        doc.children = [];
      });
      const updatedDoc = await currentHandle.doc();
      if (!updatedDoc) {
        return undefined;
      }
      currentDoc = updatedDoc as DocNode;
    } else if (!currentDoc.children) {
      return undefined;
    }

    // At this point we know currentDoc.children exists
    const children = currentDoc.children || [];

    // Find the child with the matching name
    let childNode = children.find(child => child.name === segment);

    if (!childNode && createMissing) {
      // Create a new directory document with repo.create()
      const newNodeHandle = repo.create<DocNode>();
      const newNodeId = newNodeHandle.documentId;

      // Initialize the new node as a directory
      newNodeHandle.change((doc: DocNode) => {
        Object.assign(doc, {
          type: 'dir',
          timestamps: {
            create: Date.now(),
            modified: Date.now(),
          },
          children: [],
        } as DocNode);
      });

      // Add the new node to the parent's children
      currentHandle.change((doc: DocNode) => {
        if (!doc.children) {
          doc.children = [];
        }

        const newDirNode: DocNode = {
          name: segment,
          type: 'dir',
          pointer: newNodeId,
          timestamps: {
            create: Date.now(),
            modified: Date.now(),
          },
        };

        doc.children.push(newDirNode);

        if (doc.timestamps) {
          doc.timestamps.modified = Date.now();
        }
      });

      // Update for next iteration
      parentHandle = currentHandle;
      parentDoc = currentDoc;
      currentNodeId = newNodeId;
      currentHandle = newNodeHandle;
      const newDoc = await currentHandle.doc();

      if (!newDoc) {
        return undefined;
      }

      currentDoc = newDoc as DocNode;
      lastSegmentNode = undefined;

      if (isLastSegment) {
        return {
          handle: currentHandle,
          doc: currentDoc,
          parentPath: currentPath,
        };
      }
    } else if (!childNode) {
      // Node doesn't exist and we're not creating it
      return undefined;
    } else if (childNode.pointer) {
      // Navigate to existing node (whether document or directory)
      parentHandle = currentHandle;
      parentDoc = currentDoc;
      lastSegmentNode = childNode;

      // If this is the last segment, we can return without further navigation
      if (isLastSegment) {
        return {
          handle: currentHandle,
          doc: currentDoc,
          childNode,
          parentPath: currentPath,
        };
      }

      // Only try to navigate further if it's a directory
      if (childNode.type === 'dir') {
        currentNodeId = childNode.pointer;
        currentHandle = repo.find<DocNode>(currentNodeId);
        const newDoc = await currentHandle.doc();

        if (!newDoc) {
          return undefined;
        }

        currentDoc = newDoc as DocNode;
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
    handle: currentHandle,
    doc: currentDoc,
    childNode: lastSegmentNode,
    parentPath: currentPath,
  };
}

/**
 * Finds a document at the specified path
 * @param repo The Automerge repository
 * @param rootId The ID of the root document
 * @param path The path to the document
 * @returns The DocNode if found, undefined otherwise
 */
export async function findDocument(
  repo: Repo,
  rootId: DocumentId,
  path: string,
): Promise<DocNode | undefined> {
  const result = await traverseDocTree(repo, rootId, path);

  if (!result) {
    return undefined;
  }

  // If this is a path to a document/directory in a parent directory, return the child node
  if (result.childNode) {
    return result.childNode;
  }

  // Otherwise, return the node itself (for root or created nodes)
  return result.doc;
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
  docHandle: DocHandle<T>,
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

  // Add the document to its parent directory
  result.handle.change((doc: DocNode) => {
    if (!doc.children) {
      doc.children = [];
    }

    // Check if document already exists
    const existingIndex = doc.children.findIndex(
      (child: DocNode) => child.name === fileName,
    );

    const docNode: DocNode = {
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
      doc.children[existingIndex] = docNode;
    } else {
      // Add new entry
      doc.children.push(docNode);
    }

    if (doc.timestamps) {
      doc.timestamps.modified = Date.now();
    }
  });
}
