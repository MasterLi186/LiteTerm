use super::ProgressThrottle;
use std::time::{Duration, Instant};

#[test]
fn progress_throttle_reports_first_interval_and_completion() {
    let start = Instant::now();
    let mut throttle = ProgressThrottle::new(start);
    assert!(!throttle.should_emit(start + Duration::from_millis(50), 50, 100));
    assert!(throttle.should_emit(start + Duration::from_millis(100), 60, 100));
    assert!(throttle.should_emit(start + Duration::from_millis(101), 100, 100));
}
