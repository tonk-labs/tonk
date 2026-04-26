#!/usr/bin/env node
// Generates rust/tonk-sigil/assets/sigils.svg from sigil-js's symbols/default.ts.
//
// Reads a path to a clone of https://github.com/urbit/sigil-js as argv[2].
// Emits an SVG with 256 <symbol id="sym-XX"> entries, one per byte value.
//
// Substitutions applied to each fragment:
//   @FG -> var(--sigil-fg, currentColor)
//   @BG -> var(--sigil-bg, transparent)
//   @SW -> var(--sigil-sw, 4)
//   @TR -> removed (each fragment's <g transform='@TR'> wrapper is dropped —
//          transforms live on the <use> element instead)
//
// The byte-value ordering follows Urbit's prefix/suffix tables, where the
// high nibble indexes the prefix table and the low nibble the suffix table.
// sigil-js's visual grid (4 cells) uses these at specific positions, but for
// our purposes the byte -> symbol mapping is arbitrary as long as every byte
// in [0, 255] maps to a distinct glyph. We use the suffix table for even byte
// positions and the prefix table for odd byte positions in the renderer; the
// sprite sheet contains both, namespaced by their syllable strings.
//
// Usage:
//   node scripts/gen-sprites.mjs /path/to/sigil-js

import { readFileSync, writeFileSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));

// Urbit's prefix and suffix tables (from urbit-ob src/internal/co.js).
// Each is 256 * 3 = 768 characters.
const PREFIXES = [
  "doz","mar","bin","wan","sam","lit","sig","hid","fid","lis","sog","dir","wac","sab","wis","sib",
  "rig","sol","dop","mod","fog","lid","hop","dar","dor","lor","hod","fol","rin","tog","sil","mir",
  "hol","pas","lac","rov","liv","dal","sat","lib","tab","han","tic","pid","tor","bol","fos","dot",
  "los","dil","for","pil","ram","tir","win","tad","bic","dif","roc","wid","bis","das","mid","lop",
  "ril","nar","dap","mol","san","loc","nov","sit","nid","tip","sic","rop","wit","nat","pan","min",
  "rit","pod","mot","tam","tol","sav","pos","nap","nop","som","fin","fon","ban","mor","wor","sip",
  "ron","nor","bot","wic","soc","wat","dol","mag","pic","dav","bid","bal","tim","tas","mal","lig",
  "siv","tag","pad","sal","div","dac","tan","sid","fab","tar","mon","ran","nis","wol","mis","pal",
  "las","dis","map","rab","tob","rol","lat","lon","nod","nav","fig","nom","nib","pag","sop","ral",
  "bil","had","doc","rid","moc","pac","rav","rip","fal","tod","til","tin","hap","mic","fan","pat",
  "tac","lab","mog","sim","son","pin","lom","ric","tap","fir","has","bos","bat","poc","hac","tid",
  "hav","sap","lin","dib","hos","dab","bit","bar","rac","par","lod","dos","bor","toc","hil","mac",
  "tom","dig","fil","fas","mit","hob","har","mig","hin","rad","mas","hal","rag","lag","fad","top",
  "mop","hab","nil","nos","mil","fop","fam","dat","nol","din","hat","nac","ris","fot","rib","hoc",
  "nim","lar","fit","wal","rap","sar","nal","mos","lan","don","dan","lad","dov","riv","bac","pol",
  "lap","tal","pit","nam","bon","ros","ton","fod","pon","sov","noc","sor","lav","mat","mip","fip",
];

const SUFFIXES = [
  "zod","nec","bud","wes","sev","per","sut","let","ful","pen","syt","dur","wep","ser","wyl","sun",
  "ryp","syx","dyr","nup","heb","peg","lup","dep","dys","put","lug","hec","ryt","tyv","syd","nex",
  "lun","mep","lut","sep","pes","del","sul","ped","tem","led","tul","met","wen","byn","hex","feb",
  "pyl","dul","het","mev","rut","tyl","wyd","tep","bes","dex","sef","wyc","bur","der","nep","pur",
  "rys","reb","den","nut","sub","pet","rul","syn","reg","tyd","sup","sem","wyn","rec","meg","net",
  "sec","mul","nym","tev","web","sum","mut","nyx","rex","teb","fus","hep","ben","mus","wyx","sym",
  "sel","ruc","dec","wex","syr","wet","dyl","myn","mes","det","bet","bel","tux","tug","myr","pel",
  "syp","ter","meb","set","dut","deg","tex","sur","fel","tud","nux","rux","ren","wyt","nub","med",
  "lyt","dus","neb","rum","tyn","seg","lyx","pun","res","red","fun","rev","ref","mec","ted","rus",
  "bex","leb","dux","ryn","num","pyx","ryg","ryx","fep","tyr","tus","tyc","leg","nem","fer","mer",
  "ten","lus","nus","syl","tec","mex","pub","rym","tuc","fyl","lep","deb","ber","mug","hut","tun",
  "byl","sud","pem","dev","lur","def","bus","bep","run","mel","pex","dyt","byt","typ","lev","myl",
  "wed","duc","fur","fex","nul","luc","len","ner","lex","rup","ned","lec","ryd","lyd","fen","wel",
  "nyd","hus","rel","rud","nes","hes","fet","des","ret","dun","ler","nyr","seb","hul","ryl","lud",
  "rem","lys","fyn","wer","ryc","sug","nys","nyl","lyn","dyn","dem","lux","fed","sed","bec","mun",
  "lyr","tes","mud","nyt","byr","sen","weg","fyr","mur","tel","rep","teg","pec","nel","nev","fes",
];

