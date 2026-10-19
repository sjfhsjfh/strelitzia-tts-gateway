use axum::Form;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use strelitzia_core::{ChannelKind, keys, split_voice_reference};
use strelitzia_store::{ChannelUpdate, NewApiKey, NewChannel};
use tower_sessions::Session;

use crate::pages::{
    ChannelFormPage, ChannelFormValues, ChannelRow, ChannelsPage, DashboardPage, KeysPage,
    LogDetailPage, LoginPage, LogsPage,
};
use crate::{
    AUTH_KEY, AdminError, AdminState, csrf_token, render_with_status, see_other, token_matches,
    verify_csrf,
};

pub(crate) async fn login_form(session: Session) -> Result<Response, AdminError> {
    if session
        .get::<bool>(AUTH_KEY)
        .await
        .map_err(AdminError::session)?
        == Some(true)
    {
        return Ok(see_other("/admin"));
    }

    let csrf_token = csrf_token(&session).await?;
    Ok(LoginPage {
        logged_in: false,
        csrf_token,
        error: None,
    }
    .into_response())
}

#[derive(Debug, Deserialize)]
pub(crate) struct LoginForm {
    #[serde(default)]
    csrf_token: String,
    #[serde(default)]
    token: String,
}

pub(crate) async fn login(
    State(state): State<AdminState>,
    session: Session,
    Form(form): Form<LoginForm>,
) -> Result<Response, AdminError> {
    verify_csrf(&session, &form.csrf_token).await?;
    let csrf_token = csrf_token(&session).await?;

    if !token_matches(&state, &form.token) {
        return Ok(render_with_status(
            StatusCode::UNAUTHORIZED,
            LoginPage {
                logged_in: false,
                csrf_token,
                error: Some("Invalid admin token.".to_owned()),
            },
        ));
    }

    session
        .insert(AUTH_KEY, true)
        .await
        .map_err(AdminError::session)?;
    session.cycle_id().await.map_err(AdminError::session)?;

    Ok(see_other("/admin"))
}

#[derive(Debug, Deserialize)]
pub(crate) struct CsrfForm {
    #[serde(default)]
    csrf_token: String,
}

pub(crate) async fn logout(
    session: Session,
    Form(form): Form<CsrfForm>,
) -> Result<Response, AdminError> {
    verify_csrf(&session, &form.csrf_token).await?;
    session.flush().await.map_err(AdminError::session)?;
    Ok(see_other("/admin/login"))
}

pub(crate) async fn dashboard(
    State(state): State<AdminState>,
    session: Session,
) -> Result<Response, AdminError> {
    let channels = state.db.channels().list(false).await?.len();
    let keys = state.db.api_keys().list().await?.len();
    let recent = state.db.logs().list_recent(10, 0).await?.len();
    let csrf_token = csrf_token(&session).await?;

    Ok(DashboardPage {
        logged_in: true,
        csrf_token,
        channel_count: channels,
        key_count: keys,
        recent_count: recent,
    }
    .into_response())
}

pub(crate) async fn channels(
    State(state): State<AdminState>,
    session: Session,
) -> Result<Response, AdminError> {
    let channels = state
        .db
        .channels()
        .list(false)
        .await?
        .into_iter()
        .map(|channel| {
            let voice_namespace = channel.namespace().to_owned();
            let voices_count = channel.voices.len();
            ChannelRow {
                id: channel.id,
                name: channel.name,
                kind: kind_value(channel.kind).to_owned(),
                base_url: channel.base_url,
                api_key_set: channel.api_key.is_some(),
                models: channel.models.join(", "),
                voices_count,
                voice_namespace,
                priority: channel.priority,
                weight: channel.weight,
                enabled: channel.enabled,
            }
        })
        .collect();
    let csrf_token = csrf_token(&session).await?;

    Ok(ChannelsPage {
        logged_in: true,
        csrf_token,
        channels,
    }
    .into_response())
}

pub(crate) async fn channel_new(session: Session) -> Result<Response, AdminError> {
    let csrf_token = csrf_token(&session).await?;
    Ok(ChannelFormPage {
        logged_in: true,
        csrf_token,
        editing: false,
        action: "/admin/channels".to_owned(),
        error: None,
        values: ChannelFormValues {
            name: String::new(),
            kind: "openai_compatible".to_owned(),
            base_url: String::new(),
            models: String::new(),
            voices: String::new(),
            voice_namespace: String::new(),
            priority: "0".to_owned(),
            weight: "100".to_owned(),
            enabled: true,
        },
    }
    .into_response())
}

