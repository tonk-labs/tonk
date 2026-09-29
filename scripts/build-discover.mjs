// Build installable seeds and Hub cards from the pinned, vendored catalog.
// Application notation is never parsed as ordinary YAML. Adaptations below
// are explicit and limited to the install wrapper and a redundant core anchor.
import { readFileSync, writeFileSync, mkdirSync, existsSync } from 'node:fs';
import { createHash } from 'node:crypto';

const root = new URL('../rust/tonk-core/assets/', import.meta.url);
const catalog = JSON.parse(readFileSync(new URL('discover/catalog.json', root)));
const check = process.argv.includes('--check');
function output(path, content) {
  if (existsSync(path) && readFileSync(path, 'utf8') === content) return;
  if (check) {
    throw new Error(`Stale generated asset: ${path}`);
  } else writeFileSync(path, content);
}
const escape = text => String(text).replace(/[&<>"']/g, char => ({
  '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;',
})[char]);
mkdirSync(new URL('discover/seeds/', root), { recursive: true });
const cards = [];
for (const template of catalog.templates) {
  const { slug, name, summary, description, author, entrypoint, license } = template;
  if (!/^[a-z0-9-]+$/.test(slug) || !/^[a-z0-9/-]+$/.test(entrypoint)) {
    throw new Error(`Invalid template identity: ${slug}`);
  }
  for (const [path, expected] of Object.entries(template.vendoredFiles)) {
    const bytes = readFileSync(new URL(`discover/templates/${slug}/${path}`, root));
    if (createHash('sha256').update(bytes).digest('hex') !== expected) {
      throw new Error(`Vendored source changed: ${slug}/${path}`);
    }
  }
  const sources = template.files.filter(file => !file.optional).map(file =>
    readFileSync(new URL(`discover/templates/${slug}/${file.file}`, root), 'utf8'));
  // These two exports repeat the standard component declaration. Keep its
  // pinned entity and descriptor, but let core supply the name during the
  // seed validator's combined analysis (which rejects duplicate anchors).
  if (slug === 'kanoodel' || slug === 'welcome') {
    const declaration = 'concept!: &component\n';
    if (sources[0].split(declaration).length !== 2) throw new Error(`Expected one component anchor in ${slug}`);
    sources[0] = sources[0].replace(declaration, 'concept!:\n');
  }
  // Starter space already supplies this exact home recipe. The other
  // manifests expect the installer's `--home` to supply it.
  if (slug !== 'starter-space') sources.push(`concept!: &space-home
  this: space:home
  description: The space home page.
  with:
    subject:
      description: The repository's subject DID.
      the: dialog.replica/subject
      as: entity
      cardinality: one

view!:
  this: space:home
  show:
    ui: |
      <tonk-display model=${entrypoint} />

name!:
  this: id:tonk/space
  entity: space:home
`);
  output(new URL(`discover/seeds/${slug}.yaml`, root), sources.join('\n\n'));
  const preview = template.images[0];
  const image = `/discover/templates/${slug}/${preview.file}`;
  const optional = template.files.some(file => file.optional)
    ? '<p class="space-create-help">Includes the app only. Optional demo music and artwork are not installed. Multiplayer needs your own relay.</p>' : '';
  cards.push(`<space-create class="template-card srow-wrap" data-template="${slug}">
  <button type="button" class="template-preview" data-template-details-open aria-label="Preview ${escape(name)}">
    <img src="${image}" alt="${escape(preview.alt)}" loading="lazy">
    <span class="template-caption"><strong>${escape(name)}</strong><span class="template-summary">${escape(summary)}</span><span class="template-footer"><small>by ${escape(author.name)}</small><span class="template-open">open <span aria-hidden="true">↗</span></span></span></span>
  </button>
  <tonk-dialog appearance="hub" data-template-details heading="${escape(name)}">
    <div class="space-create-form">
      <button type="button" class="template-expand" data-template-image-open aria-label="Expand ${escape(name)} preview">
        <img class="template-detail-image" src="${image}" alt="${escape(preview.alt)}" loading="lazy">
        <span>expand photo ↗</span>
      </button>
      <p class="space-create-help">${escape(description)}</p>
      <p class="space-create-help">By ${escape(author.name)} · ${escape(license)} · from Honky Tonks. This template includes code that runs in your new space. Your copy won't change the original.</p>
      ${optional}
    </div>
    <button class="m-cancel" slot="actions" type="button" data-dialog="close">back to discover</button>
    <button class="m-go" slot="actions" type="button" data-space-create-open>make a copy +</button>
  </tonk-dialog>
  <dialog class="template-photo-dialog" data-template-image aria-label="${escape(name)} preview">
    <button type="button" data-template-image-close aria-label="Close expanded photo">close ×</button>
    <img src="${image}" alt="${escape(preview.alt)}">
  </dialog>
  <tonk-dialog appearance="hub" data-space-create-dialog heading="Copy ${escape(name)}">
    <form id="copy-template-${slug}" class="space-create-form" data-space-create-form>
      <label class="space-create-field"><span>space name</span><input name="name" type="text" value="${escape(name)}" required maxlength="100" autocomplete="off"></label>
      <label class="space-create-field"><span>a short description <small>/ optional</small></span><textarea name="description" maxlength="240">${escape(summary.slice(0, 240))}</textarea></label>
      <input type="hidden" name="open" value="true">
      <input type="hidden" name="seed" value="/discover/seeds/${slug}.yaml">
      <p class="space-create-error" data-space-create-error role="alert"></p>
    </form>
    <button class="m-cancel" slot="actions" type="button" data-template-back>back to details</button>
    <button class="m-go" slot="actions" type="submit" form="copy-template-${slug}" data-space-create-submit><span data-copy-label>make a copy +</span><span data-copying-label>copying…</span></button>
  </tonk-dialog>
</space-create>`);
}
const profile = new URL('library/profile.yaml', root);
const start = '<!-- BEGIN VENDORED DISCOVER CARDS -->';
const end = '<!-- END VENDORED DISCOVER CARDS -->';
const original = readFileSync(profile, 'utf8');
if (!original.includes(start) || !original.includes(end)) throw new Error('Missing catalog markers');
const html = `<hub-collection class="template-grid">\n${cards.join('\n')}\n</hub-collection>`;
output(profile, original.replace(new RegExp(`${start}[\\s\\S]*?${end}`),
  () => `${start}\n${html.split('\n').map(line => line.trimEnd() ? '            ' + line.trimEnd() : '').join('\n')}\n            ${end}`));
