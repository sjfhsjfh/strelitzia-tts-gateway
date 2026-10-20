use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode, header};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};
use strelitzia_core::keys;
use strelitzia_core::{
    AudioResponse, Capabilities, Channel, ChannelKind, GatewayError, SpeechRequest, TtsAdapter,
};
use strelitzia_providers::{AdapterRegistry, MockAdapter};
use strelitzia_routing::RetryPolicy;
use strelitzia_server::router;
use strelitzia_server::state::AppState;
use strelitzia_store::{Db, NewApiKey, NewChannel};
use tempfile::TempDir;
use tower::ServiceExt;

#[derive(Clone, Default)]
struct TestAdapter {
    inner: MockAdapter,
    failures: Arc<Mutex<HashMap<i64, GatewayError>>>,
    calls: Arc<Mutex<Vec<(i64, SpeechRequest)>>>,
}

impl TestAdapter {
    fn new() -> Self {
        Self::default()
    }

    fn fail(&self, channel_id: i64, error: GatewayError) {
        self.failures.lock().unwrap().insert(channel_id, error);
    }

    fn calls(&self) -> Vec<(i64, SpeechRequest)> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl TtsAdapter for TestAdapter {
    fn kind(&self) -> ChannelKind {
        ChannelKind::OpenAiCompatible
    }

    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }

    async fn synthesize(
        &self,
        channel: &Channel,
        req: &SpeechRequest,
    ) -> Result<AudioResponse, GatewayError> {
        self.calls.lock().unwrap().push((channel.id, req.clone()));

        if let Some(error) = self.failures.lock().unwrap().get(&channel.id).cloned() {
            return Err(error);
        }

        self.inner.synthesize(channel, req).await
    }
}

struct TestContext {
    _dir: TempDir,
    db: Db,
    adapter: TestAdapter,
    router: Router,
    key: String,
}

impl TestContext {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::connect(&dir.path().join("strelitzia.db"))
            .await
            .unwrap();

        let key = keys::generate_key();
        db.api_keys()
            .create(NewApiKey {
                name: "test".to_owned(),
                key_hash: key.hash,
                key_prefix: key.prefix,
            })
            .await
            .unwrap();

        let adapter = TestAdapter::new();
        let mut registry = AdapterRegistry::new();
        let _ = registry.register(Arc::new(adapter.clone()));

        let state = AppState::new(
            db.clone(),
            registry,
            reqwest::Client::new(),
            RetryPolicy { max_attempts: 3 },
            Duration::from_secs(5),
        );

        Self {
            _dir: dir,
            db,
            adapter,
            router: router(state),
            key: key.plaintext,
        }
    }

    async fn add_channel(&self, name: &str, models: &[&str], priority: i64) -> Channel {
        self.db
            .channels()
            .create(NewChannel {
                name: name.to_owned(),
                kind: ChannelKind::OpenAiCompatible,
                base_url: Some("http://127.0.0.1:9".to_owned()),
                api_key: None,
                models: models.iter().map(|model| (*model).to_owned()).collect(),
                voices: Vec::new(),
                voice_namespace: None,
                priority,
                weight: 1,
                enabled: true,
            })
            .await
            .unwrap()
    }

    async fn send(&self, request: Request<Body>) -> (StatusCode, HeaderMap, Vec<u8>) {
        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, headers, body.to_vec())
    }

    async fn speech(&self, body: Value) -> (StatusCode, HeaderMap, Vec<u8>) {
        self.send(post_json("/v1/audio/speech", Some(&self.key), body))
            .await
    }

    async fn legado(&self, body: Value) -> (StatusCode, HeaderMap, Vec<u8>) {
        self.send(post_json("/speech/stream", Some(&self.key), body))
            .await
    }
}

fn post_json(uri: &str, key: Option<&str>, body: Value) -> Request<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(key) = key {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {key}"));
    }
    builder.body(Body::from(body.to_string())).unwrap()
}

fn speech_body(model: &str, input: &str, speed: f32) -> Value {
    json!({
        "model": model,
        "input": input,
        "voice": "alloy",
        "speed": speed
    })
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> &'a str {
    headers.get(name).unwrap().to_str().unwrap()
}

fn envelope(body: &[u8]) -> Value {
    serde_json::from_slice(body).unwrap()
}

