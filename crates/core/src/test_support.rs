//! Test scaffolding shared by unit and integration tests.
//!
//! Not part of the public API and exempt from semver. It is compiled
//! unconditionally rather than behind `#[cfg(test)]` because integration tests
//! in `tests/` link against this crate as an external consumer and therefore
//! cannot see `#[cfg(test)]` items. That gap is what let an integration test
//! overwrite the developer's real `~/.minutes/search.db` (#588): the one test
//! that reached global state was the one with no way to isolate it.
//!
//! Several paths are resolved from the home directory rather than from
//! [`crate::config::Config`], notably `search.db` and `graph.db`. A test that
//! isolates only `output_dir` therefore still writes to real user state. Wrap
//! any such test in [`with_temp_home`].

use std::ffi::OsString;
use std::marker::PhantomData;
use std::path::Path;
use std::rc::Rc;
use std::sync::{Condvar, Mutex, OnceLock};

struct HomeEnvLock {
    state: Mutex<HomeEnvLockState>,
    wake: Condvar,
}

#[derive(Default)]
struct HomeEnvLockState {
    owner: Option<std::thread::ThreadId>,
    depth: usize,
}

/// A re-entrant guard for the process-global home-directory environment.
///
/// Test helpers commonly hold this guard while calling production path
/// resolution. Those calls must be able to join the same critical section
/// without deadlocking, while other test threads remain excluded.
pub struct HomeEnvGuard {
    lock: &'static HomeEnvLock,
    _not_send: PhantomData<Rc<()>>,
}

impl Drop for HomeEnvGuard {
    fn drop(&mut self) {
        let current = std::thread::current().id();
        let mut state = self
            .lock
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(state.owner.as_ref(), Some(&current));
        assert!(state.depth > 0);
        state.depth -= 1;
        if state.depth == 0 {
            state.owner = None;
            self.lock.wake.notify_one();
        }
    }
}

/// Serialize tests that mutate the `HOME` environment variable.
///
/// The environment is process-global, so concurrent tests would otherwise
/// observe each other's overrides. Poisoning is ignored: a panicking test
/// leaves the lock poisoned but the guard itself is still sound to hand out.
pub fn home_env_lock() -> HomeEnvGuard {
    static LOCK: OnceLock<HomeEnvLock> = OnceLock::new();
    let lock = LOCK.get_or_init(|| HomeEnvLock {
        state: Mutex::new(HomeEnvLockState::default()),
        wake: Condvar::new(),
    });
    let current = std::thread::current().id();
    let mut state = lock
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    while state.owner.as_ref().is_some_and(|owner| owner != &current) {
        state = lock
            .wake
            .wait(state)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
    }
    state.owner = Some(current);
    state.depth = state.depth.checked_add(1).expect("home env lock depth");
    drop(state);
    HomeEnvGuard {
        lock,
        _not_send: PhantomData,
    }
}

/// Sets `HOME` for as long as it is held, restoring the previous value on drop.
///
/// Take [`home_env_lock`] first. Prefer [`with_temp_home`], which does both.
pub struct HomeOverride {
    previous: Option<OsString>,
}

impl HomeOverride {
    pub fn set(path: &Path) -> Self {
        let previous = std::env::var_os("HOME");
        std::env::set_var("HOME", path);
        Self { previous }
    }
}

impl Drop for HomeOverride {
    fn drop(&mut self) {
        if let Some(previous) = &self.previous {
            std::env::set_var("HOME", previous);
        } else {
            std::env::remove_var("HOME");
        }
    }
}

/// Run `f` with `HOME` pointed at a fresh temporary directory.
///
/// Anything the body resolves from the home directory (`search.db`, `graph.db`,
/// `~/.minutes/...`) lands in that directory and is discarded afterwards, so the
/// developer's real state is never touched. The lock is held for the duration,
/// so tests using this run one at a time.
pub fn with_temp_home<T>(f: impl FnOnce(&Path) -> T) -> T {
    let _guard = home_env_lock();
    let temp = tempfile::tempdir().expect("temp home");
    let _home = HomeOverride::set(temp.path());
    f(temp.path())
}
