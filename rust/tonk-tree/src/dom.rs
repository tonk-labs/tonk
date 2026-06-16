//! Tiny DOM-construction helpers over `web_sys`, to keep the renderer
//! readable.

use web_sys::{Document, Element, window};

pub fn document() -> Document {
    window().unwrap().document().unwrap()
}

/// Create an element with a tag name.
pub fn el(tag: &str) -> Element {
    document().create_element(tag).unwrap()
}

/// Builder-ish helpers on `Element`.
pub trait ElExt {
    fn class(self, class: &str) -> Self;
    fn attr(self, name: &str, value: &str) -> Self;
    fn text(self, text: &str) -> Self;
    fn style(self, style: &str) -> Self;
    fn child(self, child: &Element) -> Self;
}

impl ElExt for Element {
    fn class(self, class: &str) -> Self {
        self.set_class_name(class);
        self
    }
    fn attr(self, name: &str, value: &str) -> Self {
        let _ = self.set_attribute(name, value);
        self
    }
    fn text(self, text: &str) -> Self {
        self.set_text_content(Some(text));
        self
    }
    fn style(self, style: &str) -> Self {
        let _ = self.set_attribute("style", style);
        self
    }
    fn child(self, child: &Element) -> Self {
        let _ = self.append_child(child);
        self
    }
}

/// Remove all children of an element.
pub fn clear(element: &Element) {
    element.set_inner_html("");
}
