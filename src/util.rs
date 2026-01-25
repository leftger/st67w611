//! Utility functions and helpers

use crate::error::Result;
use embassy_time::{Duration, Timer};

/// Retry a fallible async operation with exponential backoff
///
/// # Arguments
/// * `max_attempts` - Maximum number of attempts (including first try)
/// * `initial_delay` - Initial delay between retries
/// * `max_delay` - Maximum delay between retries
/// * `operation` - Async function to retry
///
/// # Returns
/// Result from the operation if successful, or the last error
pub async fn retry_with_backoff<F, Fut, T>(
    max_attempts: u8,
    initial_delay: Duration,
    max_delay: Duration,
    mut operation: F,
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: core::future::Future<Output = Result<T>>,
{
    let mut attempt = 0;
    let mut delay = initial_delay;

    loop {
        attempt += 1;

        match operation().await {
            Ok(result) => return Ok(result),
            Err(e) => {
                if attempt >= max_attempts {
                    return Err(e);
                }

                // Wait before retrying
                Timer::after(delay).await;

                // Exponential backoff with cap
                delay = Duration::from_millis(core::cmp::min(
                    delay.as_millis() * 2,
                    max_delay.as_millis(),
                ));
            }
        }
    }
}

/// Retry a fallible async operation with fixed delay
///
/// # Arguments
/// * `max_attempts` - Maximum number of attempts
/// * `delay` - Delay between retries
/// * `operation` - Async function to retry
pub async fn retry_fixed<F, Fut, T>(
    max_attempts: u8,
    delay: Duration,
    mut operation: F,
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: core::future::Future<Output = Result<T>>,
{
    let mut attempt = 0;

    loop {
        attempt += 1;

        match operation().await {
            Ok(result) => return Ok(result),
            Err(e) => {
                if attempt >= max_attempts {
                    return Err(e);
                }

                Timer::after(delay).await;
            }
        }
    }
}
