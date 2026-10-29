use super::*;

const ORIGIN: &str = "https://tonk.example";

struct Fixture {
    key: ClientKey,
    now: u64,
}

impl Fixture {
    fn new() -> Self {
        Self {
            key: ClientKey::derive(&"11".repeat(32)).unwrap(),
            now: 1_800_000_000,
        }
    }

    fn at(&self, now: u64) -> AuthorizationServer<'_> {
        AuthorizationServer {
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
    assert_eq!(
        Endpoint::of("/.well-known/oauth-authorization-server"),
        Some(Endpoint::AuthorizationServer)
    );
    assert_eq!(Endpoint::of("/register"), Some(Endpoint::Register));
    assert_eq!(Endpoint::of("/mcp"), None);
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
