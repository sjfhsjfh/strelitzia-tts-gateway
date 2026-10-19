mod doubao;
mod edge;
mod mock;
mod openai;
mod registry;

pub use doubao::DoubaoTtsAdapter;
pub use edge::EdgeTtsAdapter;
pub use mock::MockAdapter;
pub use openai::OpenAiCompatibleAdapter;
pub use registry::AdapterRegistry;