#[tokio::test]
async fn openai_speech_contract() {
    let context = TestContext::new().await;
    context.add_channel("primary", &["tts-1"], 0).await;
    let body = speech_body("tts-1", "hello", 1.0);

    let (status, _, _) = context
        .send(post_json("/v1/audio/speech", None, body.clone()))
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, headers, audio) = context.speech(body).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header(&headers, "content-type"), "audio/mpeg");
    assert_eq!(audio, b"mock:tts-1:alloy:hello");

    let mut sse = speech_body("tts-1", "hello", 1.0);
    sse["stream_format"] = json!("sse");
    let (status, headers, payload) = context.speech(sse).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header(&headers, "content-type"), "text/event-stream");
    assert_eq!(header(&headers, "cache-control"), "no-cache");
    assert_eq!(header(&headers, "x-accel-buffering"), "no");

    let expected_delta = format!(
        "data: {{\"type\":\"speech.audio.delta\",\"audio\":\"{}\"}}",
        STANDARD.encode(b"mock:tts-1:alloy:hello")
    );
    let expected_done = "data: {\"type\":\"speech.audio.done\",\"usage\":{\"input_tokens\":0,\"output_tokens\":0,\"total_tokens\":0}}";
    let text = String::from_utf8(payload).unwrap();
    let frames: Vec<&str> = text
        .split("\n\n")
        .filter(|frame| !frame.is_empty())
        .collect();
    assert_eq!(frames, vec![expected_delta.as_str(), expected_done]);
    assert!(!text.contains("[DONE]"));

    let (status, _, payload) = context
        .speech(speech_body("tts-1", &"a".repeat(4097), 1.0))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(envelope(&payload)["error"]["param"], "input");

    let (status, _, payload) = context.speech(speech_body("tts-9", "hello", 1.0)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(envelope(&payload)["error"]["code"], "not_found");
}

#[tokio::test]
async fn openai_failover_and_attempts() {
    let context = TestContext::new().await;
    let failing = context.add_channel("failing", &["tts-1"], 100).await;
    let healthy = context.add_channel("healthy", &["tts-1"], 10).await;
    context.adapter.fail(
        failing.id,
        GatewayError::RateLimit {
            message: "slow down".to_owned(),
            retry_after: Some(3),
        },
    );

    let mut request = post_json(
        "/v1/audio/speech",
        Some(&context.key),
        speech_body("tts-1", "hello", 1.0),
    );
    request
        .headers_mut()
        .insert("x-request-id", "req-failover".parse().unwrap());
    let (status, headers, body) = context.send(request).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(header(&headers, "x-provider"), "mock");
    assert_eq!(body, b"mock:tts-1:alloy:hello");

    let detail = context
        .db
        .logs()
        .get_by_request_id("req-failover")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(detail.attempts.len(), 2);
    assert_eq!(detail.attempts[0].channel_id, Some(failing.id));
    assert_eq!(detail.attempts[0].outcome, "error");
    assert_eq!(detail.attempts[1].channel_id, Some(healthy.id));
    assert_eq!(detail.attempts[1].outcome, "success");

    let context = TestContext::new().await;
    let failing = context.add_channel("failing", &["tts-1"], 100).await;
    context.add_channel("healthy", &["tts-1"], 10).await;
    context.adapter.fail(
        failing.id,
        GatewayError::invalid_request("unsupported voice", Some("voice")),
    );

    let mut request = post_json(
        "/v1/audio/speech",
        Some(&context.key),
        speech_body("tts-1", "hello", 1.0),
    );
    request
        .headers_mut()
        .insert("x-request-id", "req-no-failover".parse().unwrap());
    let (status, _, body) = context.send(request).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(envelope(&body)["error"]["message"], "unsupported voice");
    assert_eq!(context.adapter.calls().len(), 1);

    let detail = context
        .db
        .logs()
        .get_by_request_id("req-no-failover")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(detail.attempts.len(), 1);
    assert_eq!(detail.attempts[0].outcome, "error");
}

#[tokio::test]
async fn legado_rate_mapping() {
    let context = TestContext::new().await;
    context.add_channel("primary", &["tts-1"], 0).await;

    for rate in ["5+0%", "15+0%", "25+0%"] {
        let (status, headers, body) = context
            .legado(json!({
                "text": "hello",
                "voice": "alloy",
                "rate": rate
            }))
            .await;
        assert_eq!(status, StatusCode::OK, "rate {rate}");
        assert_eq!(header(&headers, "content-type"), "audio/mpeg");
        assert_eq!(body, b"mock:tts-1:alloy:hello");
    }

    let speeds: Vec<f32> = context
        .adapter
        .calls()
        .iter()
        .map(|(_, request)| request.speed)
        .collect();
    let expected = [1.0 - 10.0 / 15.0, 1.0, 1.0 + 10.0 / 15.0];
    for (actual, expected) in speeds.iter().zip(expected) {
        assert!(
            (actual - expected).abs() < 1e-5,
            "expected {expected}, got {actual}"
        );
    }

    let (status, _, body) = context
        .legado(json!({"text": "hello", "voice": "alloy", "rate": "fast"}))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(envelope(&body)["error"]["param"], "rate");
}
