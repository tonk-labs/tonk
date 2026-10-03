//! Waiting on the FAB without a clock.
//!
//! The browser runs many test tabs at once, so a timer says nothing about
//! how far a transition has got or whether a rendering-step callback
//! (`transitionend`, a `ResizeObserver`, `scroll`) has run. These wait on
//! the thing itself: an event the FAB dispatches when a step lands, or the
//! animation's own end. The one timeout here is a safety net for an event
//! that never comes; reaching it is a failure, not a slow machine.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#![allow(dead_code)]

use js_sys::{Function, Promise};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{EventTarget, HtmlElement};

/// Long enough that only a missing event reaches it.
const SAFETY_MS: u32 = 20_000;

/// The next `name` event on `target`. Call it before the step that
/// dispatches the event (a step may dispatch it synchronously), then await
/// it after.
pub fn next_event(target: &EventTarget, name: &str) -> JsFuture {
    let listen = Function::new_with_args(
        "target, name, safety",
        "return new Promise((resolve, reject) => {
            const timer = setTimeout(() => reject(new Error(`no ${name} event`)), safety);
            target.addEventListener(name, (event) => { clearTimeout(timer); resolve(event); }, { once: true });
        });",
    );
    let promise: Promise = listen
        .call3(
            &JsValue::NULL,
            target,
            &name.into(),
            &JsValue::from(SAFETY_MS),
        )
        .expect("listen")
        .unchecked_into();
    JsFuture::from(promise)
}

/// Await `event`, failing the test with what was being waited for.
pub async fn arrived(event: JsFuture, what: &str) {
    if let Err(error) = event.await {
        panic!("{what}: {error:?}");
    }
}

/// The animations still playing on the FAB that end on their own: its
/// shadow tree's, and the host's own (the edge glide moves the host, which
/// is outside that tree). Not an infinite one (a spinner never ends), and
/// not a finished one a fill keeps in effect.
fn finite_animations(fab: &HtmlElement) -> js_sys::Array {
    Function::new_with_args(
        "fab",
        "fab.getBoundingClientRect();
         return [...fab.getAnimations(), ...fab.shadowRoot.getAnimations()].filter(
             (animation) => animation.playState === 'running'
                 && Number.isFinite(animation.effect?.getComputedTiming().endTime)
         );",
    )
    .call1(&JsValue::NULL, fab)
    .expect("read animations")
    .unchecked_into()
}

/// Jump every finite animation in the FAB to its end state, as if it had
/// run to completion. Returns how many there were. Whatever the FAB does on
/// their end (`transitionend`) still follows, a rendering step later.
pub fn finish_animations(fab: &HtmlElement) -> u32 {
    let animations = finite_animations(fab);
    for animation in animations.iter() {
        js_sys::Reflect::get(&animation, &"finish".into())
            .expect("finish")
            .unchecked_into::<Function>()
            .call0(&animation)
            .expect("finish the animation");
    }
    animations.length()
}

/// Wait until every finite animation in the FAB has run to its end on its
/// own timeline, including any that start as others end.
pub async fn animations_settled(fab: &HtmlElement) {
    for _ in 0..20 {
        let animations = finite_animations(fab);
        if animations.length() == 0 {
            return;
        }
        let settled = Function::new_with_args(
            "animations",
            "return Promise.all(animations.map((animation) => animation.finished.catch(() => {})));",
        )
        .call1(&JsValue::NULL, &animations)
        .expect("await animations")
        .unchecked_into::<Promise>();
        JsFuture::from(settled).await.expect("animations settle");
    }
    panic!("the FAB kept starting new animations");
}