pub(crate) async fn channel_edit(
    State(state): State<AdminState>,
    session: Session,
    Path(id): Path<i64>,
) -> Result<Response, AdminError> {
    let channel = state
        .db
        .channels()
        .get(id)
        .await?
        .ok_or(AdminError::NotFound)?;
    let csrf_token = csrf_token(&session).await?;

    Ok(ChannelFormPage {
        logged_in: true,
        csrf_token,
        editing: true,
        action: format!("/admin/channels/{id}"),
        error: None,
        values: ChannelFormValues {
            name: channel.name,
            kind: kind_value(channel.kind).to_owned(),
            base_url: channel.base_url.unwrap_or_default(),
            models: channel.models.join("\n"),
            voices: channel.voices.join("\n"),
            voice_namespace: channel.voice_namespace.unwrap_or_default(),
            priority: channel.priority.to_string(),
            weight: channel.weight.to_string(),
            enabled: channel.enabled,
        },
    }
    .into_response())
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChannelForm {
    #[serde(default)]
    csrf_token: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    base_url: String,
    #[serde(default)]
    api_key: String,
    #[serde(default)]
    models: String,
    #[serde(default)]
    voices: String,
    #[serde(default)]
    voice_namespace: String,
    #[serde(default)]
    priority: String,
    #[serde(default)]
    weight: String,
    #[serde(default)]
    enabled: Option<String>,
}

impl ChannelForm {
    fn values(&self) -> ChannelFormValues {
        ChannelFormValues {
            name: self.name.clone(),
            kind: if self.kind.is_empty() {
                "openai_compatible".to_owned()
            } else {
                self.kind.clone()
            },
            base_url: self.base_url.clone(),
            models: self.models.clone(),
            voices: self.voices.clone(),
            voice_namespace: self.voice_namespace.clone(),
            priority: self.priority.clone(),
            weight: self.weight.clone(),
            enabled: self.enabled.is_some(),
        }
    }
}

struct ParsedChannel {
    name: String,
    kind: ChannelKind,
    base_url: Option<String>,
    api_key: Option<String>,
    models: Vec<String>,
    voices: Vec<String>,
    voice_namespace: Option<String>,
    priority: i64,
    weight: u32,
    enabled: bool,
}

fn parse_channel(form: &ChannelForm) -> Result<ParsedChannel, String> {
    let name = form.name.trim();
    if name.is_empty() {
        return Err("Name is required.".to_owned());
    }

    let kind = match form.kind.trim() {
        "" | "openai_compatible" => ChannelKind::OpenAiCompatible,
        "edge_tts" => ChannelKind::EdgeTts,
        "doubao_tts" => ChannelKind::DoubaoTts,
        other => return Err(format!("Unknown channel kind `{other}`.")),
    };

    let mut models = Vec::new();
    for model in form.models.split([',', '\n']) {
        let model = model.trim();
        if !model.is_empty() && !models.iter().any(|existing| existing == model) {
            models.push(model.to_owned());
        }
    }
    if models.is_empty() {
        return Err("At least one model is required.".to_owned());
    }

    let voice_namespace = {
        let namespace = form.voice_namespace.trim().to_ascii_lowercase();
        if namespace.is_empty() {
            None
        } else if namespace.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || character == '_'
                || character == '-'
        }) {
            Some(namespace)
        } else {
            return Err(
                "Voice namespace may only contain lowercase letters, digits, `_`, and `-`."
                    .to_owned(),
            );
        }
    };
    let effective_namespace = voice_namespace
        .as_deref()
        .unwrap_or_else(|| kind.default_voice_namespace());

    let mut voices = Vec::new();
    for voice in form.voices.split([',', '\n', '\r']) {
        let voice = voice.trim();
        if voice.is_empty() {
            continue;
        }
        let stored = match split_voice_reference(voice) {
            (Some(namespace), bare) => {
                if !kind.accepts_namespace(effective_namespace, namespace) {
                    return Err(format!(
                        "Voice `{voice}` uses namespace `{namespace}`, but this channel uses `{effective_namespace}`."
                    ));
                }
                bare
            }
            (None, bare) => bare,
        };
        if !voices.iter().any(|existing| existing == stored) {
            voices.push(stored.to_owned());
        }
    }

    let priority = if form.priority.trim().is_empty() {
        0
    } else {
        form.priority
            .trim()
            .parse::<i64>()
            .map_err(|_| "Priority must be an integer.".to_owned())?
    };
    let weight = if form.weight.trim().is_empty() {
        100
    } else {
        form.weight
            .trim()
            .parse::<u64>()
            .map_err(|_| "Weight must be a non-negative integer.".to_owned())?
    };
    if weight > u64::from(u32::MAX) {
        return Err(format!("Weight must be at most {}.", u32::MAX));
    }

    let base_url = non_empty(&form.base_url);
    if kind != ChannelKind::EdgeTts && base_url.is_none() {
        return Err("Base URL is required for this channel kind.".to_owned());
    }

    Ok(ParsedChannel {
        name: name.to_owned(),
        kind,
        base_url,
        api_key: non_empty(&form.api_key),
        models,
        voices,
        voice_namespace,
        priority,
        weight: weight as u32,
        enabled: form.enabled.is_some(),
    })
}

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

