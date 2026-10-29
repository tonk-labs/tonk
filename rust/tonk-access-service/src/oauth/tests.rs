use super::*;
use crate::store::sqlite::SqliteStore;
use std::collections::HashMap;

const ORIGIN: &str = "https://tonk.example";
const CALLBACK: &str = "http://localhost:6274/oauth/callback";
/// RFC 7636 appendix B.
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
const TOKEN: &str = "c2Vzc2lvbi10b2tlbg";

/// Metadata documents answered from memory.
#[derive(Default)]
struct Served(HashMap<String, String>);

#[async_trait::async_trait]
impl Documents for Served {
    async fn fetch(&self, url: &Url) -> Result<Vec<u8>, String> {
        self.0
            .get(url.as_str())
            .map(|body| body.clone().into_bytes())
            .ok_or_else(|| "404".to_owned())
    }
}

struct Fixture {
    store: SqliteStore,
    documents: Served,
    key: ClientKey,
    now: u64,
}

/// A browser that went through `/authorize`: the handle the page got,
/// and the cookie it holds.
struct Started {
    handle: String,
    cookie: String,
}

impl Fixture {
    fn new() -> Self {
        Self {
            store: SqliteStore::in_memory().unwrap(),
            documents: Served::default(),
            key: ClientKey::derive(&"11".repeat(32)).unwrap(),
            now: 1_800_000_000,
        }
    }

    fn at(&self, now: u64) -> AuthorizationServer<'_, SqliteStore, Served> {
        AuthorizationServer {
            store: &self.store,
            documents: &self.documents,
            key: &self.key,
            origin: ORIGIN,
            now,
        }
    }

    async fn call(&self, now: u64, endpoint: Endpoint, request: Request<'_>) -> Answer {
        self.at(now).handle(&endpoint, &request).await
    }

    async fn register(&self) -> String {
        let body = serde_json::to_vec(&json!({
            "client_name": "Inspector",
            "redirect_uris": ["http://localhost/oauth/callback"],
            "grant_types": ["authorization_code", "refresh_token"],
            "token_endpoint_auth_method": "none",
        }))
        .unwrap();
        let answer = self
            .call(
                self.now,
                Endpoint::Register,
                Request {
                    method: "POST",
                    content_type: Some("application/json"),
                    body: &body,
                    ..Default::default()
                },
            )
            .await;
        assert_eq!(answer.status, 201, "{}", text(&answer));
        let registered: serde_json::Value = serde_json::from_slice(&answer.body).unwrap();
        assert_eq!(registered["grant_types"], json!(["authorization_code"]));
        registered["client_id"].as_str().unwrap().to_owned()
    }

    async fn authorize(&self, query: &[(&str, &str)]) -> Answer {
        let query = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(query)
            .finish();
        self.call(
            self.now,
            Endpoint::Authorize,
            Request {
                method: "GET",
                query: Some(&query),
                ..Default::default()
            },
        )
        .await
    }

    async fn start(&self, client_id: &str) -> Started {
        let answer = self.authorize(&request(client_id)).await;
        assert_eq!(answer.status, 303, "{}", text(&answer));
        let location = header(&answer, "Location").unwrap();
        let handle = Url::parse(location)
            .unwrap()
            .query_pairs()
            .find(|(key, _)| key == "request")
            .unwrap()
            .1
            .into_owned();
        assert!(location.starts_with(&format!("{ORIGIN}{APPROVAL_PAGE}?")));
        let cookie = header(&answer, "Set-Cookie")
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned();
        assert!(cookie.starts_with("__Host-tonk-authorize-"));
        Started { handle, cookie }
    }

    async fn answer_with(&self, now: u64, started: &Started, body: serde_json::Value) -> Answer {
        let body = serde_json::to_vec(&body).unwrap();
        self.call(
            now,
            Endpoint::Request(started.handle.clone()),
            Request {
                method: "POST",
                content_type: Some("application/json"),
                origin: Some(ORIGIN),
                cookie: Some(&started.cookie),
                body: &body,
                ..Default::default()
            },
        )
        .await
    }

    /// Approve with [`TOKEN`] and answer the code the callback got.
    async fn approve(&self, started: &Started) -> String {
        let answer = self
            .answer_with(self.now, started, json!({ "token": TOKEN }))
            .await;
        assert_eq!(answer.status, 200, "{}", text(&answer));
        let redirect: serde_json::Value = serde_json::from_slice(&answer.body).unwrap();
        let redirect = Url::parse(redirect["redirect"].as_str().unwrap()).unwrap();
        assert_eq!(redirect.path(), "/oauth/callback");
        assert_eq!(redirect.port(), Some(6274));
        let param = |name: &str| {
            redirect
                .query_pairs()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.into_owned())
        };
        assert_eq!(param("state").as_deref(), Some("xyz"));
        assert_eq!(param("iss").as_deref(), Some(ORIGIN));
        param("code").unwrap()
    }

    async fn exchange(&self, now: u64, form: &[(&str, &str)]) -> Answer {
        let body = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(form)
            .finish();
        self.call(
            now,
            Endpoint::Token,
            Request {
                method: "POST",
                content_type: Some("application/x-www-form-urlencoded"),
                body: body.as_bytes(),
                ..Default::default()
            },
        )
        .await
    }
}

