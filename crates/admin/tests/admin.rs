use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode, header};
use strelitzia_admin::{AdminState, router};
use strelitzia_store::Db;
use tempfile::TempDir;
use tower::ServiceExt;

const ADMIN_TOKEN: &str = "secret-admin-token";

struct TestContext {
    _dir: TempDir,
    router: Router,
}

impl TestContext {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::connect(&dir.path().join("admin.db")).await.unwrap();
        let router = router(AdminState::new(db, ADMIN_TOKEN));
        Self { _dir: dir, router }
    }

    async fn send(&self, request: Request<Body>) -> (StatusCode, HeaderMap, Vec<u8>) {
        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, headers, body.to_vec())
    }
}

fn get(uri: &str, cookie: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().method("GET").uri(uri);
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    builder.body(Body::empty()).unwrap()
}

fn post_form(uri: &str, cookie: Option<&str>, fields: &[(&str, &str)]) -> Request<Body> {
    let body = fields
        .iter()
        .map(|(name, value)| format!("{name}={}", encode(value)))
        .collect::<Vec<_>>()
        .join("&");
    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    builder.body(Body::from(body)).unwrap()
}

fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                char::from(byte).to_string()
            }
            b' ' => "+".to_owned(),
            other => format!("%{other:02X}"),
        })
        .collect()
}

fn set_cookie(headers: &HeaderMap) -> String {
    headers
        .get(header::SET_COOKIE)
        .expect("response should set a session cookie")
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned()
}

fn csrf_token(body: &[u8]) -> String {
    let body = std::str::from_utf8(body).unwrap();
    let marker = "name=\"csrf_token\" value=\"";
    let start = body.find(marker).expect("form should contain a CSRF token") + marker.len();
    let rest = &body[start..];
    let end = rest.find('"').unwrap();
    rest[..end].to_owned()
}

#[tokio::test]
async fn login_requires_token_csrf_and_session() {
    let context = TestContext::new().await;

    let (status, headers, _) = context.send(get("/", None)).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(headers.get(header::LOCATION).unwrap(), "/admin/login");

    let (status, headers, body) = context.send(get("/login", None)).await;
    assert_eq!(status, StatusCode::OK);
    let cookie = set_cookie(&headers);
    let csrf = csrf_token(&body);

    let (status, _, _) = context
        .send(post_form(
            "/login",
            Some(&cookie),
            &[("token", ADMIN_TOKEN)],
        ))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _, _) = context
        .send(post_form(
            "/login",
            Some(&cookie),
            &[("csrf_token", &csrf), ("token", "wrong-token")],
        ))
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, headers, _) = context
        .send(post_form(
            "/login",
            Some(&cookie),
            &[("csrf_token", &csrf), ("token", ADMIN_TOKEN)],
        ))
        .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(headers.get(header::LOCATION).unwrap(), "/admin");
    let cookie = set_cookie(&headers);

    let (status, _, body) = context.send(get("/", Some(&cookie))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(String::from_utf8_lossy(&body).contains("Dashboard"));
}
