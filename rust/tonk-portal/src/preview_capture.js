// Best-effort thumbnail of an already mounted home. No additional space mount,
// subscriptions, network fetches or computed-style walk. Missing assets are OK:
// the hub deliberately blurs this small image. Included in raw and runtime guests.
(function () {
  let lastInput = 0;
  for (const event of ['pointerdown', 'keydown', 'input']) {
    document.addEventListener(event, () => { lastInput = performance.now(); }, { passive: true });
  }
  async function capture() {
    const context = window.tonk.context;
    if (!context?.preview || document.hidden ||
        innerWidth < 300 || innerHeight < 160 || performance.now() - lastInput < 2000) return;
    // Let the actual content guest capture itself, not its iframe/chrome wrapper.
    if (document.querySelector('iframe, dialog[open], [data-state="loading"]')) return;
    if (!document.body?.textContent.trim()) return;
    const started = performance.now();
    // An inert document avoids running custom-element constructors while
    // copying the view. Capture must never remount application components.
    const snapshot = document.implementation.createHTMLDocument('');
    let nodes = 0;
    function budget() {
      if (nodes > 600 || performance.now() - started > 8) throw new Error('preview budget');
    }
    function clone(node) {
      nodes++;
      budget();
      if (node.nodeType === Node.TEXT_NODE) return snapshot.createTextNode(node.textContent.slice(0, 4000).replace(/[\x00-\x08\x0b\x0c\x0e-\x1f]/g, ''));
      if (node.nodeType !== Node.ELEMENT_NODE) return snapshot.createTextNode('');
      if (node.matches('script, style, link, template, [hidden], iframe, video, audio, input, textarea, select, [contenteditable]')) {
        return snapshot.createTextNode('');
      }
      if (node instanceof HTMLCanvasElement) {
        const img = snapshot.createElement('img');
        // Large/tainted canvases can be expensive or unreadable. Omit them.
        if (node.width * node.height <= 262144) {
          try { img.src = node.toDataURL(); } catch (_) { /* blank */ }
        }
        return img;
      }
      const out = snapshot.importNode(node, false);
      for (const attr of [...out.attributes]) {
        // Tonk's data-tonk-models carries record/unit separators. They are
        // legal in HTML DOM attributes but make SVG/XML undecodable.
        if (attr.value.length > 4096 || attr.name.startsWith('on') || ['src', 'srcset', 'href'].includes(attr.name) ||
            /[\x00-\x08\x0b\x0c\x0e-\x1f]/.test(attr.value) ||
            (!attr.namespaceURI && attr.name.includes(':'))) out.removeAttribute(attr.name);
      }
      if (out.hasAttribute('style')) out.setAttribute('style', out.getAttribute('style').replace(/url\([^)]*\)/gi, 'none'));
      // A single embedded full-size image must not consume the whole snapshot.
      if (node instanceof HTMLImageElement && node.currentSrc.length <= 32000 && node.currentSrc.startsWith('data:')) out.src = node.currentSrc;
      for (const child of (node.shadowRoot || node).childNodes) out.append(clone(child));
      return out;
    }
    const body = clone(document.body);
    let css = '', rules = 0;
    const cssRules = new Map();
    let cssBytes = 0;
    for (const sheet of document.styleSheets) {
      budget();
      try {
        for (const rule of sheet.cssRules) {
          budget();
          if (++rules > 2000) throw new Error('preview CSS budget');
          // Runtime fonts are embedded as large data URLs. Omit them before
          // counting bytes, rather than rejecting otherwise small views.
          if (rule.type === CSSRule.FONT_FACE_RULE || rule.type === CSSRule.IMPORT_RULE) continue;
          const text = rule.cssText.replace(/url\([^)]*\)/gi, 'none');
          // Runtime/component styles can repeat entire theme blocks. Keep the
          // last copy in cascade order rather than spending the budget twice.
          if (cssRules.has(text)) cssRules.delete(text);
          else cssBytes += text.length;
          cssRules.set(text, text);
          if (cssBytes > 256000) throw new Error('preview CSS budget');
        }
      }
      catch (error) { if (error.name !== 'SecurityError') throw error; }
    }
    css = [...cssRules.values()].join('');
    // No resource loading during capture. Blob fonts and remote images may be
    // absent; retain their layout and fall back to the browser's fonts.
    const style = snapshot.createElement('style'); style.textContent = css;
    body.prepend(style);
    const root = snapshot.createElement('div');
    root.className = document.documentElement.className;
    root.append(body);
    const width = Math.min(innerWidth, 1600), height = Math.min(innerHeight, 1000);
    const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}"><foreignObject width="100%" height="100%">${new XMLSerializer().serializeToString(root)}</foreignObject></svg>`;
    if (svg.length > 350000 || performance.now() - started > 12) return;
    const image = new Image();
    image.src = 'data:image/svg+xml;charset=utf-8,' + encodeURIComponent(svg);
    await image.decode();
    if (window.tonk.context !== context || document.hidden || performance.now() - lastInput < 2000) return;
    const canvas = document.createElement('canvas'); canvas.width = 256; canvas.height = 160;
    const painter = canvas.getContext('2d');
    painter.fillStyle = getComputedStyle(document.body).backgroundColor;
    painter.fillRect(0, 0, 256, 160);
    painter.drawImage(image, 0, 0, 256, 160);
    const blob = await new Promise(resolve => canvas.toBlob(resolve, 'image/webp', 0.6));
    if (!blob || blob.size > 32000 || window.tonk.context !== context) return;
    const data = await new Promise((resolve, reject) => {
      const reader = new FileReader(); reader.onload = () => resolve(reader.result);
      reader.onerror = reject; reader.readAsDataURL(blob);
    });
    await window.tonk.preview({ action: 'put', repo: context.preview, image: data });
  }
  function schedule(delay) {
    setTimeout(() => {
      const run = () => capture().catch(() => {}).finally(() => schedule(60000));
      // No timeout: don't force capture onto a busy main thread.
      if (window.requestIdleCallback) requestIdleCallback(run); else schedule(60000);
    }, delay);
  }
  window.tonk.ready.then(() => schedule(4000));
})();
