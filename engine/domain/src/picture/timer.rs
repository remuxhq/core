//! Display of a transient countdown, clamped at zero.

pub fn clock(remaining_seconds: i64) -> String {
    let remaining = remaining_seconds.max(0);
    format!("{}:{:02}", remaining / 60, remaining % 60)
}

#[cfg(test)]
mod tests {
    #[test]
    fn countdown_stays_at_zero() {
        assert_eq!(super::clock(61), "1:01");
        assert_eq!(super::clock(-5), "0:00");
    }
}
