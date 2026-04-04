pub(crate) const HOT_LOOP_THRESHOLD: u16 = 56;

#[inline(always)]
pub(crate) fn tick_hotcount(hits: &mut u16) -> bool {
    *hits = hits.saturating_add(1);
    *hits >= HOT_LOOP_THRESHOLD
}

#[cfg(test)]
mod tests {
    use super::{HOT_LOOP_THRESHOLD, tick_hotcount};

    #[test]
    fn hotcount_promotes_at_threshold() {
        let mut hits = 0;
        for _ in 0..(HOT_LOOP_THRESHOLD - 1) {
            assert!(!tick_hotcount(&mut hits));
        }

        assert!(tick_hotcount(&mut hits));
    }

    #[test]
    fn hotcount_saturates() {
        let mut hits = u16::MAX;
        assert!(tick_hotcount(&mut hits));
        assert_eq!(hits, u16::MAX);
    }
}