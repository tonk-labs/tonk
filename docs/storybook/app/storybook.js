(() => {
  "use strict";

  const data = window.STORYBOOK_DATA;
  if (!data) {
    document.body.innerHTML = "<main><h1>Storybook data is missing</h1><p>Run <code>python3 docs/storybook/scripts/build.py</code>.</p></main>";
    return;
  }

  const state = { view: "overview", query: "", screenFilter: "all", flowFilter: "all" };
  const byJourney = new Map(data.journeys.map((journey) => [journey.id, journey]));
  const byScreen = new Map(data.screens.map((screen) => [screen.id, screen]));
  const search = document.querySelector("#search");
  const screenDialog = document.querySelector("#screen-dialog");
  const flowDialog = document.querySelector("#flow-dialog");
  const cardTemplate = document.querySelector("#screen-card-template");

  const escapeHTML = (value) => String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");

  const searchable = (...values) => values.flat().join(" ").toLocaleLowerCase();
  const matches = (...values) => {
    const terms = state.query.toLocaleLowerCase().trim().split(/\s+/).filter(Boolean);
    const haystack = searchable(values);
    return terms.every((term) => haystack.includes(term));
  };

  function setText(selector, value) {
    document.querySelectorAll(selector).forEach((element) => { element.textContent = value; });
  }

  function setMetadata() {
    setText("[data-audit-commit]", data.auditCommit);
    setText("[data-visual-commit]", data.visualCommit);
    setText('[data-count="screens"]', data.screens.length);
    setText('[data-count="flows"]', data.journeys.length);
    setText('[data-count="gaps"]', data.bugs.length);
    setText('[data-metric="screens"]', data.screens.length);
    setText('[data-metric="journeys"]', data.journeys.length);
    setText('[data-metric="verification"]', data.verification.length);
    setText('[data-metric="bugs"]', data.bugs.length);
    const result = data.verificationResults;
    setText("[data-verification-summary]", `${result.pass} passed · ${result.unrun} unrun`);
  }

  function screenCard(screen) {
    const fragment = cardTemplate.content.cloneNode(true);
    const card = fragment.querySelector(".screen-card");
    const button = fragment.querySelector("button");
    const image = fragment.querySelector("img");
    image.src = screen.artifact.replace(/^app\//, "");
    image.alt = `${screen.name} screen`;
    fragment.querySelector(".screen-id").textContent = screen.id;
    fragment.querySelector(".screen-capture").textContent = screen.capture;
    fragment.querySelector("strong").textContent = screen.name;
    fragment.querySelector("small").textContent = screen.summary;
    button.setAttribute("aria-label", `Open ${screen.name}`);
    button.addEventListener("click", () => openScreen(screen.id, true));
    card.dataset.search = searchable(screen.id, screen.name, screen.area, screen.summary, screen.journey_ids);
    return fragment;
  }

  function renderOverviewScreens() {
    const target = document.querySelector("#overview-screens");
    target.replaceChildren(...data.screens.slice(0, 6).map(screenCard));
  }

  function renderFilters(targetSelector, values, current, onSelect) {
    const target = document.querySelector(targetSelector);
    const labels = ["all", ...values];
    target.replaceChildren(...labels.map((label) => {
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = label === "all" ? "All" : label;
      button.setAttribute("aria-pressed", String(label === current));
      button.addEventListener("click", () => onSelect(label));
      return button;
    }));
  }

  function renderScreens() {
    const areas = [...new Set(data.screens.map((screen) => screen.surface === "cli" ? "CLI" : screen.area))];
    renderFilters("#screen-filters", areas, state.screenFilter, (value) => {
      state.screenFilter = value;
      renderScreens();
    });
    const visible = data.screens.filter((screen) => {
      const group = screen.surface === "cli" ? "CLI" : screen.area;
      return (state.screenFilter === "all" || state.screenFilter === group)
        && matches(screen.id, screen.name, screen.area, screen.summary, screen.capture, screen.journey_ids);
    });
    setText("[data-screen-result-count]", visible.length);
    const groups = new Map();
    visible.forEach((screen) => {
      const key = screen.surface === "cli" ? "CLI" : screen.area;
      if (!groups.has(key)) groups.set(key, []);
      groups.get(key).push(screen);
    });
    const target = document.querySelector("#screen-groups");
    target.replaceChildren(...[...groups.entries()].map(([name, screens]) => {
      const section = document.createElement("section");
      section.className = "screen-group";
      const heading = document.createElement("div");
      heading.className = "group-title";
      heading.innerHTML = `<h2>${escapeHTML(name)}</h2><span>${screens.length} screen${screens.length === 1 ? "" : "s"}</span>`;
      const grid = document.createElement("div");
      grid.className = "screen-grid";
      grid.replaceChildren(...screens.map(screenCard));
      section.append(heading, grid);
      return section;
    }));
    document.querySelector("#screens-empty").hidden = visible.length !== 0;
  }

  function renderFlows() {
    const groups = [...new Set(data.journeys.map((journey) => journey.group))];
    renderFilters("#flow-filters", groups, state.flowFilter, (value) => {
      state.flowFilter = value;
      renderFlows();
    });
    const visible = data.journeys.filter((journey) =>
      (state.flowFilter === "all" || state.flowFilter === journey.group)
      && matches(journey.id, journey.group, journey.title, journey.variants, journey.evidence, journey.gaps)
    );
    setText("[data-flow-result-count]", visible.length);
    const grouped = new Map();
    visible.forEach((journey) => {
      if (!grouped.has(journey.group)) grouped.set(journey.group, []);
      grouped.get(journey.group).push(journey);
    });
    const target = document.querySelector("#flow-groups");
    target.replaceChildren(...[...grouped.entries()].map(([name, journeys]) => {
      const section = document.createElement("section");
      section.className = "flow-group";
      const heading = document.createElement("h2");
      heading.innerHTML = `${escapeHTML(name)} <span>${journeys.length}</span>`;
      const list = document.createElement("div");
      list.className = "flow-list";
      list.replaceChildren(...journeys.map((journey) => {
        const row = document.createElement("button");
        row.type = "button";
        row.className = "flow-row";
        row.innerHTML = `<span class="flow-id">${escapeHTML(journey.id)}</span><span class="flow-title">${escapeHTML(journey.title)}</span><span class="flow-evidence">${escapeHTML(journey.evidence)}</span>`;
        row.addEventListener("click", () => openFlow(journey.id, true));
        return row;
      }));
      section.append(heading, list);
      return section;
    }));
    document.querySelector("#flows-empty").hidden = visible.length !== 0;
  }

  function renderGaps() {
    const bugs = data.bugs.filter((bug) => matches(bug.id, bug.title, bug.severity, bug.area, bug.decision));
    setText("[data-gap-result-count]", bugs.length);
    const bugList = document.querySelector("#bug-list");
    bugList.replaceChildren(...bugs.map((bug) => {
      const item = document.createElement("article");
      item.className = "bug-item";
      item.innerHTML = `<span class="severity ${escapeHTML(bug.severity)}">${escapeHTML(bug.severity)}</span><div><strong>${escapeHTML(bug.id)} · ${escapeHTML(bug.title)}</strong><small>${escapeHTML(bug.area)} · ${escapeHTML(bug.decision)}</small></div>`;
      return item;
    }));
    const results = data.verificationResults;
    const total = data.verification.length || 1;
    document.querySelector("#verification-panel").innerHTML = `
      <div class="verification-chart">
        <div class="verification-bar" aria-label="${results.pass} passed, ${results.fail} failed, ${results.blocked} blocked, ${results.unrun} unrun">
          <span class="pass" style="width:${(results.pass / total) * 100}%"></span>
          <span class="fail" style="width:${(results.fail / total) * 100}%"></span>
          <span class="blocked" style="width:${(results.blocked / total) * 100}%"></span>
        </div>
        <dl>
          <div><dt>Passed</dt><dd>${results.pass}</dd></div>
          <div><dt>Failed</dt><dd>${results.fail}</dd></div>
          <div><dt>Blocked</dt><dd>${results.blocked}</dd></div>
          <div><dt>Not run</dt><dd>${results.unrun}</dd></div>
          <div><dt>Total observable checks</dt><dd>${data.verification.length}</dd></div>
        </dl>
      </div>`;
  }

  function screenLinks(screenIds) {
    if (!screenIds.length) return "<li>No canonical screen assigned.</li>";
    return screenIds.map((id) => `<li><button type="button" data-screen-link="${escapeHTML(id)}"><span>${escapeHTML(id)}</span><span>Open →</span></button></li>`).join("");
  }

  function openScreen(id, updateHash = false) {
    const screen = byScreen.get(id);
    if (!screen) return;
    if (updateHash && state.view !== "screens") showView("screens");
    const journeys = screen.journey_ids.map((journeyId) => byJourney.get(journeyId)).filter(Boolean);
    document.querySelector("#screen-dialog-body").innerHTML = `
      <div class="detail-hero"><img src="${escapeHTML(screen.artifact.replace(/^app\//, ""))}" alt="${escapeHTML(screen.name)} screen"></div>
      <div class="detail-copy">
        <div class="detail-meta"><span class="detail-pill">${escapeHTML(screen.id)}</span><span class="detail-pill">${escapeHTML(screen.surface)}</span><span class="detail-pill">${escapeHTML(screen.capture)}</span><span class="detail-pill">commit ${escapeHTML(data.visualCommit)}</span></div>
        <h2 id="screen-dialog-title">${escapeHTML(screen.name)}</h2>
        <p class="detail-summary">${escapeHTML(screen.summary)}</p>
        <div class="detail-columns">
          <section class="detail-column"><h3>Flows visible here</h3><ul class="detail-list">${journeys.map((journey) => `<li><button type="button" data-flow-link="${escapeHTML(journey.id)}"><span>${escapeHTML(journey.id)} · ${escapeHTML(journey.title)}</span><span>→</span></button></li>`).join("")}</ul></section>
          <section class="detail-column"><h3>Source ownership</h3><ul class="detail-list">${screen.source_paths.map((path) => `<li><a href="../../../${escapeHTML(path)}"><span>${escapeHTML(path)}</span><span>↗</span></a></li>`).join("")}</ul></section>
        </div>
      </div>`;
    document.querySelectorAll("#screen-dialog [data-flow-link]").forEach((button) => button.addEventListener("click", () => {
      screenDialog.close();
      openFlow(button.dataset.flowLink, true);
    }));
    if (!screenDialog.open) screenDialog.showModal();
    if (updateHash) history.replaceState(null, "", `#screens/${id}`);
  }

  function openFlow(id, updateHash = false) {
    const journey = byJourney.get(id);
    if (!journey) return;
    if (updateHash && state.view !== "flows") showView("flows");
    const screens = data.screens.filter((screen) => screen.journey_ids.includes(id));
    document.querySelector("#flow-dialog-body").innerHTML = `
      <div class="flow-detail">
        <span class="flow-id">${escapeHTML(journey.id)} · ${escapeHTML(journey.group)}</span>
        <h2 id="flow-dialog-title">${escapeHTML(journey.title)}</h2>
        <dl class="flow-facts">
          <div><dt>Starting variants</dt><dd>${escapeHTML(journey.variants)}</dd></div>
          <div><dt>Existing evidence</dt><dd>${escapeHTML(journey.evidence)}</dd></div>
          <div><dt>Missing or weak evidence</dt><dd>${escapeHTML(journey.gaps)}</dd></div>
        </dl>
        <section class="detail-column flow-screens"><h3>Canonical screens</h3><ul class="detail-list">${screenLinks(screens.map((screen) => screen.id))}</ul></section>
      </div>`;
    document.querySelectorAll("#flow-dialog [data-screen-link]").forEach((button) => button.addEventListener("click", () => {
      flowDialog.close();
      openScreen(button.dataset.screenLink, true);
    }));
    if (!flowDialog.open) flowDialog.showModal();
    if (updateHash) history.replaceState(null, "", `#flows/${id}`);
  }

  function showView(view, detailId) {
    const previousView = state.view;
    state.view = ["overview", "screens", "flows", "gaps"].includes(view) ? view : "overview";
    if (state.view === "overview" && previousView !== "overview") {
      state.query = "";
      search.value = "";
    }
    document.querySelectorAll(".view").forEach((element) => { element.hidden = element.dataset.view !== state.view; });
    document.querySelectorAll("[data-view-link]").forEach((link) => {
      if (link.dataset.viewLink === state.view) link.setAttribute("aria-current", "page");
      else link.removeAttribute("aria-current");
    });
    search.placeholder = state.view === "overview"
      ? "Search screens, flows, states, or IDs"
      : `Search ${state.view}`;
    renderCurrent();
    document.body.removeAttribute("data-menu-open");
    document.querySelector(".mobile-menu").setAttribute("aria-expanded", "false");
    if (detailId && state.view === "screens") openScreen(detailId);
    if (detailId && state.view === "flows") openFlow(detailId);
    if (previousView !== state.view) window.scrollTo(0, 0);
  }

  function renderCurrent() {
    if (state.view === "screens") renderScreens();
    if (state.view === "flows") renderFlows();
    if (state.view === "gaps") renderGaps();
  }

  function route() {
    const [view = "overview", detailId] = location.hash.replace(/^#/, "").split("/");
    showView(view, detailId);
  }

  document.querySelectorAll(".dialog-close").forEach((button) => button.addEventListener("click", () => button.closest("dialog").close()));
  [screenDialog, flowDialog].forEach((dialog) => {
    dialog.addEventListener("click", (event) => {
      if (event.target === dialog) dialog.close();
    });
    dialog.addEventListener("close", () => {
      if (location.hash.includes("/")) history.replaceState(null, "", `#${state.view}`);
    });
  });
  search.addEventListener("input", () => {
    state.query = search.value;
    if (state.view === "overview" && state.query.trim()) location.hash = "#screens";
    else renderCurrent();
  });
  document.addEventListener("keydown", (event) => {
    if (event.key === "/" && document.activeElement !== search) {
      event.preventDefault();
      search.focus();
    }
  });
  document.querySelector(".mobile-menu").addEventListener("click", (event) => {
    const open = !document.body.hasAttribute("data-menu-open");
    document.body.toggleAttribute("data-menu-open", open);
    event.currentTarget.setAttribute("aria-expanded", String(open));
  });
  document.querySelectorAll(".audience-grid a[data-query]").forEach((link) => link.addEventListener("click", () => {
    search.value = link.dataset.query;
    state.query = link.dataset.query;
  }));
  document.querySelectorAll(".audience-grid a[data-doc]").forEach((link) => link.addEventListener("click", (event) => {
    event.preventDefault();
    location.href = link.dataset.doc;
  }));
  window.addEventListener("hashchange", route);

  setMetadata();
  renderOverviewScreens();
  renderGaps();
  route();
})();
