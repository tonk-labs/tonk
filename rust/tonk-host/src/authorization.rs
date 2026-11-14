//! Answering an app's authorization request from the top page.
//!
//! An app signing in over OAuth waits on the authorization server, which
//! binds its request to the browser that began it with a cookie on the
//! app's origin, so only a page on that origin can answer it. When the
//! worker has the answer, an agent's session token or a refusal, it sends
//! the page to the server's own address for the request with the answer in
//! the address's fragment, which never reaches a server. The top page posts
//! the answer there, with the cookie, and goes where the server says.

use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{Headers, RequestInit, Response, Url, window};

/// What a worker's answer to the request at `href` carries, when `href` is
/// one: the request's address on this page's origin, and the answer.
fn answer_of(href: &str, origin: &str) -> Option<(String, String)> {
    let address = Url::new_with_base(href, origin).ok()?;
    if address.origin() != origin || !address.pathname().starts_with("/authorize/") {
        return None;
    }
    let answer = address.hash().strip_prefix("#answer=")?.to_owned();
    let answer = js_sys::decode_uri_component(&answer).ok()?.as_string()?;
    Some((format!("{origin}{}", address.pathname()), answer))
}

/// Answer the authorization request `href` names, when it names one, and go
/// where the server says. Answers whether it did.
pub(crate) fn answer_authorization(href: &str) -> bool {
    let Some(origin) = window().and_then(|window| window.location().origin().ok()) else {
        return false;
    };
    let Some((endpoint, answer)) = answer_of(href, &origin) else {
        return false;
    };
    tonk_common::log!("answering the authorization request at {endpoint}");
    wasm_bindgen_futures::spawn_local(async move {
        match post(&endpoint, &answer).await {
            Ok(redirect) => {
                if let Some(window) = window() {
                    let _ = window.location().replace(&redirect);
                }
            }
            Err(error) => {
                tonk_common::log!("the authorization was not answered: {error:?}");
            }
        }
    });
    true
}

/// Post `answer` to `endpoint` with this origin's cookies, and read where
/// the server sends the browser next.
async fn post(endpoint: &str, answer: &str) -> Result<String, JsValue> {
    let headers = Headers::new()?;
    headers.append("content-type", "application/json")?;
    let init = RequestInit::new();
    init.set_method("POST");
    init.set_headers(&headers);
    init.set_body(&JsValue::from_str(answer));
    let window = window().ok_or("no window")?;
    let response: Response = JsFuture::from(window.fetch_with_str_and_init(endpoint, &init))
        .await?
        .dyn_into()?;
    if !response.ok() {
        let said = JsFuture::from(response.text()?).await?;
        return Err(said);
    }
    let body = JsFuture::from(response.json()?).await?;
    js_sys::Reflect::get(&body, &"redirect".into())?
        .as_string()
        .ok_or_else(|| "the server said nowhere to go".into())
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::answer_of;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    fn it_reads_an_answer_to_a_request_on_this_origin() {
        let origin = "https://tonk.test";
        assert_eq!(
            answer_of(
                "https://tonk.test/authorize/abc_1#answer=%7B%22deny%22:true%7D",
                origin
            ),
            Some((
                "https://tonk.test/authorize/abc_1".to_owned(),
                r#"{"deny":true}"#.to_owned()
            ))
        );
        assert_eq!(
            answer_of("https://elsewhere.test/authorize/abc#answer=1", origin),
            None,
            "another origin's request is not answered from here"
        );
        assert_eq!(
            answer_of("https://tonk.test/authorize/abc", origin),
            None,
            "an address with no answer is a navigation"
        );
        assert_eq!(answer_of("https://tonk.test/settings", origin), None);
    }
}
