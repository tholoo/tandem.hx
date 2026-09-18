use std::time::Duration;

/// Wait a little longer after each unsuccessful attempt.
pub fn retry_delay(attempt: u32) -> Duration {
    let millis = 100 * 2_u64.pow(attempt);
    Duration::from_millis(millis)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_small_and_doubles() {
        assert_eq!(retry_delay(0), Duration::from_millis(100));
        assert_eq!(retry_delay(1), Duration::from_millis(200));
    }
}
