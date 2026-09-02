// The heading as a document switcher.
//
// On the notebook INDEX — where no document exists yet — the leading
// heading is how you get to a notebook: type a title and it suggests the
// notebooks that match, Enter opens the one you pick, and typing a name
// nothing matches creates it.
//
// Inside an EXISTING notebook the heading is just the title, and editing it
// renames. That is why this plugin is installed per-editor rather than
// switching on some runtime condition: if the switcher were live in a real
// notebook, renaming one to a title that already exists would silently
// navigate the author away into that other notebook and abandon the
// document they were editing. A construction-time flag makes that
// impossible rather than merely unlikely.

import { Plugin, PluginKey, TextSelection } from "prosemirror-state";
import type { EditorState } from "prosemirror-state";
import type { EditorView } from "prosemirror-view";
import { schema } from "./schema";
import { fuzzy } from "./fuzzy";

/** One notebook the switcher can open. */
export type Candidate = {
  /** The notebook's title, as shown. */
  title: string;
  /** Where to go when it is chosen. */
  href: string;
};

export type SwitcherOptions = {
  /** The notebooks available to switch to, read fresh on each keystroke
   *  so a notebook created in another tab shows up without a reload. */
  candidates: () => Candidate[];
  /** Open an existing notebook. */
  onOpen: (candidate: Candidate) => void;
  /** Create a notebook with this title and go to it. */
  onCreate: (title: string) => void;
  /** Render the suggestion list. Called with the rows and the index of the
   *  highlighted one; called with `null` when the list closes. */
  onSuggest: (rows: Suggestion[] | null, active: number) => void;
};

/** A candidate that survived the filter, with what to highlight. */
export type Suggestion = Candidate & {
  /** Indices into the title the query matched, for highlighting. */
  spans: number[];
  /** The "create this" row, which has no notebook behind it yet. */
  create?: true;
};

/** Rank `candidates` against `query`, best first.
 *
 *  Fuzzy (subsequence), in the manner of Obsidian's quick switcher: `wkpl`
 *  finds "Weekly Planning". A substring test would make you remember how a
 *  title starts; a subsequence test only asks for the letters in order,
 *  which is how a half-remembered name actually surfaces. */
export function rank(candidates: Candidate[], query: string): Suggestion[] {
  return candidates
    .map((candidate) => ({ candidate, match: fuzzy(candidate.title, query) }))
    .filter(({ match }) => match !== null)
    .sort((a, b) => b.match!.score - a.match!.score)
    .map(({ candidate, match }) => ({ ...candidate, spans: match!.spans }));
}

/** The candidate whose title is exactly `query`, if any.
 *
 *  Exact (ignoring case and surrounding space), NOT "the first thing still
 *  showing": typing `Not` while `Notes` exists must create `Not` rather
 *  than open `Notes`. */
export function exact(
  candidates: Candidate[],
  query: string,
): Candidate | null {
  const needle = query.trim().toLowerCase();
  if (needle === "") return null;
  return (
    candidates.find((c) => c.title.trim().toLowerCase() === needle) ?? null
  );
}

/** The list the panel shows: the matches, then a "create this" row.
 *
 *  The create row is EXPLICIT, and last, the way Obsidian and Notion both
 *  do it. Without it, creating is the invisible consequence of Enter
 *  failing to match anything, and the author cannot tell which of the two
 *  things is about to happen. As a row it is a visible target with a
 *  visible label.
 *
 *  It is omitted when the query is blank (nothing to name) or exactly
 *  matches an existing notebook (that is an open, not a create). */
export function suggestions(
  candidates: Candidate[],
  query: string,
): Suggestion[] {
  const found = rank(candidates, query);
  const typed = query.trim();
  if (typed === "" || exact(candidates, query)) return found;
  // What you typed comes FIRST, as an ordinary row.
  //
  // Every row is the same shape — a name, and a small verb saying what
  // Enter will do with it — so creating is one more thing to pick rather
  // than a differently-shaped afterthought at the bottom. First, because
  // it is the thing you just wrote: the list grows underneath what you
  // are typing instead of pushing it around.
  return [{ title: typed, href: "", spans: [], create: true }, ...found];
}

/** The heading's TITLE: its text without the `# ` marker.
 *
 *  Markers are materialized as literal text carrying the `markup` mark
 *  (markup.ts), so `textContent` on a heading node is `"# Title"`, not
 *  `"Title"`. Filtering on the raw text searches for notebooks whose names
 *  contain a hash and a space, which is nothing — the whole list would
 *  silently stay empty. */
export function headingTitle(node: {
  descendants: (f: (n: MarkedNode) => boolean) => void;
}): string {
  let out = "";
  node.descendants((child) => {
    if (child.isText && !child.marks?.some((m) => m.type.name === "markup")) {
      out += child.text ?? "";
    }
    return true;
  });
  return out;
}