if (PREFIXES.length !== 256 || SUFFIXES.length !== 256) {
  throw new Error("syllable tables must have 256 entries each");
}

const sigilJsPath = process.argv[2];
if (!sigilJsPath) {
  console.error("usage: gen-sprites.mjs /path/to/sigil-js");
  process.exit(1);
}

const defaultTs = readFileSync(resolve(sigilJsPath, "src/symbols/default.ts"), "utf8");
// The file is `const index = { phoneme: "<g>...</g>", ... }; export default index`
// Extract the object literal via a lax parse: strip the wrapper and use JSON-ish.
const match = defaultTs.match(/const\s+index\s*=\s*(\{[\s\S]*\})\s*;\s*export/);
if (!match) {
  throw new Error("could not locate object literal in default.ts");
}
// Turn JS object syntax into JSON: the keys are bareword phonemes, values are
// single- or double-quoted strings. We quote all keys of form key: "...
const objSource = match[1].replace(/([a-z]{3}):"/g, '"$1":"');
const index = JSON.parse(objSource);

const keyCount = Object.keys(index).length;
if (keyCount !== 512) {
  // Sanity check: sigil-js has 256 prefixes + 256 suffixes worth of symbols,
  // one per unique phoneme. Some phonemes may be shared though.
  console.error(`note: default.ts has ${keyCount} entries (expected ~512)`);
}

// Build the sprite sheet. Each symbol is rendered as a **mask** that
// discriminates visible (@FG) from transparent (@BG) pixels, composited
// against a single filled rectangle. The result: holes and contrast
// linework become real transparency, not just a painted background.
//
// Substitutions inside the mask:
//   @FG -> white   (mask: visible -> rect shows through -> currentColor)
//   @BG -> black   (mask: invisible -> transparent in the output)
//   @SW -> stroke-width (preserved literally, works inside the mask)
//
// The outer symbol body is one <rect> filled with var(--sigil-fg,
// currentColor), masked by the composite. Single-color sigil.

const maskSubstitute = (fragment) =>
  fragment
    // Drop the wrapping <g transform='@TR'>...</g>; positioning happens
    // on the <use> element.
    .replace(/^<g transform='@TR'>/, "")
    .replace(/<\/g>$/, "")
    .replaceAll("@FG", "white")
    .replaceAll("@BG", "black")
    .replaceAll("@SW", "4");

const buildSymbol = (id, fragment) =>
  `<symbol id="${id}" viewBox="0 0 128 128">` +
  `<mask id="m-${id}" maskUnits="userSpaceOnUse" x="0" y="0" width="128" height="128">` +
  maskSubstitute(fragment) +
  `</mask>` +
  `<rect width="128" height="128" fill="var(--sigil-fg, currentColor)" mask="url(#m-${id})"/>` +
  `</symbol>`;

const symbols = [];
for (let i = 0; i < 256; i += 1) {
  const pfxKey = PREFIXES[i];
  const sfxKey = SUFFIXES[i];
  const pfx = index[pfxKey];
  const sfx = index[sfxKey];
  if (!pfx) throw new Error(`missing symbol for prefix ${pfxKey} (byte ${i})`);
  if (!sfx) throw new Error(`missing symbol for suffix ${sfxKey} (byte ${i})`);
  const hex = i.toString(16).padStart(2, "0");
  symbols.push(buildSymbol(`pfx-${hex}`, pfx));
  symbols.push(buildSymbol(`sfx-${hex}`, sfx));
}

const svg =
  `<?xml version="1.0" encoding="UTF-8"?>\n` +
  `<svg xmlns="http://www.w3.org/2000/svg" style="display:none" aria-hidden="true">\n` +
  symbols.join("\n") +
  `\n</svg>\n`;

const outPath = resolve(__dirname, "..", "assets", "sigils.svg");
writeFileSync(outPath, svg);
console.error(`wrote ${outPath} (${svg.length} bytes, ${symbols.length} symbols)`);