/// A valid authorization request for `client_id`.
fn request(client_id: &str) -> Vec<(&str, &str)> {
    vec![
        ("response_type", "code"),
        ("client_id", client_id),
        ("redirect_uri", CALLBACK),
        ("code_challenge", CHALLENGE),
        ("code_challenge_method", "S256"),
        ("resource", "https://tonk.example/mcp"),
        ("state", "xyz"),
    ]
}

/// A valid token request for `code`.
fn redemption<'a>(client_id: &'a str, code: &'a str) -> Vec<(&'a str, &'a str)> {
    vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("client_id", client_id),
        ("redirect_uri", CALLBACK),
        ("code_verifier", VERIFIER),
        ("resource", "https://tonk.example/mcp"),
    ]
}

/// `form` with `name` set to `value`.
fn with<'a>(
    mut form: Vec<(&'a str, &'a str)>,
    name: &str,
    value: &'a str,
) -> Vec<(&'a str, &'a str)> {
    for pair in form.iter_mut() {
        if pair.0 == name {
            pair.1 = value;
        }
    }
    form
}

fn header<'a>(answer: &'a Answer, name: &str) -> Option<&'a str> {
    answer
        .headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn text(answer: &Answer) -> String {
    String::from_utf8_lossy(&answer.body).into_owned()
}

