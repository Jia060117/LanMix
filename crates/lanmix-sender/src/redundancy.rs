//! Delayed, byte-identical backups. Only the non-real-time sender worker uses this.
use lanmix_proto::MAX_DATAGRAM_BYTES;
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

const CAPACITY: usize = 128;

struct Backup {
    bytes: [u8; MAX_DATAGRAM_BYTES],
    len: usize,
    due: Instant,
}

pub struct Redundancy {
    delay: Duration,
    pending: VecDeque<Backup>,
    pub skipped: u64,
}

impl Redundancy {
    pub fn new(delay_ms: u32) -> Self {
        Self {
            delay: Duration::from_millis(delay_ms as u64),
            pending: VecDeque::with_capacity(CAPACITY),
            skipped: 0,
        }
    }

    pub fn remember(&mut self, bytes: &[u8], now: Instant) {
        if self.delay.is_zero() {
            return;
        }
        if self.pending.len() == CAPACITY {
            // Never allow redundancy to accumulate stale, unbounded work.
            self.pending.pop_front();
            self.skipped += 1;
        }
        let mut backup = Backup {
            bytes: [0; MAX_DATAGRAM_BYTES],
            len: bytes.len(),
            due: now + self.delay,
        };
        backup.bytes[..bytes.len()].copy_from_slice(bytes);
        self.pending.push_back(backup);
    }

    pub fn flush(&mut self, now: Instant, mut send: impl FnMut(&[u8])) {
        while self.pending.front().is_some_and(|p| p.due <= now) {
            let backup = self.pending.pop_front().unwrap();
            if now.saturating_duration_since(backup.due) > Duration::from_millis(100) {
                self.skipped += 1;
                continue;
            }
            send(&backup.bytes[..backup.len]);
        }
    }

    pub fn clear(&mut self) {
        self.pending.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backups_are_delayed_identical_once_and_flushed_without_new_audio() {
        let now = Instant::now();
        let mut repair = Redundancy::new(20);
        repair.remember(&[1, 2, 3], now);
        repair.flush(now + Duration::from_millis(19), |_| panic!("too early"));
        let mut copies = Vec::new();
        repair.flush(now + Duration::from_millis(20), |b| copies.push(b.to_vec()));
        repair.flush(now + Duration::from_secs(1), |_| panic!("replayed twice"));
        assert_eq!(copies, [vec![1, 2, 3]]);
    }

    #[test]
    fn disabled_bounded_and_session_reset() {
        let now = Instant::now();
        let mut disabled = Redundancy::new(0);
        disabled.remember(&[1], now);
        disabled.flush(now + Duration::from_secs(1), |_| panic!("disabled"));
        let mut repair = Redundancy::new(20);
        for _ in 0..CAPACITY + 10 {
            repair.remember(&[2], now);
        }
        assert_eq!(repair.pending.len(), CAPACITY);
        assert_eq!(repair.skipped, 10);
        repair.clear();
        repair.flush(now + Duration::from_secs(1), |_| panic!("old session"));
    }

    #[test]
    fn worker_stall_does_not_send_a_burst_of_expired_backups() {
        let now = Instant::now();
        let mut repair = Redundancy::new(20);
        repair.remember(&[1], now);
        repair.flush(now + Duration::from_millis(200), |_| panic!("stale backup"));
        assert_eq!(repair.skipped, 1);
    }
}
