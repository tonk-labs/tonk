// The ordinary app's page capability. Bare diagnostics/chat routes must
// neither register a worker nor carry its connections.
import { serveCarrierRequests } from "./rtc.mjs";

if (!globalThis.tonkBareRoute && "serviceWorker" in navigator) {
    serveCarrierRequests();
}
