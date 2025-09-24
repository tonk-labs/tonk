interface ExtractedInfo {
  name: string;
  type: 'component' | 'store' | 'unknown';
  exportType: 'default' | 'named' | 'none';
}

/**
 * Extract name and metadata from TypeScript/TSX code using AST
 */
export const extractNameFromAST = (
  content: string,
  fileName: string,
  fileType: 'component' | 'store'
): ExtractedInfo => {
  const ts = (window as any).ts;

  // Fallback function that sanitizes filename
  const fallback = (): ExtractedInfo => {
    const cleanName = fileName
      .replace(/\.(tsx?|jsx?)$/, '')
      .split(/[-_]/)
      .map(part => part.charAt(0).toUpperCase() + part.slice(1))
      .join('');

    return {
      name: cleanName,
      type: fileType,
      exportType: 'none',
    };
  };

  if (!ts) return fallback();

  try {
    const sourceFile = ts.createSourceFile(
      fileName,
      content,
      ts.ScriptTarget.Latest,
      true,
      fileName.endsWith('.tsx') ? ts.ScriptKind.TSX : ts.ScriptKind.TS
    );

    let result: ExtractedInfo | null = null;

    const visit = (node: any): void => {
      // Handle: return someStoreVariable
      if (
        ts.isReturnStatement(node) &&
        node.expression &&
        ts.isIdentifier(node.expression)
      ) {
        const returnedIdentifier = node.expression.text;

        // Look for the variable declaration of this identifier
        const varDecl = findVariableDeclaration(sourceFile, returnedIdentifier);
        if (varDecl && isStoreDeclaration(varDecl)) {
          result = {
            name: returnedIdentifier,
            type: 'store',
            exportType: 'default',
          };
        }
      }

      // Handle: export default function ComponentName()
      else if (
        ts.isFunctionDeclaration(node) &&
        hasDefaultExportModifiers(node) &&
        node.name
      ) {
        result = {
          name: node.name.text,
          type: fileType,
          exportType: 'default',
        };
      }

      // Handle: export default class ComponentName
      else if (
        ts.isClassDeclaration(node) &&
        hasDefaultExportModifiers(node) &&
        node.name
      ) {
        result = {
          name: node.name.text,
          type: fileType,
          exportType: 'default',
        };
      }

      // Handle: const ComponentName = () => {} with export default ComponentName
      else if (ts.isVariableStatement(node)) {
        const declaration = node.declarationList.declarations[0];
        if (declaration && ts.isIdentifier(declaration.name)) {
          const varName = declaration.name.text;

          // For stores: Check if this variable is returned OR exported as default
          if (fileType === 'store') {
            const isReturned = isIdentifierReturned(sourceFile, varName);
            const isExported = findDefaultExportOfIdentifier(
              sourceFile,
              varName
            );
            if (
              (isReturned || isExported) &&
              declaration.initializer &&
              isStoreCreation(declaration.initializer)
            ) {
              result = {
                name: varName,
                type: 'store',
                exportType: 'default',
              };
            }
          }
          // For components: Check if this variable is exported as default elsewhere
          else {
            const hasDefaultExport = findDefaultExportOfIdentifier(
              sourceFile,
              varName
            );
            if (hasDefaultExport) {
              result = {
                name: varName,
                type: fileType,
                exportType: 'default',
              };
            }
          }
        }
      }

      // Handle: export default ComponentName (identifier)
      else if (
        ts.isExportAssignment(node) &&
        !node.isExportEquals &&
        ts.isIdentifier(node.expression)
      ) {
        result = {
          name: node.expression.text,
          type: fileType,
          exportType: 'default',
        };
      }

      // Handle: export default create(...)
      else if (
        ts.isExportAssignment(node) &&
        !node.isExportEquals &&
        ts.isCallExpression(node.expression)
      ) {
        const callExpr = node.expression;
        if (ts.isIdentifier(callExpr.expression)) {
          const funcName = callExpr.expression.text;

          // For zustand stores: export default create(...)
          if (funcName === 'create' || funcName === 'createStore') {
            result = {
              name: extractStoreNameFromContext(sourceFile, fileName),
              type: 'store',
              exportType: 'default',
            };
          }
        }
      }

      // Continue traversing
      if (!result) {
        ts.forEachChild(node, visit);
      }
    };

    visit(sourceFile);
    return result || fallback();
  } catch (error) {
    console.warn('AST parsing failed, using fallback', error);
    return fallback();
  }
};

/**
 * Helper to check if node has default export modifiers
 */
function hasDefaultExportModifiers(node: any): boolean {
  const ts = (window as any).ts;
  return (
    node.modifiers?.some((m: any) => m.kind === ts.SyntaxKind.ExportKeyword) &&
    node.modifiers?.some((m: any) => m.kind === ts.SyntaxKind.DefaultKeyword)
  );
}

