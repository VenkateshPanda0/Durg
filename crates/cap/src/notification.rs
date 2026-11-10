//! Pure state for a single coalescing notification object.
//!
//! The scheduler must serialize access to this state and manage waiters. This
//! type does not wake threads, access physical memory, deliver interrupts or
//! provide cross-CPU synchronization. Its all-zero representation is idle, so
//! a retyped and zeroed notification starts without a pending signal.

/// Serialized notification state size in bytes.
pub const NOTIFICATION_BYTES: usize = 16;

/// A notification's pending bit and OR-combined signal badges.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NotificationState {
    pending: bool,
    badge: u64,
}

impl NotificationState {
    /// Creates an idle notification.
    pub const fn new() -> Self {
        Self {
            pending: false,
            badge: 0,
        }
    }

    /// Records a signal, combining its badge with any already-pending badge.
    /// A zero badge is still a signal and can be observed by [`take`](Self::take).
    pub fn signal(&mut self, badge: u64) {
        self.pending = true;
        self.badge |= badge;
    }

    /// Consumes the pending signal and returns its combined badge, if present.
    pub fn take(&mut self) -> Option<u64> {
        if !self.pending {
            return None;
        }
        let badge = self.badge;
        self.pending = false;
        self.badge = 0;
        Some(badge)
    }

    /// Serializes as a little-endian pending word followed by a badge word.
    pub fn to_bytes(self) -> [u8; NOTIFICATION_BYTES] {
        let mut out = [0; NOTIFICATION_BYTES];
        out[..8].copy_from_slice(&u64::from(self.pending).to_le_bytes());
        out[8..].copy_from_slice(&self.badge.to_le_bytes());
        out
    }

    /// Decodes state created by this type, rejecting malformed or stale bytes.
    pub fn from_bytes(bytes: &[u8; NOTIFICATION_BYTES]) -> Option<Self> {
        let pending = u64::from_le_bytes(bytes[..8].try_into().ok()?);
        let badge = u64::from_le_bytes(bytes[8..].try_into().ok()?);
        match pending {
            0 if badge == 0 => Some(Self::new()),
            1 => Some(Self {
                pending: true,
                badge,
            }),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn zero_badge_is_not_an_empty_notification() {
        let mut state = NotificationState::new();
        assert_eq!(state.to_bytes(), [0; NOTIFICATION_BYTES]);
        assert_eq!(state.take(), None);
        state.signal(0);
        assert_eq!(state.take(), Some(0));
        assert_eq!(state.take(), None);
    }

    #[test]
    fn badges_coalesce_and_are_cleared_after_delivery() {
        let mut state = NotificationState::new();
        state.signal(1 << 63);
        state.signal(0x12);
        state.signal(0x24);
        assert_eq!(state.take(), Some((1 << 63) | 0x36));
        assert_eq!(state.to_bytes(), [0; NOTIFICATION_BYTES]);
        state.signal(0x80);
        assert_eq!(state.take(), Some(0x80));
    }

    #[test]
    fn serialized_state_round_trips_and_rejects_invalid_words() {
        let mut state = NotificationState::new();
        assert_eq!(
            NotificationState::from_bytes(&state.to_bytes()),
            Some(state)
        );
        state.signal(u64::MAX);
        assert_eq!(
            NotificationState::from_bytes(&state.to_bytes()),
            Some(state)
        );
        let mut bytes = [0; NOTIFICATION_BYTES];
        bytes[0] = 2;
        assert_eq!(NotificationState::from_bytes(&bytes), None);
        bytes[0] = 0;
        bytes[8] = 1;
        assert_eq!(NotificationState::from_bytes(&bytes), None);
        bytes[8] = 0;
        bytes[7] = 1;
        assert_eq!(NotificationState::from_bytes(&bytes), None);
    }

    proptest! {
        #[test]
        fn operation_sequences_match_an_option_model(ops in prop::collection::vec((any::<bool>(), any::<u64>()), 0..500)) {
            let mut state = NotificationState::new();
            let mut model: Option<u64> = None;
            for (signal, badge) in ops {
                if signal {
                    state.signal(badge);
                    model = Some(model.unwrap_or(0) | badge);
                } else {
                    prop_assert_eq!(state.take(), model.take());
                }
                prop_assert_eq!(NotificationState::from_bytes(&state.to_bytes()), Some(state));
            }
            prop_assert_eq!(state.take(), model);
        }
    }
}
