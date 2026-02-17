//! Retry logic for provider calls: exponential backoff, non-retryable 4xx detection,
//! rate-limit (429) and Retry-After handling. Design reference: zeroclaw providers/reliable.rs

use super::{Provider, ProviderError};
use crate::types::{ChatContext, ModelDef, RequestOptions, RetryConfig, StreamEvent};
use futures::stream::{BoxStream, StreamExt};
use std::sync::Arc;
use std::time::Duration;

/// True if the error is a client error (4xx) that should not be retried (excluding 429 and 408).
pub fn is_non_retryable(err: &ProviderError) -> bool {
    match err {
        ProviderError::Http { status, .. } => {
            let code = *status;
            (400..500).contains(&code) && code != 429 && code != 408
        }
        ProviderError::AuthRequired(_) => true,
        ProviderError::RateLimited { .. } => false,
        _ => {
            let msg = err.to_string();
            for word in msg.split(|c: char| !c.is_ascii_digit()) {
                if let Ok(code) = word.parse::<u16>() {
                    if (400..500).contains(&code) && code != 429 && code != 408 {
                        return true;
                    }
                }
            }
            false
        }
    }
}

/// True if the error indicates rate limiting (429).
pub fn is_rate_limited(err: &ProviderError) -> bool {
    match err {
        ProviderError::Http { status, .. } => *status == 429,
        ProviderError::RateLimited { .. } => true,
        _ => {
            let msg = err.to_string();
            msg.contains("429")
                && (msg.contains("Too Many") || msg.contains("rate") || msg.contains("limit"))
        }
    }
}

/// Extract Retry-After delay in milliseconds from error (body/message or RateLimited variant).
pub fn parse_retry_after_ms(err: &ProviderError) -> Option<u64> {
    if let ProviderError::RateLimited {
        retry_after_ms: Some(ms),
    } = err
    {
        return Some(*ms);
    }
    let msg = err.to_string();
    let lower = msg.to_lowercase();
    for prefix in &[
        "retry-after:",
        "retry_after:",
        "retry-after ",
        "retry_after ",
    ] {
        if let Some(pos) = lower.find(prefix) {
            let after = &msg[pos + prefix.len()..];
            let num_str: String = after
                .trim()
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if let Ok(secs) = num_str.parse::<f64>() {
                if secs.is_finite() && secs >= 0.0 {
                    let millis = Duration::from_secs_f64(secs).as_millis();
                    if let Ok(value) = u64::try_from(millis) {
                        return Some(value);
                    }
                }
            }
        }
    }
    None
}

/// Next backoff in ms: Retry-After if present (capped at 30s), else base; base is doubled for next call.
pub fn compute_backoff(config: &RetryConfig, base_ms: u64, err: &ProviderError) -> u64 {
    let base = base_ms.max(config.base_backoff_ms.min(1));
    if let Some(retry_after) = parse_retry_after_ms(err) {
        retry_after.min(30_000).max(base)
    } else {
        base
    }
}

/// Stream that retries on retryable errors (429/408, network) with exponential backoff.
pub fn retry_stream(
    provider: Arc<dyn Provider>,
    model_def: ModelDef,
    context: ChatContext,
    options: RequestOptions,
    config: RetryConfig,
) -> BoxStream<'static, Result<StreamEvent, ProviderError>> {
    let stream = async_stream::stream! {
        let mut attempt = 0u32;
        let mut backoff_ms = config.base_backoff_ms;
        loop {
            let mut inner = provider.stream(&model_def, &context, &options);
            loop {
                match inner.next().await {
                    None => return,
                    Some(Ok(evt)) => yield Ok(evt),
                    Some(Err(e)) => {
                        if is_non_retryable(&e) || attempt >= config.max_retries {
                            yield Err(e);
                            return;
                        }
                        let wait = compute_backoff(&config, backoff_ms, &e);
                        tokio::time::sleep(Duration::from_millis(wait)).await;
                        backoff_ms = (backoff_ms.saturating_mul(2)).min(10_000);
                        attempt += 1;
                        break;
                    }
                }
            }
        }
    };
    Box::pin(stream)
}
