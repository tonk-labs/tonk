//! Browser adapter for the source-independent projection evaluator.

use dialog_artifacts::Value;
use js_sys::{Function, Reflect};
use tonk_schema::projection::{
    ControlProperty, EventAction, EventMember, ProjectionInput, SourceRead, TargetMember,
};
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Element, Event, HtmlFormElement};

/// Projection input backed by one real DOM event and authored binding element.
pub struct DomInput<'a> {
    event: &'a Event,
    binding: &'a Element,
}

impl<'a> DomInput<'a> {
    /// Bind the adapter to the event and closest `data-on*` element.
    pub fn new(event: &'a Event, binding: &'a Element) -> Self {
        Self { event, binding }
    }

    fn form(&self) -> SourceReadObject {
        if self.binding.dyn_ref::<HtmlFormElement>().is_some() {
            return SourceReadObject::Present(self.binding.clone().into());
        }
        match Reflect::get(self.binding.as_ref(), &JsValue::from_str("form")) {
            Ok(value) if value.is_null() || value.is_undefined() => SourceReadObject::Missing,
            Ok(value) => SourceReadObject::Present(value),
            Err(error) => SourceReadObject::Failed(format!("form lookup failed: {error:?}")),
        }
    }
}

enum SourceReadObject {
    Present(JsValue),
    Missing,
    Failed(String),
}

impl ProjectionInput for DomInput<'_> {
    fn control(&self, name: &str, property: ControlProperty) -> SourceRead {
        let form = match self.form() {
            SourceReadObject::Present(form) => form,
            SourceReadObject::Missing => return SourceRead::Missing,
            SourceReadObject::Failed(message) => return SourceRead::ReadFailed(message),
        };
        let elements = match Reflect::get(&form, &JsValue::from_str("elements")) {
            Ok(value) if value.is_null() || value.is_undefined() => return SourceRead::Missing,
            Ok(value) => value,
            Err(error) => {
                return SourceRead::ReadFailed(format!("form.elements failed: {error:?}"));
            }
        };
        let named_item = match Reflect::get(&elements, &JsValue::from_str("namedItem"))
            .ok()
            .and_then(|value| value.dyn_into::<Function>().ok())
        {
            Some(function) => function,
            None => return SourceRead::ReadFailed("form.elements.namedItem unavailable".into()),
        };
        let control = match named_item.call1(&elements, &JsValue::from_str(name)) {
            Ok(value) if value.is_null() || value.is_undefined() => return SourceRead::Missing,
            Ok(value) => value,
            Err(error) => {
                return SourceRead::ReadFailed(format!("named control lookup failed: {error:?}"));
            }
        };
        let property = match property {
            ControlProperty::Value => "value",
            ControlProperty::Checked => "checked",
        };
        read_property(&control, property)
    }

    fn data(&self, name: &str) -> SourceRead {
        self.binding
            .get_attribute(&format!("data-{name}"))
            .map(|value| SourceRead::Present(Value::String(value)))
            .unwrap_or(SourceRead::Missing)
    }

    fn event(&self, member: EventMember) -> SourceRead {
        read_property(self.event.as_ref(), event_member_name(member))
    }

    fn detail(&self, member: &str) -> SourceRead {
        let detail = match Reflect::get(self.event.as_ref(), &JsValue::from_str("detail")) {
            Ok(value) if value.is_null() || value.is_undefined() => return SourceRead::Missing,
            Ok(value) => value,
            Err(error) => {
                return SourceRead::ReadFailed(format!("event.detail failed: {error:?}"));
            }
        };
        read_property(&detail, member)
    }

    fn target(&self, member: TargetMember) -> SourceRead {
        let Some(target) = self.event.target() else {
            return SourceRead::Missing;
        };
        let member = match member {
            TargetMember::Value => "value",
            TargetMember::Checked => "checked",
        };
        read_property(target.as_ref(), member)
    }
}

/// Execute projection actions synchronously, in declaration order.
pub fn apply_actions(event: &Event, actions: &[EventAction]) {
    for action in actions {
        match action {
            EventAction::PreventDefault => event.prevent_default(),
            EventAction::StopPropagation => event.stop_propagation(),
            EventAction::StopImmediatePropagation => event.stop_immediate_propagation(),
        }
    }
}

fn read_property(object: &JsValue, property: &str) -> SourceRead {
    match Reflect::get(object, &JsValue::from_str(property)) {
        Ok(value) if value.is_null() || value.is_undefined() => SourceRead::Missing,
        Ok(value) => js_scalar(value).unwrap_or_else(|| {
            SourceRead::ReadFailed(format!("property {property:?} is not a supported scalar"))
        }),
        Err(error) => SourceRead::ReadFailed(format!("property {property:?} failed: {error:?}")),
    }
}

fn js_scalar(value: JsValue) -> Option<SourceRead> {
    if let Some(value) = value.as_string() {
        return Some(SourceRead::Present(Value::String(value)));
    }
    if let Some(value) = value.as_bool() {
        return Some(SourceRead::Present(Value::Boolean(value)));
    }
    value
        .as_f64()
        .map(|value| SourceRead::Present(Value::Float(value)))
}

fn event_member_name(member: EventMember) -> &'static str {
    match member {
        EventMember::Type => "type",
        EventMember::Key => "key",
        EventMember::Code => "code",
        EventMember::Repeat => "repeat",
        EventMember::ShiftKey => "shiftKey",
        EventMember::CtrlKey => "ctrlKey",
        EventMember::AltKey => "altKey",
        EventMember::MetaKey => "metaKey",
        EventMember::Button => "button",
        EventMember::ClientX => "clientX",
        EventMember::ClientY => "clientY",
        EventMember::TimeStamp => "timeStamp",
    }
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    use web_sys::{EventInit, window};

    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    fn control_lookup_uses_exact_name_from_form_and_button_form() {
        let document = window().unwrap().document().unwrap();
        let form = document.create_element("form").unwrap();
        form.set_inner_html(
            r#"<input name="note-body" value=""><button type="submit">save</button>"#,
        );
        document.body().unwrap().append_child(&form).unwrap();
        let button = form.query_selector("button").unwrap().unwrap();
        let event = Event::new_with_event_init_dict("submit", &EventInit::new()).unwrap();

        assert_eq!(
            DomInput::new(&event, &form).control("note-body", ControlProperty::Value),
            SourceRead::Present(Value::String(String::new()))
        );
        assert_eq!(
            DomInput::new(&event, &button).control("note-body", ControlProperty::Value),
            SourceRead::Present(Value::String(String::new()))
        );
        assert_eq!(
            DomInput::new(&event, &button).control("noteBody", ControlProperty::Value),
            SourceRead::Missing,
            "control names are never camel-cased"
        );
    }
}
