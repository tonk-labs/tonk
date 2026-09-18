//! The inspector: a floating bar over the observed display, and the
//! sections it unfolds.
//!
//! The bar names what is selected and offers one toggle per thing
//! there is to know about it — **data**, **model**, **view**,
//! **commands** — plus the transport. Sections are independent, not
//! tabs: the point of opening `view` is usually to read it against
//! `data`, and a tab bar makes that the one thing you cannot do.
//!
//! Every section renders notation rather than a table, because that is
//! what an author already reads and writes. The data section is the
//! entity as a `head!:` assertion, the model section is its
//! `concept!:` declaration, the view section is the template, and the
//! commands section is a `command!:` declaration per binding.
//!
//! All four key on the same thing: a **field name**. A line in the
//! data, a field in the declaration, a `{field}` in the template and a
//! marker on the page all carry `data-field`, so resting on any one of
//! them lights the others. Lines also carry `data-attribute` — the
//! dialog relation under the field name — which is the handle for
//! asking about the same relation elsewhere.

use std::collections::BTreeSet;

use wasm_bindgen::JsCast;
use web_sys::{Document, Element};

use super::command::Command;
use super::notation::{self, Line, Token};
use super::recorder::Timeline;
use super::slot::Snapshot;
use super::source::{Piece, pieces};

/// One thing the bar can unfold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Section {
    /// The entity, as the assertion that would make it.
    Data,
    /// The concept declaration the display resolved.
    Model,
    /// The view template, with a switcher over the model's facets.
    View,
    /// The commands the templates bind, and their declarations.
    Commands,
}

impl Section {
    /// Every section, in bar order.
    pub const ALL: [Section; 4] = [
        Section::Data,
        Section::Model,
        Section::View,
        Section::Commands,
    ];

    fn key(self) -> &'static str {
        match self {
            Section::Data => "data",
            Section::Model => "model",
            Section::View => "view",
            Section::Commands => "commands",
        }
    }

    fn of(key: &str) -> Option<Self> {
        Section::ALL
            .into_iter()
            .find(|section| section.key() == key)
    }
}

/// The inspector's DOM and what it currently shows.
pub struct Panel {
    root: Element,
    head: Element,
    title: Element,
    toggles: Element,
    transport: Element,
    body: Element,
    /// Sections the user has unfolded.
    open: BTreeSet<Section>,
    /// The facet the view section is showing.
    facet: Option<String>,
    /// The command whose declaration the commands section is showing.
    command: Option<String>,
    /// What the body was built for, so an unchanged frame does not
    /// rebuild it under the pointer.
    shown: Option<String>,
}

impl Panel {
    /// Build the inspector and attach it to `layer`. Hidden until shown.
    pub fn build(document: &Document, layer: &Element) -> Option<Self> {
        let root = div(document, "panel")?;
        let head = div(document, "head")?;
        let title = div(document, "title")?;
        let toggles = div(document, "toggles")?;
        let transport = div(document, "transport")?;
        let body = div(document, "body")?;

        let close = button(document, "close", "\u{00d7}")?;
        let _ = close.set_attribute("title", "stop observing");
        let _ = head.append_child(&title);
        let _ = head.append_child(&close);

        for section in Section::ALL {
            let toggle = button(document, "toggle", section.key())?;
            let _ = toggle.set_attribute("data-section", section.key());
            let _ = toggles.append_child(&toggle);
        }

        for (action, label, hint) in [
            ("step-back", "\u{25c0}", "step back one frame"),
            ("hold", "\u{23f8}", "hold updates"),
            ("step-forward", "\u{25b6}", "step forward one frame"),
            ("live", "\u{23ed}", "go live"),
        ] {
            let control = button(document, "control", label)?;
            let _ = control.set_attribute("data-action", action);
            let _ = control.set_attribute("title", hint);
            let _ = transport.append_child(&control);
        }
        let position = div(document, "position")?;
        let _ = transport.append_child(&position);

        for part in [&head, &toggles, &transport, &body] {
            let _ = root.append_child(part);
        }
        let _ = layer.append_child(&root);
        Some(Self {
            root,
            head,
            title,
            toggles,
            transport,
            body,
            open: BTreeSet::new(),
            facet: None,
            command: None,
            shown: None,
        })
    }

    /// The inspector root, for installing listeners on.
    pub fn root(&self) -> &Element {
        &self.root
    }

