// Shared theme initialization for the browser page and hosted space embeds.
(() => {
  // Activate the default theme with the shoelace palette. Light/dark
  // is toggled reactively on `prefers-color-scheme` changes.
  document.documentElement.classList.add('wa-theme-default', 'wa-palette-shoelace');

  //
  // A page shown inside an agent app follows the app's theme instead:
  // its host calls `__tonkSetTheme('dark' | 'light')`, and the system
  // preference no longer applies.
  const mq = window.matchMedia('(prefers-color-scheme: dark)');
  let chosen;
  const apply = (isDark) => {
    const cls = document.documentElement.classList;
    cls.toggle('wa-dark', isDark);
    cls.toggle('wa-light', !isDark);
  };
  apply(mq.matches);
  mq.addEventListener('change', (e) => chosen || apply(e.matches));
  window.__tonkSetTheme = (theme) => {
    if (theme !== 'dark' && theme !== 'light') return;
    chosen = theme;
    apply(theme === 'dark');
    document.documentElement.style.colorScheme = theme;
  };
  // A runtime that sets the classes again after the host chose keeps the
  // host's theme: only a change to the other one is undone.
  new MutationObserver(() => {
    if (!chosen) return;
    const dark = chosen === 'dark',
      cls = document.documentElement.classList;
    if (cls.contains('wa-dark') !== dark || cls.contains('wa-light') === dark) apply(dark);
  }).observe(document.documentElement, { attributes: true, attributeFilter: ['class'] });
})();
