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
use super::source::{Markup, pieces};

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
            let Ok(span) = document.create_element("span") else {
                continue;
            };
            let _ = span.set_attribute("class", markup_class(piece.markup));
            // A field or command span carries the same key a concept
            // row and a page marker do, so the three highlight each
            // other; plain markup carries none and stays inert.
            if let Some(name) = &piece.name {
                let _ = span.set_attribute("data-field", name);
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
        // The spans go in one box, not straight into the row. A row is
        // a flex container so the relation can sit at its end, and a
        // span placed directly in it becomes a flex item — which is
        // what broke `account:` into `ac/co/un/t:` down the side.
        let Some(code) = div(document, "ncode") else {
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
            let _ = code.append_child(&element);
        }
        let _ = row.append_child(&code);
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

fn markup_class(markup: Markup) -> &'static str {
    match markup {
        Markup::Text => "m-text",
        Markup::Tag => "m-tag",
        Markup::Attribute => "m-attr",
        Markup::Value => "m-value",
        Markup::Punct => "m-punct",
        Markup::Comment => "m-comment",
        Markup::Field => "m-field",
        Markup::Command => "m-command",
    }
}

fn token_class(token: Token) -> &'static str {
    match token {
        Token::Head => "t-head",
        Token::Effect => "t-effect",
        Token::Anchor => "t-anchor",
        Token::Sigil => "t-sigil",
        Token::Key => "t-key",
        Token::Value => "t-value",
        Token::Number => "t-number",
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

/// Everything the inspector draws.
///
/// Colour follows the app's Bauhaus palette (`tonk-ui/styles.css`),
/// with its role assignments rather than a second set: keys take the
/// triangle (yellow — structural, eye-catching), strings the square
/// (red — grounded, literal), numbers and types the circle (blue —
/// abstract, receding), comments the closure grey, and the `!` effect
/// marker the alarm. Each is read through its `--tonk-*` variable with
/// the literal as a fallback, the way `tonk-tree` does, because the
/// overlay lives in a shadow root inside a guest and cannot count on
/// the sheet that defines them having been injected there.
///
/// Corners are square throughout, which is the palette's own rule
/// (`--tonk-code-radius: 0`).
pub const CSS: &str = "\
.panel { position: fixed; display: none; z-index: 5; flex-direction: column;
         width: min(72ch, calc(100vw - 16px)); max-height: min(72vh, 680px);
         pointer-events: auto; color: #e6e3de; background: #17171a;
         border: 1px solid #2c2a27; border-radius: 0;
         box-shadow: 0 10px 34px rgba(0,0,0,.5); }
.head { position: relative; display: flex; align-items: center; gap: 8px; cursor: move;
        padding: 9px 34px 9px 12px; user-select: none; border-bottom: 1px solid #2c2a27; }
.title { font-weight: 600; letter-spacing: .01em; white-space: nowrap; overflow: hidden;
         text-overflow: ellipsis; }
.close { position: absolute; top: 6px; right: 6px; width: 24px; height: 24px; padding: 0;
         font: 15px/24px inherit; color: #7a7268; cursor: pointer; background: transparent;
         border: 0; border-radius: 0; }
.close:hover { color: #e6e3de; background: #24231f; }
.toggles { display: flex; gap: 1px; padding: 8px 12px 0; flex-wrap: wrap; }
.toggle { min-height: 26px; padding: 5px 14px; font: inherit; color: #7a7268; cursor: pointer;
          background: #1e1d1b; border: 0; border-radius: 0; }
.toggle:hover { color: #e6e3de; background: #24231f; }
.toggle.on { color: #17171a; background: var(--tonk-triangle, #c89a2b); }
.transport { display: flex; align-items: center; gap: 1px; padding: 8px 12px; }
.control { min-width: 30px; min-height: 24px; padding: 3px 8px; font: inherit; color: #7a7268;
           cursor: pointer; background: #1e1d1b; border: 0; border-radius: 0; }
.control:hover { color: #e6e3de; background: #24231f; }
.control.on { color: #17171a; background: var(--tonk-square, #b94a3d); }
.position { margin-left: 10px; color: #7a7268; }
.body { overflow: auto; border-top: 1px solid #2c2a27; }
.section + .section { border-top: 1px solid #2c2a27; }
.section-head { display: flex; align-items: center; gap: 10px; flex-wrap: wrap;
                padding: 6px 12px; background: #1b1a18; }
.section-name { color: #7a7268; text-transform: uppercase; letter-spacing: .12em; }
.chips { display: flex; gap: 1px; flex-wrap: wrap; }
.facet, .command-chip { min-height: 22px; padding: 3px 10px; font: inherit; color: #7a7268;
                        cursor: pointer; background: #24231f; border: 0; border-radius: 0; }
.facet:hover, .command-chip:hover { color: #e6e3de; background: #302e29; }
.facet.on { color: #17171a; background: var(--tonk-circle, #3d6da8); }
.command-chip.on { color: #17171a; background: var(--tonk-alarm, #a8302a); }
.command-chip.inert { color: var(--tonk-alarm, #a8302a); text-decoration: line-through; }
.section-body { padding: 6px 0; }
.nline { display: flex; align-items: flex-start; gap: 14px; padding: 2px 12px;
         line-height: 1.55; }
.nline:hover { background: #1f1e1b; }
/* The code owns the line; `min-width: 0` is what stops the relation
   tag from squeezing it. */
.ncode { flex: 1 1 auto; min-width: 0; white-space: pre-wrap; overflow-wrap: anywhere; }
.relation { flex: none; max-width: 40%; overflow: hidden; text-overflow: ellipsis;
            white-space: nowrap; color: #55504a; }
.t-head { color: var(--tonk-triangle, #c89a2b); font-weight: 600; }
.t-effect { color: var(--tonk-alarm, #a8302a); font-weight: 600; }
.t-anchor { color: var(--tonk-triangle, #c89a2b); }
.t-sigil { color: #7a7268; }
.t-key { color: var(--tonk-triangle, #c89a2b); }
.t-value { color: var(--tonk-square, #b94a3d); }
.t-number { color: var(--tonk-circle, #3d6da8); }
.t-entity { color: var(--tonk-circle, #3d6da8); text-decoration: underline; }
.t-comment { color: var(--tonk-closure, #7a7268); font-style: italic; }
.t-plain { color: #8d877f; }
.source { padding: 6px 12px; line-height: 1.55; white-space: pre-wrap;
          overflow-wrap: anywhere; color: #8d877f; }
.source .m-text { color: #8d877f; }
.source .m-tag { color: var(--tonk-triangle, #c89a2b); }
.source .m-attr { color: var(--tonk-circle, #3d6da8); }
.source .m-value { color: var(--tonk-square, #b94a3d); }
.source .m-punct { color: #55504a; }
.source .m-comment { color: var(--tonk-closure, #7a7268); font-style: italic; }
.source .m-field { color: #17171a; background: var(--tonk-circle, #3d6da8); }
.source .m-command { color: #17171a; background: var(--tonk-alarm, #a8302a); }
.source [data-field]:hover { outline: 1px solid #e6e3de; }
.note { padding: 6px 12px; color: #7a7268; font-style: italic; }
";