    /// The bar's title strip, which is also the drag handle.
    pub fn head(&self) -> &Element {
        &self.head
    }

    /// Unfold or fold a section.
    pub fn toggle(&mut self, section: Section) {
        if !self.open.remove(&section) {
            self.open.insert(section);
        }
        self.shown = None;
    }

    /// Show a particular facet in the view section.
    pub fn select_facet(&mut self, facet: &str) {
        self.facet = Some(facet.to_owned());
        self.shown = None;
    }

    /// Show a particular command's declaration, or fold it away.
    pub fn select_command(&mut self, name: &str) {
        self.command = if self.command.as_deref() == Some(name) {
            None
        } else {
            Some(name.to_owned())
        };
        self.shown = None;
    }

    /// The section a click landed on, if it landed on a toggle.
    pub fn section_of(target: &Element) -> Option<Section> {
        let button = target.closest(".toggle").ok().flatten()?;
        Section::of(&button.get_attribute("data-section")?)
    }

    /// The transport action a click landed on, if any.
    pub fn action_of(target: &Element) -> Option<String> {
        let button = target.closest(".control").ok().flatten()?;
        button.get_attribute("data-action")
    }

    /// The facet chip a click landed on, if any.
    pub fn facet_of(target: &Element) -> Option<String> {
        let chip = target.closest(".facet").ok().flatten()?;
        chip.get_attribute("data-facet")
    }

    /// The command chip a click landed on, if any.
    pub fn command_of(target: &Element) -> Option<String> {
        let chip = target.closest(".command-chip").ok().flatten()?;
        chip.get_attribute("data-command")
    }

    /// Stop showing anything.
    pub fn hide(&mut self) {
        let _ = self.root.set_attribute("style", "display:none");
        self.shown = None;
    }

