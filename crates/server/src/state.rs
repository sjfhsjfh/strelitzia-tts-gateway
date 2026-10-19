use std::sync::Arc;
use std::time::Duration;

use strelitzia_providers::AdapterRegistry;
use strelitzia_routing::RetryPolicy;
use strelitzia_store::Db;

#[derive(Clone)]
pub struct AppState {
    inner: Arc<Inner>,
}

struct Inner {
    db: Db,
    adapters: AdapterRegistry,
    client: reqwest::Client,
    retry: RetryPolicy,
    upstream_timeout: Duration,
}

impl AppState {
    pub fn new(
        db: Db,
        adapters: AdapterRegistry,
        client: reqwest::Client,
        retry: RetryPolicy,
        upstream_timeout: Duration,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                db,
                adapters,
                client,
                retry,
                upstream_timeout,
            }),
        }
    }

    pub fn db(&self) -> &Db {
        &self.inner.db
    }

    pub fn adapters(&self) -> &AdapterRegistry {
        &self.inner.adapters
    }

    pub fn client(&self) -> &reqwest::Client {
        &self.inner.client
    }

    pub fn retry(&self) -> RetryPolicy {
        self.inner.retry
    }

    pub fn upstream_timeout(&self) -> Duration {
        self.inner.upstream_timeout
    }
}
