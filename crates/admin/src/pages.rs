use askama::Template;
use askama_web::WebTemplate;
use chrono::{DateTime, Utc};
use strelitzia_store::{ApiKey, RequestAttempt, RequestLog};

#[derive(Template, WebTemplate)]
#[template(path = "login.html")]
pub struct LoginPage {
    pub logged_in: bool,
    pub csrf_token: String,
    pub error: Option<String>,
}

#[derive(Template, WebTemplate)]
#[template(path = "dashboard.html")]
pub struct DashboardPage {
    pub logged_in: bool,
    pub csrf_token: String,
    pub channel_count: usize,
    pub key_count: usize,
    pub recent_count: usize,
}

pub struct ChannelRow {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub base_url: Option<String>,
    pub api_key_set: bool,
    pub models: String,
    pub voices_count: usize,
    pub voice_namespace: String,
    pub priority: i64,
    pub weight: u32,
    pub enabled: bool,
}

#[derive(Template, WebTemplate)]
#[template(path = "channels.html")]
pub struct ChannelsPage {
    pub logged_in: bool,
    pub csrf_token: String,
    pub channels: Vec<ChannelRow>,
}

pub struct ChannelFormValues {
    pub name: String,
    pub kind: String,
    pub base_url: String,
    pub models: String,
    pub voices: String,
    pub voice_namespace: String,
    pub priority: String,
    pub weight: String,
    pub enabled: bool,
}

#[derive(Template, WebTemplate)]
#[template(path = "channel_form.html")]
pub struct ChannelFormPage {
    pub logged_in: bool,
    pub csrf_token: String,
    pub editing: bool,
    pub action: String,
    pub error: Option<String>,
    pub values: ChannelFormValues,
}

#[derive(Template, WebTemplate)]
#[template(path = "keys.html")]
pub struct KeysPage {
    pub logged_in: bool,
    pub csrf_token: String,
    pub keys: Vec<ApiKey>,
    pub new_key: Option<String>,
    pub error: Option<String>,
}

#[derive(Template, WebTemplate)]
#[template(path = "logs.html")]
pub struct LogsPage {
    pub logged_in: bool,
    pub csrf_token: String,
    pub logs: Vec<RequestLog>,
    pub limit: u32,
    pub prev_offset: Option<u32>,
    pub next_offset: Option<u32>,
}

#[derive(Template, WebTemplate)]
#[template(path = "log_detail.html")]
pub struct LogDetailPage {
    pub logged_in: bool,
    pub csrf_token: String,
    pub log: RequestLog,
    pub attempts: Vec<RequestAttempt>,
}

pub trait Timestamp {
    fn stamp(&self) -> String;
}

impl Timestamp for DateTime<Utc> {
    fn stamp(&self) -> String {
        self.format("%Y-%m-%d %H:%M:%S UTC").to_string()
    }
}