fn error(answer: &Answer) -> String {
    serde_json::from_slice::<serde_json::Value>(&answer.body).unwrap()["error"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// The `error` a redirect to the callback carries.
fn redirected_error(answer: &Answer) -> String {
    assert_eq!(answer.status, 303, "{}", text(answer));
    let location = Url::parse(header(answer, "Location").unwrap()).unwrap();
    assert!(location.as_str().starts_with(CALLBACK));
    location
        .query_pairs()
        .find(|(key, _)| key == "error")
        .unwrap()
        .1
        .into_owned()
}

#[dialog_common::test]
async fn it_describes_the_resource_and_the_server() {
    let fixture = Fixture::new();
    let get = Request {
        method: "GET",
        ..Default::default()
    };
    let resource = fixture
        .call(fixture.now, Endpoint::ProtectedResource, get)
        .await;
    let resource: serde_json::Value = serde_json::from_slice(&resource.body).unwrap();
    assert_eq!(resource["resource"], "https://tonk.example/mcp");
    assert_eq!(resource["authorization_servers"], json!([ORIGIN]));

    let server = fixture
        .call(fixture.now, Endpoint::AuthorizationServer, get)
        .await;
    assert_eq!(header(&server, "Access-Control-Allow-Origin"), Some("*"));
    let server: serde_json::Value = serde_json::from_slice(&server.body).unwrap();
    assert_eq!(server["issuer"], ORIGIN);
    assert_eq!(server["code_challenge_methods_supported"], json!(["S256"]));
    assert_eq!(server["client_id_metadata_document_supported"], true);
    assert_eq!(
        server["registration_endpoint"],
        "https://tonk.example/register"
    );
    assert_eq!(
        server["authorization_endpoint"],
        "https://tonk.example/authorize"
    );
    assert_eq!(server["token_endpoint"], "https://tonk.example/token");
    assert_eq!(
        server["grant_types_supported"],
        json!(["authorization_code"])
    );
}

#[dialog_common::test]
fn it_routes_by_path() {
    assert_eq!(
        Endpoint::of("/.well-known/oauth-protected-resource/mcp"),
        Some(Endpoint::ProtectedResource)
    );
    assert_eq!(Endpoint::of("/authorize"), Some(Endpoint::Authorize));
    assert_eq!(
        Endpoint::of("/authorize/abc"),
        Some(Endpoint::Request("abc".into()))
    );
    assert_eq!(Endpoint::of("/authorize/"), None);
    assert_eq!(Endpoint::of("/authorize/a/b"), None);
    assert_eq!(Endpoint::of("/mcp"), None);
}

#[dialog_common::test]
async fn it_hands_the_finished_token_to_the_client_once() {
    let fixture = Fixture::new();
    let client_id = fixture.register().await;
    let started = fixture.start(&client_id).await;

    let described = fixture
        .call(
            fixture.now,
            Endpoint::Request(started.handle.clone()),
            Request {
                method: "GET",
                ..Default::default()
            },
        )
        .await;
    assert_eq!(described.status, 200, "{}", text(&described));
    let described: serde_json::Value = serde_json::from_slice(&described.body).unwrap();
    assert_eq!(described["client_name"], "Inspector");
    assert_eq!(described["registration"], "dynamic");
    assert_eq!(described["redirect_uri"], CALLBACK);
    assert_eq!(described["redirect_host"], "localhost");

    let code = fixture.approve(&started).await;
    let answer = fixture
        .exchange(fixture.now, &redemption(&client_id, &code))
        .await;
    assert_eq!(answer.status, 200, "{}", text(&answer));
    assert_eq!(header(&answer, "Cache-Control"), Some("no-store"));
    let token: serde_json::Value = serde_json::from_slice(&answer.body).unwrap();
    assert_eq!(
        token,
        json!({ "access_token": TOKEN, "token_type": "Bearer" }),
        "the token, with no expiry and no refresh token"
    );
}

#[dialog_common::test]
async fn it_refuses_a_replayed_code() {
    let fixture = Fixture::new();
    let client_id = fixture.register().await;
    let code = fixture.approve(&fixture.start(&client_id).await).await;
    let form = redemption(&client_id, &code);
    assert_eq!(fixture.exchange(fixture.now, &form).await.status, 200);

    let replayed = fixture.exchange(fixture.now, &form).await;
    assert_eq!(replayed.status, 400);
    assert_eq!(error(&replayed), "invalid_grant");
}

#[dialog_common::test]
async fn it_refuses_the_wrong_verifier() {
    let fixture = Fixture::new();
    let client_id = fixture.register().await;
    let code = fixture.approve(&fixture.start(&client_id).await).await;

    let other = "x".repeat(43);
    let wrong = fixture
        .exchange(
            fixture.now,
            &with(redemption(&client_id, &code), "code_verifier", &other),
        )
        .await;
    assert_eq!(error(&wrong), "invalid_grant");
    let malformed = fixture
        .exchange(
            fixture.now,
            &with(redemption(&client_id, &code), "code_verifier", "short"),
        )
        .await;
    assert_eq!(error(&malformed), "invalid_grant");

    // A guess spends nothing: the client holding the verifier still
    // redeems.
    let right = fixture
        .exchange(fixture.now, &redemption(&client_id, &code))
        .await;
    assert_eq!(right.status, 200, "{}", text(&right));
}

#[dialog_common::test]
async fn it_refuses_the_wrong_resource() {
    let fixture = Fixture::new();
    let client_id = fixture.register().await;

    let elsewhere = fixture
        .authorize(&with(
            request(&client_id),
            "resource",
            "https://other.example/mcp",
        ))
        .await;
    assert_eq!(redirected_error(&elsewhere), "invalid_target");

    let code = fixture.approve(&fixture.start(&client_id).await).await;
    let wrong = fixture
        .exchange(
            fixture.now,
            &with(
                redemption(&client_id, &code),
                "resource",
                "https://tonk.example/",
            ),
        )
        .await;
    assert_eq!(error(&wrong), "invalid_target");
    let missing: Vec<_> = redemption(&client_id, &code)
        .into_iter()
        .filter(|(name, _)| *name != "resource")
        .collect();
    assert_eq!(
        error(&fixture.exchange(fixture.now, &missing).await),
        "invalid_request"
    );
}

#[dialog_common::test]
async fn it_refuses_the_wrong_redirect() {
    let fixture = Fixture::new();
    let client_id = fixture.register().await;

    // An unregistered callback is never redirected to.
    let unregistered = fixture
        .authorize(&with(
            request(&client_id),
            "redirect_uri",
            "https://evil.example/callback",
        ))
        .await;
    assert_eq!(unregistered.status, 400);
    assert_eq!(header(&unregistered, "Location"), None);

    // A code redeems only with the callback it went to: another port of
    // the same loopback callback is a different callback here.
    let code = fixture.approve(&fixture.start(&client_id).await).await;
    let wrong = fixture
        .exchange(
            fixture.now,
            &with(
                redemption(&client_id, &code),
                "redirect_uri",
                "http://localhost:9999/oauth/callback",
            ),
        )
        .await;
    assert_eq!(error(&wrong), "invalid_grant");
}

#[dialog_common::test]
async fn it_refuses_an_expired_code() {
    let fixture = Fixture::new();
    let client_id = fixture.register().await;
    let code = fixture.approve(&fixture.start(&client_id).await).await;

    let late = fixture
        .exchange(fixture.now + CODE_TTL, &redemption(&client_id, &code))
        .await;
    assert_eq!(error(&late), "invalid_grant");
}

#[dialog_common::test]
async fn it_refuses_another_clients_code() {
    let fixture = Fixture::new();
    let client_id = fixture.register().await;
    let code = fixture.approve(&fixture.start(&client_id).await).await;
    let other = fixture
        .key
        .register(br#"{"redirect_uris":["http://localhost/oauth/callback"],"client_name":"Other"}"#)
        .unwrap();

    let wrong = fixture
        .exchange(fixture.now, &redemption(&other.id, &code))
        .await;
    assert_eq!(error(&wrong), "invalid_grant");
}

#[dialog_common::test]
async fn it_refuses_an_authorization_without_pkce() {
    let fixture = Fixture::new();
    let client_id = fixture.register().await;

    let plain = fixture
        .authorize(&with(request(&client_id), "code_challenge_method", "plain"))
        .await;
    assert_eq!(redirected_error(&plain), "invalid_request");
    let bogus = fixture
        .authorize(&with(request(&client_id), "code_challenge", "short"))
        .await;
    assert_eq!(redirected_error(&bogus), "invalid_request");
    let token = fixture
        .authorize(&with(request(&client_id), "response_type", "token"))
        .await;
    assert_eq!(redirected_error(&token), "unsupported_response_type");
}

#[dialog_common::test]
async fn it_refuses_an_unknown_client_without_redirecting() {
    let fixture = Fixture::new();
    let forged = fixture
        .authorize(&request("eyJuYW1lIjoiRXZpbCJ9.AAAA"))
        .await;
    assert_eq!(forged.status, 400);
    assert_eq!(header(&forged, "Location"), None);

    let repeated = fixture
        .call(
            fixture.now,
            Endpoint::Authorize,
            Request {
                method: "GET",
                query: Some("client_id=a&client_id=b"),
                ..Default::default()
            },
        )
        .await;
    assert_eq!(repeated.status, 400);
}

#[dialog_common::test]
async fn it_takes_an_answer_only_from_the_browser_that_started() {
    let fixture = Fixture::new();
    let client_id = fixture.register().await;
    let started = fixture.start(&client_id).await;
    let body = serde_json::to_vec(&json!({ "token": TOKEN })).unwrap();
    let answer = |origin, cookie| {
        fixture.call(
            fixture.now,
            Endpoint::Request(started.handle.clone()),
            Request {
                method: "POST",
                content_type: Some("application/json"),
                origin,
                cookie,
                body: &body,
                ..Default::default()
            },
        )
    };

    let elsewhere = answer(Some("https://evil.example"), Some(started.cookie.as_str())).await;
    assert_eq!(elsewhere.status, 403);
    let without = answer(Some(ORIGIN), None).await;
    assert_eq!(without.status, 403);
    let forged = format!("{}=forged", started.cookie.split('=').next().unwrap());
    let other = answer(Some(ORIGIN), Some(forged.as_str())).await;
    assert_eq!(other.status, 403);

    // Still pending: the right browser can finish.
    fixture.approve(&started).await;
    let again = fixture
        .answer_with(fixture.now, &started, json!({ "token": TOKEN }))
        .await;
    assert_eq!(again.status, 404, "an answered request answers once");
}

#[dialog_common::test]
async fn it_returns_a_refusal_to_the_client() {
    let fixture = Fixture::new();
    let client_id = fixture.register().await;
    let started = fixture.start(&client_id).await;

    let denied = fixture
        .answer_with(fixture.now, &started, json!({ "deny": true }))
        .await;
    assert_eq!(denied.status, 200, "{}", text(&denied));
    let redirect: serde_json::Value = serde_json::from_slice(&denied.body).unwrap();
    let redirect = Url::parse(redirect["redirect"].as_str().unwrap()).unwrap();
    assert!(
        redirect
            .query_pairs()
            .any(|(key, value)| key == "error" && value == "access_denied")
    );
    assert!(header(&denied, "Set-Cookie").unwrap().contains("Max-Age=0"));

    let after = fixture
        .answer_with(fixture.now, &started, json!({ "token": TOKEN }))
        .await;
    assert_eq!(after.status, 404);
}

#[dialog_common::test]
async fn it_lets_a_request_lapse() {
    let fixture = Fixture::new();
    let client_id = fixture.register().await;
    let started = fixture.start(&client_id).await;

    let late = fixture
        .answer_with(
            fixture.now + REQUEST_TTL,
            &started,
            json!({ "token": TOKEN }),
        )
        .await;
    assert_eq!(late.status, 404);
}

#[dialog_common::test]
async fn it_refuses_a_token_no_bearer_header_can_carry() {
    let fixture = Fixture::new();
    let client_id = fixture.register().await;
    let started = fixture.start(&client_id).await;

    for token in ["", "has space", "line\nbreak"] {
        let answer = fixture
            .answer_with(fixture.now, &started, json!({ "token": token }))
            .await;
        assert_eq!(answer.status, 400, "{token:?} is accepted");
    }
}

#[dialog_common::test]
async fn it_identifies_a_client_by_its_metadata_document() {
    let mut fixture = Fixture::new();
    let id = "https://client.example/oauth/metadata.json";
    fixture.documents.0.insert(
        id.into(),
        json!({
            "client_id": id,
            "client_name": "Example Agent",
            "redirect_uris": ["http://localhost/oauth/callback"],
        })
        .to_string(),
    );
    let started = fixture.start(id).await;
    let code = fixture.approve(&started).await;
    let answer = fixture.exchange(fixture.now, &redemption(id, &code)).await;
    assert_eq!(answer.status, 200, "{}", text(&answer));

    let unpublished = fixture
        .authorize(&request("https://client.example/oauth/missing.json"))
        .await;
    assert_eq!(unpublished.status, 400);
    assert_eq!(header(&unpublished, "Location"), None);
}

#[dialog_common::test]
fn it_seals_a_token_under_its_code() {
    let code = random_bytes();
    let sealed = seal(&code, TOKEN.as_bytes());
    assert!(!sealed.contains(TOKEN));
    assert_eq!(open(&code, &sealed).as_deref(), Some(TOKEN));
    assert_eq!(open(&random_bytes(), &sealed), None);
}

#[dialog_common::test]
async fn it_names_the_cookie_for_a_plain_http_origin() {
    let fixture = Fixture::new();
    let client_id = fixture.register().await;
    let server = AuthorizationServer {
        origin: "http://localhost:8080",
        ..fixture.at(fixture.now)
    };
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(with(
            request(&client_id),
            "resource",
            "http://localhost:8080/mcp",
        ))
        .finish();
    let answer = server
        .handle(
            &Endpoint::Authorize,
            &Request {
                method: "GET",
                query: Some(&query),
                ..Default::default()
            },
        )
        .await;
    let cookie = header(&answer, "Set-Cookie").unwrap();
    assert!(cookie.starts_with("tonk-authorize-"), "{cookie}");
    assert!(!cookie.contains("Secure"));
}

#[dialog_common::test]
async fn it_registers_a_client_it_can_identify_again() {
    let fixture = Fixture::new();
    let client_id = fixture.register().await;
    let client = fixture.key.registered(&client_id).unwrap();
    assert_eq!(client.name, "Inspector");
    assert_eq!(client.redirect_uris, ["http://localhost/oauth/callback"]);
}

#[dialog_common::test]
async fn it_refuses_a_registration_it_cannot_read() {
    let fixture = Fixture::new();
    let form = fixture
        .call(
            fixture.now,
            Endpoint::Register,
            Request {
                method: "POST",
                content_type: Some("application/x-www-form-urlencoded"),
                body: b"redirect_uris=https://example.com/cb",
                ..Default::default()
            },
        )
        .await;
    assert_eq!(form.status, 400);
    assert_eq!(error(&form), "invalid_client_metadata");

    let secret = fixture
        .call(
            fixture.now,
            Endpoint::Register,
            Request {
                method: "POST",
                content_type: Some("application/json; charset=utf-8"),
                body: br#"{"redirect_uris":["https://example.com/cb"],"token_endpoint_auth_method":"client_secret_post"}"#,
                ..Default::default()
            },
        )
        .await;
    assert_eq!(secret.status, 400);
    assert_eq!(error(&secret), "invalid_client_metadata");
    assert_eq!(header(&secret, "Access-Control-Allow-Origin"), Some("*"));
}

#[dialog_common::test]
async fn it_answers_preflights_and_refuses_other_methods() {
    let fixture = Fixture::new();
    let preflight = fixture
        .call(
            fixture.now,
            Endpoint::Register,
            Request {
                method: "OPTIONS",
                ..Default::default()
            },
        )
        .await;
    assert_eq!(preflight.status, 204);
    assert_eq!(header(&preflight, "Access-Control-Allow-Origin"), Some("*"));

    let get = fixture
        .call(
            fixture.now,
            Endpoint::Register,
            Request {
                method: "GET",
                ..Default::default()
            },
        )
        .await;
    assert_eq!(get.status, 405);
}
