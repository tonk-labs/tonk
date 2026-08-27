(() => {
  "use strict";

  const stage = document.querySelector("#stage");
  const screen = new URLSearchParams(location.search).get("screen") || "WEB-07";
  const accountScreens = new Set(["WEB-07", "WEB-08", "WEB-09", "WEB-10", "WEB-11", "WEB-12", "WEB-13"]);
  const activationScreens = new Set(["WEB-14", "WEB-15"]);

  function showOnly(host, selector) {
    host.querySelectorAll("#account-choice, #account-create, #account-link, #account-handoff, #account-success")
      .forEach((panel) => { panel.hidden = `#${panel.id}` !== selector; });
    const working = host.querySelector("#account-working");
    if (working) working.hidden = true;
  }

  function setText(host, selector, text) {
    const element = host.querySelector(selector);
    if (element) element.textContent = text;
  }

  function populateSettings(host) {
    setText(host, "#account-email-value", "alex@example.com");
    setText(host, "#account-passkey-device-value", "1Password");
    setText(host, "#account-passkey-created-value", "12 Aug 2026");
    setText(host, "#account-registration-value", "active");
    const name = host.querySelector("#account-display-name");
    if (name) {
      name.value = "Alex Rivera";
      name.disabled = false;
      name.setAttribute("aria-busy", "false");
    }
    const profiles = host.querySelector("#account-profile-list");
    if (profiles) profiles.innerHTML = '<li data-active><span><strong>Alex Rivera</strong><small>alex@example.com · this account</small></span></li><li><button type="button" data-activate="tonk-9c81"><span><strong>Studio account</strong><small>studio@example.com</small></span><span aria-hidden="true">→</span></button></li>';
  }

  async function accountFixture() {
    const [markup, css] = await Promise.all([
      fetch("../../../rust/tonk-ui/src/account.html").then((response) => response.text()),
      fetch("../../../rust/tonk-ui/src/account.css").then((response) => response.text()),
    ]);
    const style = document.createElement("style");
    style.textContent = css;
    document.head.append(style);
    const host = document.createElement("tonk-account");
    host.innerHTML = markup;
    stage.append(host);
    if (screen === "WEB-07") {
      showOnly(host, "#account-choice");
      return;
    }
    if (screen === "WEB-08") {
      showOnly(host, "#account-create");
      const input = host.querySelector("#account-email");
      if (input) input.value = "alex@example.com";
      return;
    }
    if (screen === "WEB-09") {
      showOnly(host, "#account-link");
      return;
    }
    if (screen === "WEB-10") {
      showOnly(host, "#account-handoff");
      setText(host, "#account-handoff-name", "Alex’s MacBook");
      setText(host, "#account-handoff-did", "did:key:z6MkrP…w91f");
      return;
    }
    showOnly(host, "#account-success");
    populateSettings(host);
    if (screen === "WEB-12") {
      host.querySelector("#account-pane-account").hidden = true;
      host.querySelector("#account-pane-devices").hidden = false;
      host.querySelector("#account-tab-account").setAttribute("aria-selected", "false");
      host.querySelector("#account-tab-devices").setAttribute("aria-selected", "true");
      host.querySelector("#account-tab-devices").removeAttribute("tabindex");
      const devices = host.querySelector("#account-device-list");
      devices.innerHTML = '<li><span><strong>Safari on this Mac</strong><small>added 12 Aug 2026 · this device</small></span></li><li><span><strong>Alex’s MacBook CLI</strong><small>added 26 Aug 2026</small></span><button type="button">remove</button></li><li><span><strong>Work browser</strong><small>added 19 Aug 2026</small></span><button type="button">remove</button></li>';
    }
    if (screen === "WEB-13") {
      const confirmation = host.querySelector("#account-confirmation");
      confirmation.hidden = false;
      setText(host, "#account-confirm-title", "delete account permanently");
      setText(host, "#account-confirm-body", "This permanently deletes the account and every hosted space it owns. Local and joined spaces are outside this deletion.");
      const arming = host.querySelector("#account-delete-arming");
      arming.hidden = false;
      setText(host, "#account-delete-scope", "2 hosted spaces will be deleted:");
      host.querySelector("#account-delete-spaces").innerHTML = "<li>Product roadmap</li><li>Research library</li>";
      host.querySelector("#account-delete-email").value = "alex@example.com";
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