    /// Draw the inspector.
    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        document: &Document,
        snapshot: &Snapshot,
        signature: &str,
        subject: Option<&str>,
        commands: &[Command],
        timeline: Timeline,
        truncated: bool,
        position: &str,
    ) {
        let _ = self
            .root
            .set_attribute("style", &format!("display:flex;{position}"));
        // The transport reflects a position that moves without the
        // frame changing shape, so it is written on every pass rather
        // than only on a rebuild.
        self.draw_transport(timeline);

        let key = format!(
            "{signature}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
            subject.unwrap_or_default(),
            self.open
                .iter()
                .map(|section| section.key())
                .collect::<Vec<_>>()
                .join(","),
            self.facet.clone().unwrap_or_default(),
            self.command.clone().unwrap_or_default(),
        );
        if self.shown.as_deref() == Some(key.as_str()) {
            return;
        }
        self.shown = Some(key);

        self.draw_title(snapshot, subject);
        self.draw_toggles();
        self.body.set_inner_html("");
        for section in Section::ALL {
            if !self.open.contains(&section) {
                continue;
            }
            match section {
                Section::Data => self.draw_data(document, snapshot, subject),
                Section::Model => self.draw_model(document, snapshot),
                Section::View => self.draw_view(document, snapshot),
                Section::Commands => self.draw_commands(document, commands, snapshot),
            }
        }
        if self.open.is_empty() {
            self.note(document, "pick one above to unfold it");
        }
        if truncated {
            // The page cannot say what it is not drawing, so the
            // inspector has to.
            self.note(document, "too many slots to mark them all on the page");
        }
    }

    fn draw_title(&self, snapshot: &Snapshot, subject: Option<&str>) {
        let model = snapshot
            .model_name
            .clone()
            .or_else(|| snapshot.model.clone())
            .or_else(|| snapshot.model_entity.clone())
            .unwrap_or_else(|| "?".to_owned());
        let facet = snapshot.facet.clone().unwrap_or_else(|| "?".to_owned());
        let mode = if snapshot.directory {
            format!("{} subjects", snapshot.subject_count())
        } else {
            "detail".to_owned()
        };
        self.title
            .set_text_content(Some(&format!("{model} \u{00b7} {facet} \u{00b7} {mode}")));
        let _ = self
            .title
            .set_attribute("title", subject.unwrap_or("no subject"));
    }

    fn draw_toggles(&self) {
        let Ok(buttons) = self.toggles.query_selector_all(".toggle") else {
            return;
        };
        for index in 0..buttons.length() {
            let Some(button) = buttons
                .item(index)
                .and_then(|node| node.dyn_into::<Element>().ok())
            else {
                continue;
            };
            let open = button
                .get_attribute("data-section")
                .and_then(|key| Section::of(&key))
                .is_some_and(|section| self.open.contains(&section));
            let _ = button.set_attribute("class", if open { "toggle on" } else { "toggle" });
        }
    }

    fn draw_transport(&self, timeline: Timeline) {
        if let Ok(Some(position)) = self.transport.query_selector(".position") {
            position.set_text_content(Some(&timeline.label()));
        }
        if let Ok(Some(hold)) = self.transport.query_selector("[data-action=hold]") {
            let _ = hold.set_attribute(
                "class",
                if timeline.is_held() {
                    "control on"
                } else {
                    "control"
                },
            );
        }
    }

    fn draw_data(&self, document: &Document, snapshot: &Snapshot, subject: Option<&str>) {
        let section = self.section(document, "data", None);
        let Some(section) = section else { return };
        let entity = subject.and_then(|subject| {
            snapshot
                .entities
                .iter()
                .find(|entity| entity.this == subject)
        });
        let Some(entity) = entity else {
            self.note_in(document, &section, "no subject to show");
            return;
        };
        let head = snapshot
            .model_name
            .clone()
            .or_else(|| snapshot.model.clone())
            .unwrap_or_else(|| "concept".to_owned());
        let relations = snapshot
            .descriptor
            .as_deref()
            .map(notation::relations)
            .unwrap_or_default();
        let document_lines = notation::entity(&head, &entity.this, &entity.values, &relations);
        draw_notation(document, &section, &document_lines);
    }

    fn draw_model(&self, document: &Document, snapshot: &Snapshot) {
        let Some(section) = self.section(document, "model", None) else {
            return;
        };
        let Some(descriptor) = snapshot.descriptor.as_deref() else {
            self.note_in(document, &section, "no concept resolved");
            return;
        };
        let lines = notation::declaration("concept", snapshot.model_name.as_deref(), descriptor);
        draw_notation(document, &section, &lines);
    }

    fn draw_view(&self, document: &Document, snapshot: &Snapshot) {
        let showing = self
            .facet
            .clone()
            .filter(|facet| snapshot.facets.contains_key(facet))
            .or_else(|| snapshot.facet.clone())
            .unwrap_or_default();
        // Chips for every facet the model declares, not just the one
        // rendered — the switcher is the answer to "what else could
        // this show?".
        let chips = div(document, "chips");
        if let Some(chips) = &chips {
            for facet in snapshot.facets.keys() {
                let Some(chip) = button(
                    document,
                    if *facet == showing {
                        "facet on"
                    } else {
                        "facet"
                    },
                    facet,
                ) else {
                    continue;
                };
                let _ = chip.set_attribute("data-facet", facet);
                let _ = chips.append_child(&chip);
            }
        }
        let Some(section) = self.section(document, "view", chips) else {
            return;
        };
        let template = snapshot
            .facets
            .get(&showing)
            .cloned()
            .or_else(|| snapshot.template.clone());
        let Some(template) = template else {
            self.note_in(document, &section, "no view template mounted");
            return;
        };
        let Some(source) = div(document, "source") else {
            return;
        };
        for piece in pieces(&template) {
            let (class, field) = match &piece {
                Piece::Literal { .. } => ("lit", None),
                Piece::Field { name, .. } => ("ref", Some(name.clone())),
                Piece::Command { name, .. } => ("cmd", Some(name.clone())),
            };
            let Ok(span) = document.create_element("span") else {
                continue;
            };
            let _ = span.set_attribute("class", class);
            if let Some(field) = field {
                let _ = span.set_attribute("data-field", &field);
            }
            span.set_text_content(Some(piece.text()));
            let _ = source.append_child(&span);
        }
        let _ = section.append_child(&source);
    }

    fn draw_commands(&self, document: &Document, commands: &[Command], snapshot: &Snapshot) {
        let chips = div(document, "chips");
        if let Some(chips) = &chips {
            for command in commands {
                let selected = self.command.as_deref() == Some(command.command.as_str());
                let class = match (selected, command.is_live()) {
                    (true, _) => "command-chip on",
                    (false, true) => "command-chip",
                    (false, false) => "command-chip inert",
                };
                let Some(chip) = button(document, class, &command.command) else {
                    continue;
                };
                let _ = chip.set_attribute("data-command", &command.command);
                let _ = chip.set_attribute("data-field", &command.command);
                let _ = chip.set_attribute(
                    "title",
                    &match &command.event_type {
                        Some(event) => format!("fires on {event}"),
                        None => "no listener: its declaration did not resolve".to_owned(),
                    },
                );
                let _ = chips.append_child(&chip);
            }
        }
        let Some(section) = self.section(document, "commands", chips) else {
            return;
        };
        if commands.is_empty() {
            self.note_in(document, &section, "this view binds no commands");
            return;
        }
        let Some(name) = self.command.clone() else {
            self.note_in(document, &section, "pick a command to see its declaration");
            return;
        };
        let definition = snapshot
            .commands
            .iter()
            .find(|definition| definition.name == name);
        match definition.and_then(|definition| definition.descriptor.as_deref()) {
            Some(descriptor) => {
                let lines = notation::declaration("command", Some(&name), descriptor);
                draw_notation(document, &section, &lines);
            }
            None => self.note_in(
                document,
                &section,
                "this name resolved to nothing, so nothing listens for it",
            ),
        }
    }

    /// Append a titled section to the body and return its content box.
    fn section(&self, document: &Document, name: &str, chips: Option<Element>) -> Option<Element> {
        let section = div(document, "section")?;
        let header = div(document, "section-head")?;
        let label = div(document, "section-name")?;
        label.set_text_content(Some(name));
        let _ = header.append_child(&label);
        if let Some(chips) = chips {
            let _ = header.append_child(&chips);
        }
        let content = div(document, "section-body")?;
        let _ = section.append_child(&header);
        let _ = section.append_child(&content);
        let _ = self.body.append_child(&section);
        Some(content)
    }

    fn note(&self, document: &Document, text: &str) {
        if let Some(note) = div(document, "note") {
            note.set_text_content(Some(text));
            let _ = self.body.append_child(&note);
        }
    }

    fn note_in(&self, document: &Document, parent: &Element, text: &str) {
        if let Some(note) = div(document, "note") {
            note.set_text_content(Some(text));
            let _ = parent.append_child(&note);
        }
    }
}

