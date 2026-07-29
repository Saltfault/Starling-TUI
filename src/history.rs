use starling::history::TrustedStore;

/// Object-safe durable history backend shared by typed space handles.
pub trait HistoryBackend: TrustedStore + Send + Sync {}

impl<T> HistoryBackend for T where T: TrustedStore + Send + Sync {}
