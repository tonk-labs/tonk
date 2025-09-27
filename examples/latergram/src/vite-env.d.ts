/// <reference types="vite/client" />
/// <reference types="vite/types/importMeta.d.ts" />

declare module '*.md' {
  const content: string;
  export default content;
}
