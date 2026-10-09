// Shared theme initialization for the browser page and hosted space embeds.
(() => {
    // Activate the default theme with the shoelace palette. Light/dark
    // is toggled reactively on `prefers-color-scheme` changes.
    document.documentElement.classList.add(
        "wa-theme-default",
        "wa-palette-shoelace",
    );

    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const apply = (isDark) => {
        const cls = document.documentElement.classList;
        cls.toggle("wa-dark", isDark);
        cls.toggle("wa-light", !isDark);
    };
    apply(mq.matches);
    mq.addEventListener("change", (e) => apply(e.matches));
})();
