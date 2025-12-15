export { default as Desktop } from "./components/Desktop";
export {
  clearThumbnailCache,
  invalidateThumbnailCache,
} from "./hooks/useThumbnail";
export type { DesktopState } from "./services/DesktopService";
export {
  getDesktopService,
  // TODO: Currently unused. Re-enable when bundle lifecycle management is needed.
  resetDesktopService,
} from "./services/DesktopService";
export type { DesktopFile, FileIconShapeProps } from "./types";
