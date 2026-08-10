//! Shared lifecycle control for Archive's sandboxed child processes.
//!
//! The desktop process exits immediately when its only window closes. Child
//! processes do not: on POSIX they are reparented and can keep source bytes and
//! derived text alive until their own deadline. Every Archive launcher uses
//! this registry so close and purge can cancel the build and kill every live
//! worker process group before the desktop process exits.

use std::collections::BTreeSet;
use std::process::{Child, Command, ExitStatus};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct WorkerRegistry {
    process_groups: BTreeSet<u32>,
    terminating: bool,
}

#[derive(Clone, Default)]
pub struct LiveWorkerProcesses {
    registry: Arc<Mutex<WorkerRegistry>>,
}

impl std::fmt::Debug for LiveWorkerProcesses {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("LiveWorkerProcesses([redacted process groups])")
    }
}

impl LiveWorkerProcesses {
    pub fn control(&self, cancelled: Arc<AtomicBool>) -> WorkerProcessControl {
        WorkerProcessControl {
            live: self.clone(),
            cancelled,
        }
    }

    /// Kill every registered process group and its leader.
    ///
    /// The direct-PID signal closes the small spawn race before the child has
    /// completed `setpgid`; the group signal also retires anything the worker
    /// itself launched. Reaping remains the launcher's job, after which its
    /// registration is removed.
    pub fn terminate_all(&self) {
        let mut registry = self
            .registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        registry.terminating = true;
        for pid in registry.process_groups.iter().copied() {
            terminate_process_group(pid);
        }
    }

    pub fn live_count(&self) -> usize {
        self.registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .process_groups
            .len()
    }
}

#[derive(Clone)]
pub struct WorkerProcessControl {
    live: LiveWorkerProcesses,
    cancelled: Arc<AtomicBool>,
}

impl std::fmt::Debug for WorkerProcessControl {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("WorkerProcessControl([shared cancellation and registry])")
    }
}

impl WorkerProcessControl {
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

struct WorkerRegistration {
    live: LiveWorkerProcesses,
    pid: u32,
}

impl std::fmt::Debug for WorkerRegistration {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("WorkerRegistration([redacted process group])")
    }
}

impl Drop for WorkerRegistration {
    fn drop(&mut self) {
        self.live
            .registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .process_groups
            .remove(&self.pid);
    }
}

/// A child that is killed and reaped on every early-return path.
pub struct RegisteredChild {
    child: Child,
    registration: Option<WorkerRegistration>,
    reaped: bool,
}

impl std::fmt::Debug for RegisteredChild {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("RegisteredChild([redacted child process])")
    }
}

impl RegisteredChild {
    pub fn spawn(
        command: &mut Command,
        control: Option<&WorkerProcessControl>,
    ) -> std::io::Result<Self> {
        let (child, registration) = if let Some(control) = control {
            // Hold the registry lock across spawn + registration. Purge either
            // sees this PID and kills it, or marks the registry terminating
            // before a later spawn can start. There is no post-snapshot gap in
            // which a worker can escape the purge fence.
            let mut registry = control
                .live
                .registry
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if registry.terminating || control.is_cancelled() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Interrupted,
                    "Archive worker launch cancelled",
                ));
            }
            let child = command.spawn()?;
            let pid = child.id();
            registry.process_groups.insert(pid);
            drop(registry);
            (
                child,
                Some(WorkerRegistration {
                    live: control.live.clone(),
                    pid,
                }),
            )
        } else {
            (command.spawn()?, None)
        };
        Ok(Self {
            child,
            registration,
            reaped: false,
        })
    }

    pub fn child_mut(&mut self) -> &mut Child {
        &mut self.child
    }

    pub fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        let status = self.child.try_wait()?;
        if status.is_some() {
            self.reaped = true;
            self.registration.take();
        }
        Ok(status)
    }

    pub fn terminate(&mut self) {
        if !self.reaped {
            terminate_process_group(self.child.id());
            let _ = self.child.kill();
            let _ = self.child.wait();
            self.reaped = true;
        }
        self.registration.take();
    }
}

impl Drop for RegisteredChild {
    fn drop(&mut self) {
        self.terminate();
    }
}

fn terminate_process_group(pid: u32) {
    #[cfg(unix)]
    unsafe {
        libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
        libc::kill(pid as libc::pid_t, libc::SIGKILL);
    }
    #[cfg(not(unix))]
    let _ = pid;
}