pub(crate) async fn channel_create(
    State(state): State<AdminState>,
    session: Session,
    Form(form): Form<ChannelForm>,
) -> Result<Response, AdminError> {
    verify_csrf(&session, &form.csrf_token).await?;

    let parsed = match parse_channel(&form) {
        Ok(parsed) => parsed,
        Err(error) => {
            let csrf_token = csrf_token(&session).await?;
            return Ok(render_with_status(
                StatusCode::BAD_REQUEST,
                ChannelFormPage {
                    logged_in: true,
                    csrf_token,
                    editing: false,
                    action: "/admin/channels".to_owned(),
                    error: Some(error),
                    values: form.values(),
                },
            ));
        }
    };

    state
        .db
        .channels()
        .create(NewChannel {
            name: parsed.name,
            kind: parsed.kind,
            base_url: parsed.base_url,
            api_key: parsed.api_key,
            models: parsed.models,
            voices: parsed.voices,
            voice_namespace: parsed.voice_namespace,
            priority: parsed.priority,
            weight: parsed.weight,
            enabled: parsed.enabled,
        })
        .await?;

    Ok(see_other("/admin/channels"))
}

pub(crate) async fn channel_update(
    State(state): State<AdminState>,
    session: Session,
    Path(id): Path<i64>,
    Form(form): Form<ChannelForm>,
) -> Result<Response, AdminError> {
    verify_csrf(&session, &form.csrf_token).await?;

    let parsed = match parse_channel(&form) {
        Ok(parsed) => parsed,
        Err(error) => {
            let csrf_token = csrf_token(&session).await?;
            return Ok(render_with_status(
                StatusCode::BAD_REQUEST,
                ChannelFormPage {
                    logged_in: true,
                    csrf_token,
                    editing: true,
                    action: format!("/admin/channels/{id}"),
                    error: Some(error),
                    values: form.values(),
                },
            ));
        }
    };

    state
        .db
        .channels()
        .update(
            id,
            ChannelUpdate {
                name: Some(parsed.name),
                kind: Some(parsed.kind),
                base_url: Some(parsed.base_url),
                // None leaves the stored key untouched.
                api_key: parsed.api_key.map(Some),
                models: Some(parsed.models),
                voices: Some(parsed.voices),
                voice_namespace: Some(parsed.voice_namespace),
                priority: Some(parsed.priority),
                weight: Some(parsed.weight),
                enabled: Some(parsed.enabled),
            },
        )
        .await?;

    Ok(see_other("/admin/channels"))
}

pub(crate) async fn channel_delete(
    State(state): State<AdminState>,
    session: Session,
    Path(id): Path<i64>,
    Form(form): Form<CsrfForm>,
) -> Result<Response, AdminError> {
    verify_csrf(&session, &form.csrf_token).await?;
    if !state.db.channels().delete(id).await? {
        return Err(AdminError::NotFound);
    }
    Ok(see_other("/admin/channels"))
}