/** The parts of a text node this module reads. */
type MarkedNode = {
  isText?: boolean;
  text?: string | null;
  marks?: readonly { type: { name: string } }[];
};

/** The plugins implementing the switcher. */
export function headingSwitcher(options: SwitcherOptions): Plugin[] {
  let active = 0;
  let shown: Suggestion[] = [];
  // The query the panel currently reflects, so an update that changed
  // something else (a selection move, a remote patch) does not re-rank.
  let last: string | null = null;

  const suggest = (filter: string) => {
    shown = suggestions(options.candidates(), filter);
    if (active >= shown.length) active = 0;
    options.onSuggest(shown, active);
  };

  /** Take the highlighted row: open a notebook, or create one. */
  const choose = (): boolean => {
    const row = shown[active];
    if (!row) return false;
    if (row.create) options.onCreate(row.title);
    else options.onOpen(row);
    return true;
  };

  // One plugin for both jobs, with an explicit key.
  //
  // Two unkeyed `new Plugin({})` instances collide: ProseMirror derives the
  // same key for both and rejects the state with "Adding different
  // instances of a keyed plugin", which takes the editor down with it.
  //
  // It does two things:
  //
  //  - Parks the caret after the `# ` marker on mount. The marker is
  //    literal text inside the heading, so the default position 0 sits
  //    BEFORE it and typing produces `x# ` rather than a heading.
  //  - Opens and closes the autocomplete from WHERE THE CARET IS. The
  //    heading has no `@` or `/` to trigger on: being in it is the trigger.
  const inHeading = (state: EditorState): boolean => {
    const { $head, empty } = state.selection;
    return (
      empty &&
      $head.parent.type === schema.nodes.heading &&
      $head.before($head.depth) === 0
    );
  };

  const driver = new Plugin({
    key: new PluginKey("headingSwitcher"),
    props: {
      // Only while the caret is in the heading and rows are showing —
      // otherwise Enter and the arrows are the editor's, as usual.
      handleKeyDown(view, event) {
        if (!inHeading(view.state) || shown.length === 0) return false;
        switch (event.key) {
          case "ArrowDown":
            active = (active + 1) % shown.length;
            options.onSuggest(shown, active);
            return true;
          case "ArrowUp":
            active = (active - 1 + shown.length) % shown.length;
            options.onSuggest(shown, active);
            return true;
          case "Enter":
            return choose();
          case "Escape":
            shown = [];
            active = 0;
            last = null;
            options.onSuggest(null, 0);
            return true;
          default:
            return false;
        }
      },
    },
    view: (view) => {
      // Park once, on mount — but NOT synchronously.
      //
      // `view()` runs inside `new EditorView`, before the constructor has
      // finished. Dispatching there re-enters `dispatchTransaction`, whose
      // closure refers to the `view` binding still being initialized:
      // "ReferenceError: Cannot access 'i' before initialization", and the
      // editor never mounts. A microtask lands after the constructor
      // returns, when dispatching is safe.
      let alive = true;
      queueMicrotask(() => {
        // The view may be gone by now — a mount that is torn down in the
        // same tick (a re-render, a test) would otherwise dispatch into a
        // destroyed view and reject.
        if (!alive || !view.dom.isConnected) return;
        const first = view.state.doc.firstChild;
        if (!first || first.type !== schema.nodes.heading) return;
        // Only an untouched starting document: if the author already typed
        // or clicked, their caret is theirs.
        if (first.textContent.trim() !== "#") return;
        const end = Math.min(1 + first.content.size, view.state.doc.content.size);
        view.dispatch(
          view.state.tr.setSelection(TextSelection.create(view.state.doc, end)),
        );
      });
      return {
        destroy() {
          alive = false;
        },
        update(view: EditorView, previous: EditorState) {
          // Drive the suggestions from the heading's own text on EVERY
          // update, not from the autocomplete's filter tracking.
          //
          // That tracking is built around a trigger character: it opens on
          // the char and follows the run of text after it. Ours has no
          // trigger — being in the heading is the trigger — so it sees no
          // run to follow and emits no `filter` as you type. Reading the
          // heading directly is both simpler and exactly right, since the
          // heading IS the query.
          const now = inHeading(view.state);
          if (!now) {
            if (inHeading(previous)) {
              shown = [];
              active = 0;
              last = null;
              options.onSuggest(null, 0);
            }
            return;
          }
          const query = headingTitle(view.state.selection.$head.parent);
          if (query === last && inHeading(previous)) return;
          last = query;
          suggest(query);
        },
      };
    },
  });

  return [driver];
}
