(() => {
  "use strict";

  const stage = document.querySelector("#stage");
  const screen = new URLSearchParams(location.search).get("screen") || "WEB-07";
  // The account screens that still exist as markup this fixture can
  // serve: the hub's settings panel and its clusters. Sign-up, log-in,
  // and the link-an-account panels became the registration cluster the
  // hub raises, which is captured from the running product.
  const accountScreens = new Set(["WEB-10", "WEB-11", "WEB-12", "WEB-13"]);
  const activationScreens = new Set(["WEB-14", "WEB-15"]);

  function setText(host, selector, text) {
    const element = host.querySelector(selector);
    if (element) element.textContent = text;
  }

  function showPane(host, name) {
    host.querySelectorAll(".s-rail [data-pane]").forEach((tab) => {
      tab.classList.toggle("cur", tab.dataset.pane === name);
    });
    host.querySelectorAll(".s-body .pane").forEach((pane) => {
      pane.hidden = pane.dataset.pane !== name;
    });
  }

  function populateSettings(host) {
    setText(host, "[data-settings-email]", "alex@example.com");
    setText(host, "[data-settings-passkey-device]", "1Password");
    setText(host, "[data-settings-passkey-created]", "created 12 Aug 2026");
    const name = host.querySelector("[data-settings-name]");
    if (name) name.value = "Alex Rivera";
    const devices = host.querySelector("[data-settings-devices]");
    if (devices) devices.innerHTML = '<div class="srowd"><b class="lft">Safari on this Mac<span class="dev-self"> · this device</span></b><span class="dev-r"><span class="dev-when">linked 12 Aug 2026</span></span></div><div class="srowd"><b class="lft">Alex’s MacBook CLI</b><span class="dev-r"><span class="dev-when">linked 26 Aug 2026</span><button class="cta" type="button">remove access</button></span></div>';
  }

  async function accountFixture() {
    const markup = await fetch("../../../rust/tonk-workspace/src/ui_account_settings.html")
      .then((response) => response.text());
    const host = document.createElement("ui-account-settings");
    host.innerHTML = markup;
    stage.append(host);
    populateSettings(host);
    if (screen === "WEB-10") {
      showPane(host, "link");
      setText(host, "[data-link-name]", "Alex’s MacBook");
      setText(host, "[data-link-did]", "did:key:z6MkrP…w91f");
      return;
    }
    showPane(host, screen === "WEB-12" ? "devices" : "account");
    if (screen === "WEB-13") {
      const dialog = host.querySelector("[data-delete-account-dialog]");
      if (dialog && typeof dialog.show === "function") dialog.show();
      setText(host, "[data-delete-scope]", "2 owned hosted spaces will be deleted. 1 joined space will be left intact.");
      host.querySelector("[data-delete-spaces]").innerHTML = "<li>Product roadmap</li><li>Research library</li>";
      host.querySelector("[data-delete-email]").value = "alex@example.com";
    }
  }

  async function activationFixture() {
    const [markup, css] = await Promise.all([
      fetch("../../../rust/tonk-ui/src/activate.html").then((response) => response.text()),
      fetch("../../../rust/tonk-ui/src/account.css").then((response) => response.text()),
    ]);
    const style = document.createElement("style");
    style.textContent = css;
    document.head.append(style);
    const host = document.createElement("tonk-activate");
    host.innerHTML = markup;
    stage.append(host);
    const confirm = host.querySelector("#activate-confirm");
    const done = host.querySelector("#activate-done");
    if (screen === "WEB-14") {
      confirm.hidden = false;
      done.hidden = true;
    } else {
      confirm.hidden = true;
      done.hidden = false;
    }
  }

  async function cliFixture() {
    const response = await fetch(`cli/${screen.toLocaleLowerCase()}.txt`);
    if (!response.ok) throw new Error(`No CLI transcript for ${screen}`);
    const transcript = await response.text();
    stage.className = "terminal-stage";
    const window = document.createElement("section");
    window.className = "terminal-window";
    window.setAttribute("aria-label", `${screen} terminal capture`);
    const titlebar = document.createElement("div");
    titlebar.className = "terminal-titlebar";
    titlebar.innerHTML = '<span class="terminal-dot"></span><span class="terminal-dot"></span><span class="terminal-dot"></span><span class="terminal-title">tonk · 96 columns</span>';
    const pre = document.createElement("pre");
    pre.textContent = transcript;
    window.append(titlebar, pre);
    stage.append(window);
  }

  const load = accountScreens.has(screen)
    ? accountFixture()
    : activationScreens.has(screen)
      ? activationFixture()
      : screen.startsWith("CLI-")
        ? cliFixture()
        : Promise.reject(new Error(`${screen} must be captured from the running product`));

  load.then(() => document.documentElement.setAttribute("data-ready", "true"))
    .catch((error) => {
      stage.innerHTML = `<p class="fixture-error">${error.message}</p>`;
      document.documentElement.setAttribute("data-ready", "error");
    });
})();
