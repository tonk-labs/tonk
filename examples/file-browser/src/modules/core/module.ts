/**
 * Basic type for a Module function
 */
export type ModuleFunction = {
  name: string;
  description: string;
  fn: Function;
};

/**
 * Helper to create a Module function (without runtime validation)
 */
export function createFunction<T extends (...args: any[]) => any>(
  name: string,
  description: string,
  fn: T
): ModuleFunction & { fn: T } {
  return {
    name,
    description,
    fn,
  };
}

/**
 * Creates a Module from a set of functions
 */
export function createModule<T extends Record<string, Function>>(
  functions: Array<ModuleFunction & { fn: Function }>
): T {
  const module = {} as Record<string, Function>;

  for (const func of functions) {
    module[func.name] = func.fn;
  }

  return module as T;
}
