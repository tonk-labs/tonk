// Observing what the bar claims on the profile.
//
// The bar claims by `POST`ing the request as JSON to the profile branch's
// `/transact`, through `window.fetch`. This stands in for `fetch` while a
// test runs: it records each of those requests and answers it the way the
// test says the worker would. Every other request goes to the real `fetch`.
//
// Only wasm test suites include this, and the bar's own unit tests do so
// with `include!`, which is why nothing here is an inner attribute or an
// inner doc comment.

use js_sys::{Function, Promise, Reflect};
use serde_json::Value;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::window;

/// The stand-in `fetch`. Dropping it puts the original back.
pub struct ProfileFetch {
    original: JsValue,
    stub: Function,
}

/// Replace `window.fetch` until the returned value is dropped.
///
/// `answer` is a JavaScript expression evaluated for each claim, giving
/// what `fetch` answers it with: a `Response`, or a promise of one (a
/// rejected promise for a request that fails, one that never settles for a
/// request that hangs).
pub fn install(answer: &str) -> ProfileFetch {
    let win = window().expect("window");
    let original = Reflect::get(&win, &"fetch".into()).expect("original fetch");
    let answer = Function::new_no_args(&format!("return {answer};"));
    let stub: Function = Function::new_with_args(
        "original, answer",
        "const requests = [];
         const stub = function (url, init) {
             const claim = typeof url === 'string'
                 && url.endsWith('/transact')
                 && init?.method === 'POST';
             if (!claim) {
                 return original.call(this, url, init);
             }
             requests.push([url, JSON.parse(init.body)]);
             return Promise.resolve(answer());
         };
         stub.requests = requests;
         return stub;",
    )
    .call2(&JsValue::NULL, &original, &answer)
    .expect("build the fetch stub")
    .unchecked_into();
    Reflect::set(&win, &"fetch".into(), &stub).expect("stub fetch");
    ProfileFetch { original, stub }
}

impl ProfileFetch {
    /// The claims sent so far, oldest first: the URL each was posted to and
    /// its parsed JSON body.
    ///
    /// A claim leaves from a spawned task, so this first gives the event
    /// loop a turn: everything a click queued has reached `fetch`, and an
    /// answer `fetch` already gave has been read, by the time it returns.
    pub async fn requests(&self) -> Vec<(String, Value)> {
        let turn = Promise::new(&mut |resolve, _| {
            window()
                .expect("window")
                .set_timeout_with_callback(&resolve)
                .expect("timeout");
        });
        JsFuture::from(turn).await.expect("timeout resolves");
        let requests = Reflect::get(&self.stub, &"requests".into()).expect("requests");
        let json: String = js_sys::JSON::stringify(&requests)
            .expect("requests serialize")
            .into();
        serde_json::from_str(&json).expect("requests parse")
    }
}

impl Drop for ProfileFetch {
    fn drop(&mut self) {
        let win = window().expect("window");
        Reflect::set(&win, &"fetch".into(), &self.original).expect("restore fetch");
    }
}
