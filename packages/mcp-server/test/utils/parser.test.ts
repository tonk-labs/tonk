import {describe, it, expect, vi, beforeEach} from 'vitest';
import {parseModule, parseModuleFile} from '../../src/utils/parser.js';
import {readFile} from 'fs/promises';

// Mock fs/promises
vi.mock('fs/promises', () => ({
  readFile: vi.fn(),
}));

describe('parseModule', () => {
  it('should parse functions from a valid module', () => {
    const content = `
/**
 * Example math module with basic operations
 */
import { createModule, createFunction } from '../core/module';

export const addFn = createFunction(
  'add',
  'Adds two numbers together',
  (a: number, b: number): number => a + b
);

export const subtractFn = createFunction(
  'subtract',
  'Subtracts b from a',
  (a: number, b: number): number => a - b
);

export default createModule<{
  add: typeof addFn.fn;
  subtract: typeof subtractFn.fn;
}>([
  addFn,
  subtractFn
]);
`;

    const result = parseModule(content);
    expect(result).not.toBeNull();
    expect(result).toHaveLength(2);

    const addFn = result![0];
    expect(addFn.name).toBe('add');
    expect(addFn.description).toBe('Adds two numbers together');

    const subtractFn = result![1];
    expect(subtractFn.name).toBe('subtract');
    expect(subtractFn.description).toBe('Subtracts b from a');
  });

  it('should return null for invalid module content', () => {
    const invalidContents = [
      '', // Empty content
      'export const x = 1;', // No functions
      'export default {};', // No createModule
      'export default createModule<{}>([]);', // No functions
      'export const addFn = createFunction();', // Invalid function
    ];

    invalidContents.forEach(content => {
      expect(parseModule(content)).toBeNull();
    });
  });
});

describe('parseModuleFile', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('should parse a module file successfully', async () => {
    const mockContent = `
/**
 * Example math module
 */
export const addFn = createFunction(
  'add',
  'Adds two numbers together',
  (a: number, b: number): number => a + b
);

export default createModule<{
  add: typeof addFn.fn;
}>([addFn]);
`;

    (readFile as unknown as ReturnType<typeof vi.fn>).mockResolvedValue(
      mockContent,
    );

    const result = await parseModuleFile('test.ts');
    expect(result).not.toBeNull();
    expect(result).toHaveLength(1);
    expect(result![0].name).toBe('add');
    expect(result![0].description).toBe('Adds two numbers together');
    expect(readFile).toHaveBeenCalledWith('test.ts', 'utf-8');
  });

  it('should handle file read errors', async () => {
    (readFile as unknown as ReturnType<typeof vi.fn>).mockRejectedValue(
      new Error('File not found'),
    );

    const result = await parseModuleFile('nonexistent.ts');
    expect(result).toBeNull();
  });
});
