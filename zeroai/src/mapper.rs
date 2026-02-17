/// Helper for mapping between full model IDs (provider/model) and short IDs (model).
#[derive(Debug, Clone, Default)]
pub struct ModelMapper;

impl ModelMapper {
    pub fn new() -> Self {
        Self
    }

    /// Split a full model ID into (provider, short_id).
    pub fn split_id<'a>(&self, full_id: &'a str) -> Option<(&'a str, &'a str)> {
        let slash = full_id.find('/')?;
        if slash == 0 || slash == full_id.len() - 1 {
            return None;
        }
        Some((&full_id[..slash], &full_id[slash + 1..]))
    }

    /// Add provider prefix to a short model ID.
    pub fn join_id(&self, provider: &str, short_id: &str) -> String {
        format!("{}/{}", provider, short_id)
    }
}
