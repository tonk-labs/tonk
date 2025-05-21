import path from "path";

/**
 * Get the project root directory regardless of where the code is running from
 * @returns The absolute path to the project root
 */
export function getProjectRoot(): string {
  // When running from dist, we need to go up one level
  const isRunningFromDist = __dirname.includes('dist');
  return isRunningFromDist 
    ? path.resolve(__dirname, '..') 
    : path.resolve(__dirname);
}
