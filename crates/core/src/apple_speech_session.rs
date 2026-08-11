//! Session-scoped safe-fallback latch for Apple Speech transcription.
//!
//! Apple Speech runs in a separate XPC worker process, so a worker crash is
//! failure-isolated: the parent observes an error and falls back to Whisper for
//! that utterance without losing it. This is the RFC 0004 failure-isolation
//! boundary and the standing "recording must never be degraded by an optional
//! consumer" decision, applied to engine selection.
//!
//! Runtime capability cannot be predicted before attempting (a device may lack
//! Speech assets, or abort while constructing the analyzer), so callers attempt
//! once and cache the verdict here: the first failure latches the session onto
//! Whisper, so the worker is never re-spawned only to crash again for the same
//! reason. The latch uses interior mutability (an atomic) so a shared `&self`
//! reference threads cleanly through a per-utterance loop without `&mut`
//! plumbing.

use std::sync::atomic::{AtomicU8, Ordering};

const UNKNOWN: u8 = 0;
const USABLE: u8 = 1;
const UNAVAILABLE: u8 = 2;

/// Per-session Apple Speech capability verdict, learned by attempting.
#[derive(Debug)]
pub struct AppleSpeechSession {
    state: AtomicU8,
}

impl Default for AppleSpeechSession {
    fn default() -> Self {
        Self::new()
    }
}

impl AppleSpeechSession {
    pub const fn new() -> Self {
        Self {
            state: AtomicU8::new(UNKNOWN),
        }
    }

    /// Whether an Apple Speech attempt is worth making. False once a failure has
    /// been recorded this session, so a known-incapable device goes straight to
    /// Whisper instead of re-spawning a worker that will abort.
    pub fn should_attempt(&self) -> bool {
        self.state.load(Ordering::Acquire) != UNAVAILABLE
    }

    /// Record that Apple Speech produced a usable transcript this session.
    pub fn record_success(&self) {
        // Never overwrite a latched failure: a mid-session recovery claim must
        // not re-arm a worker that already crashed once this session.
        let _ = self
            .state
            .compare_exchange(UNKNOWN, USABLE, Ordering::AcqRel, Ordering::Acquire);
    }

    /// Record that Apple Speech failed (crash, error, or unusable result).
    /// Latches: once unavailable, the session stays on Whisper.
    pub fn record_failure(&self) {
        self.state.store(UNAVAILABLE, Ordering::Release);
    }

    /// True once a failure has latched this session onto Whisper.
    pub fn is_unavailable(&self) -> bool {
        self.state.load(Ordering::Acquire) == UNAVAILABLE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_session_attempts_apple_speech() {
        let session = AppleSpeechSession::new();
        assert!(session.should_attempt());
        assert!(!session.is_unavailable());
    }

    #[test]
    fn a_failure_latches_the_session_off_apple_speech() {
        let session = AppleSpeechSession::new();
        session.record_failure();
        assert!(!session.should_attempt());
        assert!(session.is_unavailable());
    }

    #[test]
    fn a_success_keeps_the_session_attempting() {
        let session = AppleSpeechSession::new();
        session.record_success();
        assert!(session.should_attempt());
        assert!(!session.is_unavailable());
    }

    #[test]
    fn a_latched_failure_is_not_undone_by_a_later_success() {
        // A worker that crashed once this session must stay latched off, even if
        // some later code path reports a stray success: never re-spawn a crasher.
        let session = AppleSpeechSession::new();
        session.record_failure();
        session.record_success();
        assert!(session.is_unavailable());
        assert!(!session.should_attempt());
    }

    #[test]
    fn the_default_impl_matches_new() {
        let session = AppleSpeechSession::default();
        assert!(session.should_attempt());
    }
}