/**
 * Find variable declaration by identifier name
 */
function findVariableDeclaration(sourceFile: any, identifier: string): any {
  const ts = (window as any).ts;
  let declaration: any = null;

  const visit = (node: any) => {
    if (ts.isVariableStatement(node)) {
      for (const decl of node.declarationList.declarations) {
        if (ts.isIdentifier(decl.name) && decl.name.text === identifier) {
          declaration = decl;
        }
      }
    }

    if (!declaration) {
      ts.forEachChild(node, visit);
    }
  };

  visit(sourceFile);
  return declaration;
}

/**
 * Check if a variable declaration is a store creation
 */
function isStoreDeclaration(declaration: any): boolean {
  return declaration.initializer && isStoreCreation(declaration.initializer);
}

/**
 * Check if a node represents store creation (create() call)
 */
function isStoreCreation(node: any): boolean {
  const ts = (window as any).ts;

  if (!ts.isCallExpression(node)) {
    return false;
  }

  // Handle create<Type>()(sync(...)) - curried call pattern
  if (ts.isCallExpression(node.expression)) {
    const innerCall = node.expression;
    if (
      ts.isIdentifier(innerCall.expression) &&
      innerCall.expression.text === 'create'
    ) {
      return true;
    }
  }

  // Handle simple create() call
  if (ts.isIdentifier(node.expression) && node.expression.text === 'create') {
    return true;
  }

  return false;
}

/**
 * Check if an identifier is returned in the file
 */
function isIdentifierReturned(sourceFile: any, identifier: string): boolean {
  const ts = (window as any).ts;
  let isReturned = false;

  const visit = (node: any) => {
    if (
      ts.isReturnStatement(node) &&
      node.expression &&
      ts.isIdentifier(node.expression) &&
      node.expression.text === identifier
    ) {
      isReturned = true;
    }

    if (!isReturned) {
      ts.forEachChild(node, visit);
    }
  };

  visit(sourceFile);
  return isReturned;
}

/**
 * Find if an identifier is exported as default
 */
function findDefaultExportOfIdentifier(
  sourceFile: any,
  identifier: string
): boolean {
  const ts = (window as any).ts;
  let found = false;

  const visit = (node: any) => {
    if (
      ts.isExportAssignment(node) &&
      !node.isExportEquals &&
      ts.isIdentifier(node.expression) &&
      node.expression.text === identifier
    ) {
      found = true;
    }

    if (!found) {
      ts.forEachChild(node, visit);
    }
  };

  visit(sourceFile);
  return found;
}

/**
 * Determine export type for an identifier
 */
function findExportTypeForIdentifier(
  sourceFile: any,
  identifier: string
): 'default' | 'named' | 'none' {
  const ts = (window as any).ts;
  let exportType: 'default' | 'named' | 'none' = 'none';

  const visit = (node: any) => {
    // Check for export default
    if (
      ts.isExportAssignment(node) &&
      !node.isExportEquals &&
      ts.isIdentifier(node.expression) &&
      node.expression.text === identifier
    ) {
      exportType = 'default';
    }

    // Check for named export
    if (
      ts.isExportDeclaration(node) &&
      node.exportClause &&
      ts.isNamedExports(node.exportClause)
    ) {
      for (const element of node.exportClause.elements) {
        if ((element.propertyName?.text || element.name.text) === identifier) {
          exportType = 'named';
        }
      }
    }

    ts.forEachChild(node, visit);
  };

  visit(sourceFile);
  return exportType;
}

/**
 * Extract store name from context when using create() directly
 */
function extractStoreNameFromContext(
  sourceFile: any,
  fileName: string
): string {
  const ts = (window as any).ts;

  // Look for comments or nearby context that might indicate the store name
  // Check for interface definitions like "interface UserStore"
  let storeName: string | null = null;

  const visit = (node: any) => {
    if (ts.isInterfaceDeclaration(node) && node.name.text.endsWith('Store')) {
      storeName = 'use' + node.name.text;
    } else if (
      ts.isTypeAliasDeclaration(node) &&
      node.name.text.endsWith('Store')
    ) {
      storeName = 'use' + node.name.text;
    }

    if (!storeName) {
      ts.forEachChild(node, visit);
    }
  };

  visit(sourceFile);

  if (storeName) return storeName;

  // Fallback to filename-based naming
  const baseName = fileName
    .replace(/\.(tsx?|jsx?)$/, '')
    .split(/[-_]/)
    .map(part => part.charAt(0).toUpperCase() + part.slice(1))
    .join('');

  return `use${baseName}Store`;
}

// Specialized extractors for backwards compatibility
export const extractComponentName = (
  content: string,
  fileName: string
): string => {
  return extractNameFromAST(content, fileName, 'component').name;
};

export const extractStoreName = (content: string, fileName: string): string => {
  return extractNameFromAST(content, fileName, 'store').name;
};
