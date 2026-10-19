CREATE TABLE channels (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    base_url TEXT,
    api_key TEXT,
    models TEXT NOT NULL,
    priority INTEGER NOT NULL DEFAULT 0,
    weight INTEGER NOT NULL DEFAULT 100,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE api_keys (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    key_hash TEXT NOT NULL UNIQUE,
    key_prefix TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    last_used_at TEXT
);

CREATE TABLE request_logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id TEXT NOT NULL UNIQUE,
    api_key_id INTEGER,
    model TEXT NOT NULL,
    voice TEXT NOT NULL,
    provider TEXT,
    channel_id INTEGER,
    chars INTEGER NOT NULL DEFAULT 0,
    status_code INTEGER NOT NULL,
    error TEXT,
    latency_ms INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE request_attempts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    request_log_id INTEGER NOT NULL REFERENCES request_logs(id) ON DELETE CASCADE,
    channel_id INTEGER,
    attempt_no INTEGER NOT NULL,
    outcome TEXT NOT NULL,
    error TEXT,
    latency_ms INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_request_logs_created_at ON request_logs(created_at);
CREATE INDEX idx_request_logs_api_key_id ON request_logs(api_key_id);
CREATE INDEX idx_request_attempts_request_log_id ON request_attempts(request_log_id);
CREATE INDEX idx_channels_enabled ON channels(enabled);