/// Render a notation document: one row per line, spans coloured by
/// token, each row keyed to its field and relation so the highlight
/// reaches the page and the other sections.
fn draw_notation(document: &Document, parent: &Element, lines: &notation::Document) {
    let Some(block) = div(document, "notation") else {
        return;
    };
    for line in &lines.lines {
        let Some(row) = div(document, "nline") else {
            continue;
        };
        if let Some(field) = &line.field {
            let _ = row.set_attribute("data-field", field);
        }
        if let Some(attribute) = &line.attribute {
            let _ = row.set_attribute("data-attribute", attribute);
            let _ = row.set_attribute("title", attribute);
        }
        for span in &line.spans {
            let Ok(element) = document.create_element("span") else {
                continue;
            };
            let _ = element.set_attribute("class", token_class(span.token));
            element.set_text_content(Some(&span.text));
            let _ = row.append_child(&element);
        }
        // The relation, dim and to the right: the field name is local
        // to the concept, the attribute is the fact that was stored.
        if let Some(attribute) = &line.attribute
            && line.field.is_some()
            && is_leading(line)
            && let Some(tag) = div(document, "relation")
        {
            tag.set_text_content(Some(attribute));
            let _ = row.append_child(&tag);
        }
        let _ = block.append_child(&row);
    }
    let _ = parent.append_child(&block);
}

/// Whether a line is the first of its field — the one the relation tag
/// belongs on, so a list does not repeat it per item.
fn is_leading(line: &Line) -> bool {
    line.spans
        .iter()
        .any(|span| span.token == Token::Key && span.text.starts_with(char::is_alphabetic))
}

fn token_class(token: Token) -> &'static str {
    match token {
        Token::Head => "t-head",
        Token::Anchor => "t-anchor",
        Token::Key => "t-key",
        Token::Value => "t-value",
        Token::Entity => "t-entity",
        Token::Comment => "t-comment",
        Token::Plain => "t-plain",
    }
}

/// The field a hovered part of the inspector refers to — a notation
/// line, a marked span in a template, a command chip. All carry
/// `data-field`, so one lookup serves every section.
pub fn field_of(target: &Element) -> Option<String> {
    let holder = target.closest("[data-field]").ok().flatten()?;
    holder.get_attribute("data-field")
}

fn div(document: &Document, class: &str) -> Option<Element> {
    let element = document.create_element("div").ok()?;
    let _ = element.set_attribute("class", class);
    Some(element)
}

