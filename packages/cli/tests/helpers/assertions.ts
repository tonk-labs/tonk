import { expect } from 'vitest';
import fs from 'fs-extra';

export function expectFileExists(filePath: string) {
  return expect(fs.pathExists(filePath)).resolves.toBe(true);
}

export function expectFileNotExists(filePath: string) {
  return expect(fs.pathExists(filePath)).resolves.toBe(false);
}

export async function expectFileContent(
  filePath: string,
  expectedContent: string
) {
  const actualContent = await fs.readFile(filePath, 'utf-8');
  expect(actualContent.trim()).toBe(expectedContent.trim());
}

export async function expectFileContentContains(
  filePath: string,
  substring: string
) {
  const content = await fs.readFile(filePath, 'utf-8');
  expect(content).toContain(substring);
}

export async function expectJsonFile(filePath: string, expectedData: any) {
  const actualData = await fs.readJson(filePath);
  expect(actualData).toEqual(expectedData);
}

export async function expectJsonFileContains(
  filePath: string,
  partialData: any
) {
  const actualData = await fs.readJson(filePath);
  expect(actualData).toMatchObject(partialData);
}

export function expectDirectoryExists(dirPath: string) {
  return expect(fs.pathExists(dirPath)).resolves.toBe(true);
}

export async function expectDirectoryContains(
  dirPath: string,
  fileNames: string[]
) {
  const files = await fs.readdir(dirPath);
  for (const fileName of fileNames) {
    expect(files).toContain(fileName);
  }
}

export function expectCLISuccess(result: {
  success: boolean;
  code: number | null;
}) {
  expect(result.success).toBe(true);
  expect(result.code).toBe(0);
}

export function expectCLIFailure(result: {
  success: boolean;
  code: number | null;
}) {
  expect(result.success).toBe(false);
  expect(result.code).not.toBe(0);
}

export function expectCLIOutput(
  result: { stdout: string },
  expectedOutput: string | RegExp
) {
  if (typeof expectedOutput === 'string') {
    expect(result.stdout).toContain(expectedOutput);
  } else {
    expect(result.stdout).toMatch(expectedOutput);
  }
}

export function expectCLIError(
  result: { stderr: string },
  expectedError: string | RegExp
) {
  if (typeof expectedError === 'string') {
    expect(result.stderr).toContain(expectedError);
  } else {
    expect(result.stderr).toMatch(expectedError);
  }
}
