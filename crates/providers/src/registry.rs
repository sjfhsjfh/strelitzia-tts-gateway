use std::sync::Arc;

use strelitzia_core::adapter::TtsAdapter;
use strelitzia_core::domain::ChannelKind;

#[derive(Default)]
pub struct AdapterRegistry {
    adapters: Vec<(ChannelKind, Arc<dyn TtsAdapter>)>,
}

impl AdapterRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, adapter: Arc<dyn TtsAdapter>) -> Option<Arc<dyn TtsAdapter>> {
        let kind = adapter.kind();
        if let Some((_, existing)) = self.adapters.iter_mut().find(|(slot, _)| *slot == kind) {
            return Some(std::mem::replace(existing, adapter));
        }
        self.adapters.push((kind, adapter));
        None
    }

    pub fn get(&self, kind: ChannelKind) -> Option<Arc<dyn TtsAdapter>> {
        self.adapters
            .iter()
            .find(|(slot, _)| *slot == kind)
            .map(|(_, adapter)| Arc::clone(adapter))
    }
}