fn button(document: &Document, class: &str, text: &str) -> Option<Element> {
    let element = document.create_element("button").ok()?;
    let _ = element.set_attribute("class", class);
    element.set_text_content(Some(text));
    Some(element)
}

/// Everything the inspector draws. Concatenated into the overlay's sheet.
pub const CSS: &str = "\
.panel { position: fixed; display: none; z-index: 5; flex-direction: column;
         width: min(58ch, calc(100vw - 16px)); max-height: min(70vh, 640px);
         pointer-events: auto; color: #e9e9ee; background: rgba(18,18,22,.97);
         border: 1px solid rgba(255,255,255,.10); border-radius: 6px; overflow: hidden;
         box-shadow: 0 10px 34px rgba(0,0,0,.5); }
.head { position: relative; display: flex; align-items: center; gap: 8px; cursor: move;
        padding: 7px 32px 7px 10px; user-select: none;
        border-bottom: 1px solid rgba(255,255,255,.10); }
.title { font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.close { position: absolute; top: 4px; right: 4px; width: 24px; height: 24px; padding: 0;
         font: 15px/24px inherit; color: inherit; cursor: pointer; background: transparent;
         border: 0; border-radius: 3px; }
.close:hover { background: rgba(255,255,255,.14); }
.toggles { display: flex; gap: 4px; padding: 6px 10px; flex-wrap: wrap; }
.toggle { min-height: 24px; padding: 4px 12px; font: inherit; color: #9aa0ad; cursor: pointer;
          background: rgba(255,255,255,.06); border: 0; border-radius: 4px; }
.toggle:hover { color: #e9e9ee; background: rgba(255,255,255,.12); }
.toggle.on { color: #0b0b0e; background: #7aa2ff; }
.transport { display: flex; align-items: center; gap: 4px; padding: 0 10px 6px; }
.control { min-width: 26px; min-height: 22px; padding: 2px 6px; font: inherit; color: #9aa0ad;
           cursor: pointer; background: rgba(255,255,255,.06); border: 0; border-radius: 4px; }
.control:hover { color: #e9e9ee; background: rgba(255,255,255,.12); }
.control.on { color: #0b0b0e; background: #e8b339; }
.position { margin-left: 6px; color: #6f7684; }
.body { overflow: auto; border-top: 1px solid rgba(255,255,255,.08); }
.section + .section { border-top: 1px solid rgba(255,255,255,.08); }
.section-head { display: flex; align-items: center; gap: 8px; flex-wrap: wrap;
                padding: 5px 10px; background: rgba(255,255,255,.03); }
.section-name { color: #6f7684; text-transform: uppercase; letter-spacing: .08em; }
.chips { display: flex; gap: 4px; flex-wrap: wrap; }
.facet, .command-chip { min-height: 20px; padding: 2px 8px; font: inherit; color: #9aa0ad;
                        cursor: pointer; background: rgba(255,255,255,.06); border: 0;
                        border-radius: 3px; }
.facet:hover, .command-chip:hover { color: #e9e9ee; background: rgba(255,255,255,.14); }
.facet.on { color: #0b0b0e; background: #7aa2ff; }
.command-chip.on { color: #0b0b0e; background: #f06595; }
.command-chip.inert { color: #c92a2a; text-decoration: line-through; }
.section-body { padding: 4px 0; }
.notation { padding: 2px 0; }
.nline { display: flex; gap: 8px; padding: 1px 10px; white-space: pre-wrap;
         word-break: break-word; }
.nline:hover { background: rgba(255,255,255,.08); }
.nline .relation { margin-left: auto; color: #4d535e; white-space: nowrap; }
.t-head { color: #7aa2ff; font-weight: 600; }
.t-anchor { color: #e8b339; }
.t-key { color: #22a06b; }
.t-value { color: #e9e9ee; }
.t-entity { color: #b197fc; }
.t-comment { color: #6f7684; font-style: italic; }
.t-plain { color: #9aa0ad; }
.source { padding: 4px 10px; white-space: pre-wrap; word-break: break-word; color: #9aa0ad; }
.source .ref { color: #22a06b; background: color-mix(in srgb, #22a06b 18%, transparent);
               border-radius: 2px; }
.source .cmd { color: #f06595; background: color-mix(in srgb, #d6336c 20%, transparent);
               border-radius: 2px; }
.source [data-field]:hover { outline: 1px solid currentColor; }
.note { padding: 5px 10px; color: #6f7684; }
";
