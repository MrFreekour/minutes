//! Session-scoped safe fallback for Apple Speech transcription.
//!
//! Apple Speech runs in a separate XPC worker process, so a worker crash is
//! failure-isolated: the parent observes an error and can fall back to Whisper
//! for that utterance without losing it. This is the RFC 0004 failure-isolation
//! boundary and the standing "recording must never be degraded by an optional
//! consumer" decision, applied to engine selection.
//!
//! Runtime capability cannot be predicted before attempting (a device may lack
//! Speech assets, or abort constructing the analyzer), so this attempts once
//! and caches the verdict: the first failure marks Apple Speech unavailable for
//! the rest of the session, so the worker is never re-spawned only to crash
//! again. The orchestrator is generic over the transcribe closures so it is
//! unit-testable without a real Speech runtime.

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
        self.state.store(USABLE, Ordering::Release);
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

/// Outcome of a fallback-orchestrated utterance: which engine produced the
/// transcript, so callers can surface the honest backend and log shadow deltas.
#[derive(Debug, PartialEq, Eq)]
pub enum Engine {
    AppleSpeech,
    Whisper,
}

/// Attempt Apple Speech for one utterance, falling back to Whisper on any
/// failure, and never losing the utterance.
///
/// `attempt_apple` returns `Ok(t)` only when the worker returned a response; a
/// worker crash or XPC interruption is an `Err`. `is_usable` decides whether an
/// `Ok` response is a real transcript (runtime supported and non-empty) versus a
/// structured "unsupported" result that must still fall back. `whisper` is
/// infallible from the caller's perspective: it is the guaranteed transcript, so
/// the utterance is never dropped.
///
/// A failure (error or unusable result) latches the session onto Whisper for
/// every later utterance.
pub fn transcribe_or_fall_back<T, E>(
    session: &AppleSpeechSession,
    attempt_apple: impl FnOnce() -> Result<T, E>,
    is_usable: impl FnOnce(&T) -> bool,
    whisper: impl FnOnce() -> T,
) -> (Engine, T) {
    if session.should_attempt() {
        if let Ok(result) = attempt_apple() {
            if is_usable(&result) {
                session.record_success();
                return (Engine::AppleSpeech, result);
            }
        }
        // Error or unusable result: latch the session onto Whisper.
        session.record_failure();
    }
    (Engine::Whisper, whisper())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_usable_apple_result_is_returned_and_keeps_the_session_usable() {
        let session = AppleSpeechSession::new();
        let (engine, text) = transcribe_or_fall_back(
            &session,
            || Ok::<_, ()>("apple transcript".to_string()),
            |t| !t.is_empty(),
            || "whisper transcript".to_string(),
        );
        assert_eq!(engine, Engine::AppleSpeech);
        assert_eq!(text, "apple transcript");
        assert!(session.should_attempt());
        assert!(!session.is_unavailable());
    }

    #[test]
    fn a_worker_error_falls_back_to_whisper_and_latches_unavailable() {
        let session = AppleSpeechSession::new();
        let (engine, text) = transcribe_or_fall_back(
            &session,
            || Err::<String, _>("worker crashed (XPC interrupted)"),
            |t| !t.is_empty(),
            || "whisper transcript".to_string(),
        );
        assert_eq!(engine, Engine::Whisper);
        assert_eq!(text, "whisper transcript");
        assert!(session.is_unavailable());
        assert!(!session.should_attempt());
    }

    #[test]
    fn an_unusable_apple_result_falls_back_and_latches_unavailable() {
        // runtimeSupported=false comes back as Ok but not usable.
        let session = AppleSpeechSession::new();
        let (engine, text) = transcribe_or_fall_back(
            &session,
            || Ok::<_, ()>(String::new()),
            |t| !t.is_empty(),
            || "whisper transcript".to_string(),
        );
        assert_eq!(engine, Engine::Whisper);
        assert_eq!(text, "whisper transcript");
        assert!(session.is_unavailable());
    }

    #[test]
    fn after_a_failure_apple_is_not_attempted_again_this_session() {
        let session = AppleSpeechSession::new();
        session.record_failure();
        let mut attempted = false;
        let (engine, text) = transcribe_or_fall_back(
            &session,
            || {
                attempted = true;
                Ok::<_, ()>("apple transcript".to_string())
            },
            |t| !t.is_empty(),
            || "whisper transcript".to_string(),
        );
        assert!(
            !attempted,
            "must not re-spawn the worker after a session failure"
        );
        assert_eq!(engine, Engine::Whisper);
        assert_eq!(text, "whisper transcript");
    }

    #[test]
    fn the_utterance_is_never_lost_even_when_both_paths_are_exercised() {
        // Whisper is the guaranteed transcript, so a caller always gets text.
        let session = AppleSpeechSession::new();
        let (_, text) = transcribe_or_fall_back(
            &session,
            || Err::<String, _>("crash"),
            |t| !t.is_empty(),
            || "whisper fallback".to_string(),
        );
        assert!(!text.is_empty());
    }
}
