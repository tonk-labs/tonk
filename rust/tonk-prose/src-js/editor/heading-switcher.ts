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

import autocomplete, {
  ActionKind,
  closeAutocomplete,
  openAutocomplete,
  type AutocompleteAction,
} from "prosemirror-autocomplete";
import { Plugin, TextSelection } from "prosemirror-state";
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
  return [...found, { title: typed, href: "", spans: [], create: true }];
}

/** Is the selection inside the document's leading heading? */
function inLeadingHeading(view: EditorView): boolean {
  const { $head, empty } = view.state.selection;
  if (!empty) return false;
  if ($head.parent.type !== schema.nodes.heading) return false;
  // The FIRST block only. A `## Section` further down is structure, not the
  // document's name, and must not offer to navigate away.
  return $head.before($head.depth) === 0;
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

  const reducer = (action: AutocompleteAction): boolean => {
    switch (action.kind) {
      case ActionKind.open:
      case ActionKind.filter:
        suggest(action.filter ?? "");
        return true;
      case ActionKind.close:
        shown = [];
        active = 0;
        options.onSuggest(null, 0);
        return true;
      case ActionKind.up:
        if (shown.length === 0) return false;
        active = (active - 1 + shown.length) % shown.length;
        options.onSuggest(shown, active);
        return true;
      case ActionKind.down:
        if (shown.length === 0) return false;
        active = (active + 1) % shown.length;
        options.onSuggest(shown, active);
        return true;
      case ActionKind.enter:
        // Enter always has a visible target: the highlighted row, which is
        // either a notebook to open or the explicit "create this" row.
        return choose();
      default:
        return false;
    }
  };

  // Drive open/close from WHERE THE CARET IS, not from a trigger
  // character. The heading has no `@` or `/` to key on: being in it is the
  // trigger, so the plugin opens the autocomplete imperatively.
  const caret = new Plugin({
    view: () => ({
      update(view: EditorView, previous) {
        const now = inLeadingHeading(view);
        const before =
          previous.selection.empty &&
          previous.selection.$head.parent.type === schema.nodes.heading &&
          previous.selection.$head.before(previous.selection.$head.depth) === 0;
        if (now && !before) {
          openAutocomplete(view, "", headingTitle(view.state.selection.$head.parent));
        } else if (!now && before) {
          closeAutocomplete(view);
        }
      },
    }),
  });

  // Park the caret AFTER the `# ` marker on mount.
  //
  // The marker is literal text inside the heading, so the default position
  // 0 puts the caret before it: you type into `|# ` and get `x# `, which is
  // not a heading at all. This moves it past the marker once, when the
  // document is the untouched starting one.
  const park = new Plugin({
    view: (view) => {
      const { doc } = view.state;
      const first = doc.firstChild;
      if (first && first.type === schema.nodes.heading) {
        const end = 1 + first.content.size;
        view.dispatch(
          view.state.tr.setSelection(
            TextSelection.create(doc, Math.min(end, doc.content.size)),
          ),
        );
      }
      return {};
    },
  });

  return [...autocomplete({ reducer, triggers: [] }), caret, park];
}
