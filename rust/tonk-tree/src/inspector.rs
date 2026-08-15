//! The node inspector pane: the selected node's detail (full hash, size,
//! count, storage, separator) and, for a segment, its entries table.
//! Values are formatted so their type is legible; clicking a fact row
//! unfolds a detail view.

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Element, Event};

use crate::dom::{ElExt, clear, el};
use crate::key;
use crate::model::{Kind, TreeEntry};
use crate::web::{Shared, append_key_full, human_size};

fn pane(state: &Shared) -> Element {
    state
        .borrow()
        .shadow
        .query_selector(".inspector")
        .unwrap()
        .unwrap()
}

/// Render the inspector for the current selection.
pub fn render(state: &Shared) {
    let body = pane(state);
    clear(&body);

    let (hash, node) = {
        let s = state.borrow();
        let hash = s.selected.clone();
        let node = hash.as_ref().and_then(|h| s.nodes.get(h).cloned());
        (hash, node)
    };

    let Some(hash) = hash else {
        let _ = body.append_child(&el("div").class("empty").text("no node selected"));
        return;
    };
    let Some(node) = node else {
        let _ = body.append_child(&el("div").class("status").text("loading…"));
        return;
    };

    let title = if node.kind == Kind::Index {
        "Index node"
    } else {
        "Segment node"
    };
    let _ = body.append_child(&el("h2").text(title));

    // Full hash — not truncated; it's the identifier.
    let _ = body.append_child(&kv("hash", hash.strip_prefix('#').unwrap_or(&hash)));
    let _ = body.append_child(&size_kv(state, node.size));
    let count_label = if node.kind == Kind::Index {
        "children"
    } else {
        "entries"
    };
    let _ = body.append_child(&kv(count_label, &node.count.to_string()));
    if let Some(rank) = node.rank {
        let _ = body.append_child(&kv("rank", &rank.to_string()));
    }
    // Scale and novelty describe the node's SHAPE — how much the subtree
    // is estimated to hold, and how much is buffered on it rather than
    // written down. Both are what the root has to say about the tree.
    if let Some(scale) = node.scale {
        let _ = body.append_child(&kv("scale", &scale.to_string()));
    }
    if let Some(novelty) = node.novelty {
        let _ = body.append_child(&kv("novelty", &novelty.to_string()));
    }
    let _ = body.append_child(&kv(
        "storage",
        if node.cached {
            "local"
        } else {
            "remote (fetched on demand)"
        },
    ));

    // The configuration the node was written under, read off the node
    // itself rather than assumed from today's defaults.
    if !node.manifest.is_empty() {
        let _ = body.append_child(&el("h3").text("manifest"));
        for (label, value) in &node.manifest {
            let _ = body.append_child(&kv(label, &value.to_string()));
        }
    }

    if !node.bound_parts.is_empty() {
        let _ = body.append_child(&kv("separator", ""));
        let keyrow = el("div").class("keybytes");
        append_key_full(&keyrow, &node.bound_parts);
        let _ = body.append_child(&keyrow);
    }

    if node.kind == Kind::Segment {
        render_entries(state, &body, &hash);
    }
}

fn kv(k: &str, v: &str) -> Element {
    el("div")
        .class("kv")
        .child(&el("span").class("k").text(k))
        .child(&el("span").class("v").text(v))
}

fn size_kv(state: &Shared, size: u64) -> Element {
    let row = kv("size", &human_size(size));
    let max = state.borrow().max_size.max(1);
    let w = (80.0 * (size as f64 / max as f64)).max(2.0).round() as u64;
    let bar = el("span").class("sizebar").style(&format!("width: {w}px"));
    let _ = row.append_child(&bar);
    row
}

fn render_entries(state: &Shared, body: &Element, hash: &str) {
    let box_ = el("div").class("entries");
    let _ = box_.append_child(&el("div").class("status").text("loading entries…"));
    let _ = body.append_child(&box_);

    let state = state.clone();
    let hash = hash.to_owned();
    spawn_local(async move {
        let loader = state.borrow().loader.clone();
        let entries = loader.entries(&hash).await;
        // Bail if the selection moved on.
        if state.borrow().selected.as_deref() != Some(hash.as_str()) {
            return;
        }
        match entries {
            Ok(entries) => {
                clear(&box_);
                let _ = box_.append_child(
                    &el("div")
                        .class("k")
                        .text(&format!("{} entries", entries.len())),
                );
                let _ = box_.append_child(&entry_table(&entries));
            }
            Err(e) => {
                clear(&box_);
                let _ = box_.append_child(&el("div").class("err").text(&e));
            }
        }
    });
}

