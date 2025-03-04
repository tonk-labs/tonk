import {readFile} from 'fs/promises';
import * as ts from 'typescript';

/**
 * Interface for a parsed function
 */
export interface ParsedFunction {
  /** Name of the function */
  name: string;
  /** Description of the function */
  description: string;
}

/**
 * Parses a file content to extract module information using TypeScript's AST.
 * This parser is designed to work with partial code and doesn't require all imports
 * to be resolvable. It focuses on extracting function declarations without performing
 * type checking or import resolution.
 *
 * @param content - The content of the file to parse
 * @returns Array of parsed functions or null if no valid functions found
 */
export function parseModule(content: string): ParsedFunction[] | null {
  const functions: ParsedFunction[] = [];

  function visit(node: ts.Node) {
    if (ts.isImportDeclaration(node)) {
      return;
    }

    if (ts.isVariableStatement(node)) {
      const declaration = node.declarationList.declarations[0];
      if (
        ts.isVariableDeclaration(declaration) &&
        declaration.initializer &&
        ts.isCallExpression(declaration.initializer)
      ) {
        if (declaration.initializer.expression.getText() === 'createFunction') {
          const args = declaration.initializer.arguments;
          if (args.length >= 3) {
            const name = args[0].getText().replace(/['"]/g, '');
            const description = args[1].getText().replace(/['"]/g, '');
            functions.push({name, description});
          }
        }
      }
    }

    ts.forEachChild(node, visit);
  }

  try {
    const sourceFile = ts.createSourceFile(
      'temp.ts',
      content,
      ts.ScriptTarget.Latest,
      true,
    );

    visit(sourceFile);

    if (functions.length === 0) {
      return null;
    }

    return functions;
  } catch (error) {
    console.error('Error parsing module:', error);
    return null;
  }
}

/**
 * Parses a file to extract module information
 * @param filePath - Path to the file to parse
 * @returns The parsed module information
 */
export async function parseModuleFile(
  filePath: string,
): Promise<ParsedFunction[] | null> {
  try {
    const content = await readFile(filePath, 'utf-8');
    return parseModule(content);
  } catch (error) {
    console.error(`Error parsing module file ${filePath}:`, error);
    return null;
  }
}
