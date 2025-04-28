enum EnvironmentType {
  Browser,
  Node,
  Unknown,
}

export function checkEnvironment(): EnvironmentType {
  if (typeof window !== 'undefined' && typeof window.document !== 'undefined') {
    return EnvironmentType.Browser;
  } else if (
    typeof process !== 'undefined' &&
    process.versions &&
    process.versions.node
  ) {
    return EnvironmentType.Node;
  } else {
    return EnvironmentType.Unknown;
  }
}