/// A table of the segment's entries: Entity · Attribute · Value, the
/// value formatted so its type reads from the text. A row click unfolds
/// a detail view with the type name, full value, and key bytes.
fn entry_table(entries: &[TreeEntry]) -> Element {
    let table = el("table");
    let thead = el("thead");
    let hr = el("tr");
    for h in ["Entity", "Attribute", "Value"] {
        let _ = hr.append_child(&el("th").text(h));
    }
    let _ = thead.append_child(&hr);
    let _ = table.append_child(&thead);

    let tbody = el("tbody");
    for entry in entries {
        let tr = el("tr").class(if entry.retracted {
            "entry removed"
        } else {
            "entry"
        });

        let ent = el("td").class("col-ent");
        if let Some(e) = &entry.entity {
            ent.set_text_content(Some(&short(e, 14)));
        } else if let Some(blob) = &entry.blob {
            // A blob-index row has no entity — it references content.
            ent.set_text_content(Some(&format!("blob:{}", short(blob, 10))));
        }
        let _ = tr.append_child(&ent);

        // A history, coverage or blob record has no attribute; naming
        // its index there says what the row IS, instead of leaving the
        // cell blank as though the record were malformed.
        let attr = match (&entry.attribute, &entry.ordering) {
            (Some(a), _) => el("td").class("col-attr").text(a),
            (None, Some(ordering)) => el("td").class("col-attr col-ordering").text(ordering),
            (None, None) => el("td").class("col-attr"),
        };
        let _ = tr.append_child(&attr);

        let val_td = el("td").class("col-val");
        if entry.retracted {
            val_td.set_text_content(Some("(retracted)"));
        } else if let (Some(v), Some(t)) = (&entry.value, &entry.type_name) {
            let formatted = key::format_value(v, t);
            // Color the value by type so entity / string / number parse
            // at a glance; entities also underlined (they are URIs).
            let span = el("span")
                .class(&format!("val val-{}", t.to_lowercase()))
                .text(&trunc(&formatted, 40));
            let _ = val_td.append_child(&span);
        } else if let Some(size) = entry.blob_size {
            val_td.set_text_content(Some(&human_size(size)));
        } else if entry.supersedes.is_some_and(|n| n > 0) {
            // A covering record's payload IS how much it supersedes.
            let n = entry.supersedes.unwrap_or(0);
            val_td.set_text_content(Some(&format!("covers {n}")));
        }
        let _ = tr.append_child(&val_td);

        // Detail row, hidden until the entry is clicked.
        let detail = el("tr").class("detail").attr("hidden", "");
        let dtd = el("td").attr("colspan", "3");
        let _ = dtd.append_child(&entry_detail(entry));
        let _ = detail.append_child(&dtd);

        let detail_c = detail.clone();
        let cb = Closure::<dyn FnMut(Event)>::new(move |_e: Event| {
            if detail_c.has_attribute("hidden") {
                let _ = detail_c.remove_attribute("hidden");
            } else {
                let _ = detail_c.set_attribute("hidden", "");
            }
        });
        let _ = tr.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref());
        cb.forget();

        let _ = tbody.append_child(&tr);
        let _ = tbody.append_child(&detail);
    }
    let _ = table.append_child(&tbody);
    table
}

/// The unfolded detail for one entry: type, full value, key bytes.
fn entry_detail(entry: &TreeEntry) -> Element {
    let box_ = el("div").class("entry-detail");
    if let Some(t) = &entry.type_name {
        let _ = box_.append_child(&kv("type", t));
    }
    if let Some(e) = &entry.entity {
        let _ = box_.append_child(&kv("entity", e.strip_prefix('#').unwrap_or(e)));
    }
    if let Some(v) = &entry.value {
        let t = entry.type_name.as_deref().unwrap_or("");
        let _ = box_.append_child(&kv("value", &key::format_value(v, t)));
    }
    if let Some(ordering) = &entry.ordering {
        let _ = box_.append_child(&kv("index", ordering));
    }
    // The claim version and its lineage. Zeroes are omitted: an ordinary
    // fact carries none of this, and printing `cause 0` on every row
    // would bury the records where it is the whole story.
    if let (Some(origin), Some(edition)) = (&entry.origin, entry.edition) {
        let _ = box_.append_child(&kv("version", &format!("{}@{edition}", short(origin, 12))));
    }
    for (label, value) in [
        ("cause", entry.cause),
        ("collapsed", entry.collapsed),
        ("supersedes", entry.supersedes),
    ] {
        if let Some(n) = value.filter(|n| *n > 0) {
            let _ = box_.append_child(&kv(label, &n.to_string()));
        }
    }
    if entry.retraction {
        let _ = box_.append_child(&kv("retraction", "yes"));
    }
    if let Some(spill) = &entry.spill {
        let _ = box_.append_child(&kv("spilled to", spill));
    }
    if let Some(blob) = &entry.blob {
        let _ = box_.append_child(&kv("blob", blob));
        if let Some(size) = entry.blob_size {
            let _ = box_.append_child(&kv("blob size", &human_size(size)));
        }
    }
    let keyrow = el("div").class("keybytes");
    append_key_full(&keyrow, &entry.key_parts);
    let _ = box_.append_child(&el("div").class("k").text("key"));
    let _ = box_.append_child(&keyrow);
    box_
}

fn short(s: &str, n: usize) -> String {
    let raw = s.strip_prefix('#').unwrap_or(s);
    raw.chars().take(n).collect()
}

fn trunc(s: &str, n: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() > n {
        let h: String = chars[..n].iter().collect();
        format!("{h}…")
    } else {
        s.to_owned()
    }
}