pub(crate) async fn channel_toggle(
    State(state): State<AdminState>,
    session: Session,
    Path(id): Path<i64>,
    Form(form): Form<CsrfForm>,
) -> Result<Response, AdminError> {
    verify_csrf(&session, &form.csrf_token).await?;
    let channel = state
        .db
        .channels()
        .get(id)
        .await?
        .ok_or(AdminError::NotFound)?;
    if !state
        .db
        .channels()
        .set_enabled(id, !channel.enabled)
        .await?
    {
        return Err(AdminError::NotFound);
    }
    Ok(see_other("/admin/channels"))
}

const fn kind_value(kind: ChannelKind) -> &'static str {
    match kind {
        ChannelKind::OpenAiCompatible => "openai_compatible",
        ChannelKind::EdgeTts => "edge_tts",
        ChannelKind::DoubaoTts => "doubao_tts",
    }
}

pub(crate) async fn keys(
    State(state): State<AdminState>,
    session: Session,
) -> Result<Response, AdminError> {
    let keys = state.db.api_keys().list().await?;
    let csrf_token = csrf_token(&session).await?;
    Ok(KeysPage {
        logged_in: true,
        csrf_token,
        keys,
        new_key: None,
        error: None,
    }
    .into_response())
}

#[derive(Debug, Deserialize)]
pub(crate) struct KeyForm {
    #[serde(default)]
    csrf_token: String,
    #[serde(default)]
    name: String,
}

pub(crate) async fn key_create(
    State(state): State<AdminState>,
    session: Session,
    Form(form): Form<KeyForm>,
) -> Result<Response, AdminError> {
    verify_csrf(&session, &form.csrf_token).await?;
    let csrf_token = csrf_token(&session).await?;

    let name = form.name.trim();
    if name.is_empty() {
        let keys = state.db.api_keys().list().await?;
        return Ok(render_with_status(
            StatusCode::BAD_REQUEST,
            KeysPage {
                logged_in: true,
                csrf_token,
                keys,
                new_key: None,
                error: Some("Name is required.".to_owned()),
            },
        ));
    }

    let generated = keys::generate_key();
    state
        .db
        .api_keys()
        .create(NewApiKey {
            name: name.to_owned(),
            key_hash: generated.hash,
            key_prefix: generated.prefix,
        })
        .await?;

    let keys = state.db.api_keys().list().await?;
    Ok(KeysPage {
        logged_in: true,
        csrf_token,
        keys,
        new_key: Some(generated.plaintext),
        error: None,
    }
    .into_response())
}

pub(crate) async fn key_revoke(
    State(state): State<AdminState>,
    session: Session,
    Path(id): Path<i64>,
    Form(form): Form<CsrfForm>,
) -> Result<Response, AdminError> {
    verify_csrf(&session, &form.csrf_token).await?;
    if !state.db.api_keys().revoke(id).await? {
        return Err(AdminError::NotFound);
    }
    Ok(see_other("/admin/keys"))
}

#[derive(Debug, Deserialize)]
pub(crate) struct LogsQuery {
    #[serde(default = "default_limit")]
    limit: u32,
    #[serde(default)]
    offset: u32,
}

fn default_limit() -> u32 {
    50
}

pub(crate) async fn logs(
    State(state): State<AdminState>,
    session: Session,
    Query(query): Query<LogsQuery>,
) -> Result<Response, AdminError> {
    let limit = query.limit.clamp(1, 200);
    let logs = state.db.logs().list_recent(limit, query.offset).await?;
    let csrf_token = csrf_token(&session).await?;

    let prev_offset = (query.offset > 0).then(|| query.offset.saturating_sub(limit));
    let next_offset = (logs.len() >= limit as usize).then(|| query.offset.saturating_add(limit));

    Ok(LogsPage {
        logged_in: true,
        csrf_token,
        logs,
        limit,
        prev_offset,
        next_offset,
    }
    .into_response())
}

pub(crate) async fn log_detail(
    State(state): State<AdminState>,
    session: Session,
    Path(request_id): Path<String>,
) -> Result<Response, AdminError> {
    let detail = state
        .db
        .logs()
        .get_by_request_id(&request_id)
        .await?
        .ok_or(AdminError::NotFound)?;
    let csrf_token = csrf_token(&session).await?;

    Ok(LogDetailPage {
        logged_in: true,
        csrf_token,
        log: detail.log,
        attempts: detail.attempts,
    }
    .into_response())
}
