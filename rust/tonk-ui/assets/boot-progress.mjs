// Trunk initializer for the ui wasm (wired via `data-initializer` on the
// `rel="rust"` link in index.html; see https://trunkrs.dev/assets/).
//
// Trunk calls these hooks around the main wasm fetch, which lets the static
// boot shell (`#tonk-boot`, also in index.html) show REAL download progress
// for the multi-megabyte bundle instead of a dead black viewport. The shell
// itself is hidden by CSS the moment the app mounts `<tonk-site>` (see the
// `body:has(tonk-site)` rule in styles.css) — no JS teardown here.
//
// Two degraded paths, both deliberate:
//   - The CDN serves the wasm zstd-compressed with no content-length, so
//     `total` may be 0/undefined. Then the bar keeps its CSS indeterminate
//     sweep and the status line counts megabytes instead of percent.
//   - If a (future) trunk version stops calling initializers, none of this
//     runs: the shell still renders with the sweep and a static status.
export default function initializer() {
    const fill = document.querySelector("[data-boot-fill]");
    const status = document.querySelector("[data-boot-status]");

    const set = (text) => {
        if (status) status.textContent = text;
    };
    const mib = (bytes) => (bytes / (1024 * 1024)).toFixed(1);

    return {
        onStart: () => set("downloading…"),
        onProgress: ({ current, total }) => {
            if (!total) {
                set(`downloading… ${mib(current)} MB`);
                return;
            }
            const pct = Math.min(100, Math.round((current / total) * 100));
            if (fill) {
                // Switching to determinate kills the sweep animation.
                fill.setAttribute("data-determinate", "");
                fill.style.width = `${pct}%`;
            }
            set(`downloading… ${pct}%`);
        },
        onComplete: () => {},
        onSuccess: () => set("starting…"),
        onFailure: (error) => {
            console.error("boot: wasm failed to load", error);
            if (fill) {
                fill.removeAttribute("data-determinate");
                fill.setAttribute("data-failed", "");
            }
            set("failed to load — check your connection and reload");
        },
    };
}
