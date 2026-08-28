// Trunk initializer for the ui wasm (wired via `data-initializer` on the
// `rel="rust"` link in index.html; see https://trunkrs.dev/assets/).
//
// Trunk calls these hooks around the main wasm fetch, which lets the static
// boot shell (`#tonk-boot`, also in index.html) show REAL download progress
// for the multi-megabyte bundle to assistive technology while the visual shell
// uses the shared pulse instead of a dead black viewport. The shell
// itself is hidden by CSS the moment the app mounts `<tonk-site>` (see the
// `body:has(tonk-site)` rule in styles.css) — no JS teardown here.
//
// Two degraded paths, both deliberate:
//   - The CDN serves the wasm zstd-compressed with no content-length, so
//     `total` may be 0/undefined. Then the hidden status counts megabytes
//     instead of percent.
//   - If a (future) trunk version stops calling initializers, none of this
//     runs: the shell still renders with the pulse and a static status.
export default function initializer() {
    const status = document.querySelector("[data-boot-status]");

    const set = (text) => {
        if (status) status.textContent = text;
    };
    const mib = (bytes) => (bytes / (1024 * 1024)).toFixed(1);

    // The boot watchdog (index.html) reloads a boot that shows no signs
    // of life; every callback here is one, so a slow download is never
    // mistaken for a wedge.
    const life = () => self.tonkBootLife?.();

    return {
        onStart: () => {
            life();
            set("downloading…");
        },
        onProgress: ({ current, total }) => {
            life();
            if (!total) {
                set(`downloading… ${mib(current)} MB`);
                return;
            }
            const pct = Math.min(100, Math.round((current / total) * 100));
            set(`downloading… ${pct}%`);
        },
        onComplete: () => life(),
        onSuccess: () => {
            life();
            set("starting…");
        },
        onFailure: (error) => {
            console.error("boot: wasm failed to load", error);
            status?.setAttribute("data-failed", "");
            set("failed to load — check your connection and reload");
            // A DETECTED failure needs no stall window: recover now.
            // Past the retry budget this repaints the failure line and
            // stops, so the message above still stands.
            self.tonkBootRecover?.(`wasm failed to load: ${error}`);
        },
    };
}
