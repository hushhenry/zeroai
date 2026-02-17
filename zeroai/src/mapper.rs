/// Split a full model ID (e.g. "openai/gpt-4o") into (provider, short_id).
pub fn split_model_id(full_id: &str) -> Option<(&str, &str)> {
    let slash = full_id.find('/')?;
    if slash == 0 || slash == full_id.len() - 1 {
        return None;
    }
    Some((&full_id[..slash], &full_id[slash + 1..]))
}

/// Join a provider and short model ID into a full model ID.
pub fn join_model_id(provider: &str, short_id: &str) -> String {
    format!("{}/{}", provider, short_id)
}
