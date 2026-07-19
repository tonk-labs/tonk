//! Bounded retry policy for the FAB's subscribing `ui-` children.
//!
//! A subscription that fails (a 404 from an unloadable space, say) is
//! resubscribed by the host without bound. The FAB mounts one subscribing
//! element per listed space, so an unbounded loop per stale entry is real
//! load. These children retry a few times with exponential backoff, then
//! give up and render a terminal state instead of retrying forever.

/// Retries before a subscription is declared dead.
pub const MAX_ATTEMPTS: u32 = 4;

/// Delay before the first retry; each subsequent retry doubles it.
const BASE_DELAY_MS: i32 = 500;

/// Exponential backoff with a hard attempt ceiling.
#[derive(Debug, Default)]
pub struct RetryPolicy {
    attempts: u32,
}

impl RetryPolicy {
    pub fn new() -> Self {
        Self { attempts: 0 }
    }

    /// The next backoff delay, or `None` once the ceiling is reached —
    /// the caller must then stop and render its terminal state.
    pub fn next_delay_ms(&mut self) -> Option<i32> {
        if self.attempts >= MAX_ATTEMPTS {
            return None;
        }
        let delay = BASE_DELAY_MS * (1 << self.attempts);
        self.attempts += 1;
        Some(delay)
    }

    /// Clear the attempt count — call on a frame that arrives successfully.
    pub fn reset(&mut self) {
        self.attempts = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_backs_off_exponentially() {
        let mut policy = RetryPolicy::new();
        assert_eq!(policy.next_delay_ms(), Some(500));
        assert_eq!(policy.next_delay_ms(), Some(1000));
        assert_eq!(policy.next_delay_ms(), Some(2000));
        assert_eq!(policy.next_delay_ms(), Some(4000));
    }

    #[test]
    fn it_gives_up_after_max_attempts() {
        let mut policy = RetryPolicy::new();
        for _ in 0..MAX_ATTEMPTS {
            assert!(policy.next_delay_ms().is_some());
        }
        assert_eq!(policy.next_delay_ms(), None);
    }

    #[test]
    fn it_restarts_after_reset() {
        let mut policy = RetryPolicy::new();
        while policy.next_delay_ms().is_some() {}
        policy.reset();
        assert_eq!(policy.next_delay_ms(), Some(500));
    }
}
