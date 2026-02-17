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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::RetryConfig;

    fn http_err(status: u16) -> ProviderError {
        ProviderError::Http {
            status,
            body: String::new(),
        }
    }

    #[test]
    fn is_non_retryable_4xx_except_429_408() {
        assert!(is_non_retryable(&http_err(400)));
        assert!(is_non_retryable(&http_err(401)));
        assert!(is_non_retryable(&http_err(403)));
        assert!(is_non_retryable(&http_err(404)));
        assert!(!is_non_retryable(&http_err(429)));
        assert!(!is_non_retryable(&http_err(408)));
    }

    #[test]
    fn is_non_retryable_5xx_and_other() {
        assert!(!is_non_retryable(&http_err(500)));
        assert!(!is_non_retryable(&http_err(502)));
        assert!(!is_non_retryable(&ProviderError::Other("timeout".into())));
        assert!(!is_non_retryable(&ProviderError::Other("connection reset".into())));
    }

    #[test]
    fn is_non_retryable_auth_required() {
        assert!(is_non_retryable(&ProviderError::AuthRequired("key required".into())));
    }

    #[test]
    fn is_non_retryable_parses_message_for_status() {
        assert!(is_non_retryable(&ProviderError::Other("400 Bad Request".into())));
        assert!(is_non_retryable(&ProviderError::Other("401 Unauthorized".into())));
        assert!(!is_non_retryable(&ProviderError::Other("429 Too Many Requests".into())));
    }

    #[test]
    fn is_rate_limited_429() {
        assert!(is_rate_limited(&http_err(429)));
        assert!(is_rate_limited(&ProviderError::RateLimited { retry_after_ms: None }));
    }

    #[test]
    fn is_rate_limited_others_false() {
        assert!(!is_rate_limited(&http_err(400)));
        assert!(!is_rate_limited(&http_err(500)));
        assert!(!is_rate_limited(&ProviderError::Other("timeout".into())));
    }

    #[test]
    fn is_rate_limited_message_contains_429_and_keyword() {
        assert!(is_rate_limited(&ProviderError::Other("429 Too Many Requests".into())));
        assert!(is_rate_limited(&ProviderError::Other("HTTP 429 rate limit exceeded".into())));
    }

    #[test]
    fn parse_retry_after_ms_integer() {
        let err = ProviderError::Http {
            status: 429,
            body: "429 Too Many Requests, Retry-After: 5".into(),
        };
        assert_eq!(parse_retry_after_ms(&err), Some(5000));
    }

    #[test]
    fn parse_retry_after_ms_float() {
        let err = ProviderError::Other("Rate limited. retry_after: 2.5 seconds".into());
        assert_eq!(parse_retry_after_ms(&err), Some(2500));
    }

    #[test]
    fn parse_retry_after_ms_missing() {
        let err = ProviderError::Other("500 Internal Server Error".into());
        assert_eq!(parse_retry_after_ms(&err), None);
    }

    #[test]
    fn parse_retry_after_ms_from_rate_limited_variant() {
        let err = ProviderError::RateLimited {
            retry_after_ms: Some(3000),
        };
        assert_eq!(parse_retry_after_ms(&err), Some(3000));
    }

    #[test]
    fn compute_backoff_uses_retry_after() {
        let config = RetryConfig::default();
        let err = ProviderError::Http {
            status: 429,
            body: "429 Retry-After: 3".into(),
        };
        assert_eq!(compute_backoff(&config, 500, &err), 3000);
    }

    #[test]
    fn compute_backoff_caps_at_30s() {
        let config = RetryConfig::default();
        let err = ProviderError::Http {
            status: 429,
            body: "429 Retry-After: 120".into(),
        };
        assert_eq!(compute_backoff(&config, 500, &err), 30_000);
    }

    #[test]
    fn compute_backoff_falls_back_to_base() {
        let config = RetryConfig::default();
        let err = ProviderError::Other("500 Server Error".into());
        // When no Retry-After, uses base (base_ms clamped by config.base_backoff_ms minimum)
        assert_eq!(compute_backoff(&config, 500, &err), 500);
        assert_eq!(compute_backoff(&config, 2000, &err), 2000);
    }
}
