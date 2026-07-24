//! Delayed-publication child boundary for private audio on macOS.
//!
//! The configured transcription executable is launched suspended with a
//! seekable POSIX shared-memory descriptor that contains only zeroes. The live
//! process CodeDirectory hash is checked before any private byte is written.
//! Only the authenticated XPC parent can then fill the already-unlinked object
//! and request resume. A same-UID `SIGCONT` race can therefore cause denial,
//! but it cannot expose audio to an unattested executable.

use std::ffi::{c_void, CString};
use std::io::{Read, Seek, SeekFrom};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::ExitStatus;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use zeroize::{Zeroize, Zeroizing};

const AUDIO_DESCRIPTOR: RawFd = 30;
const FIRST_ATTACHMENT_DESCRIPTOR: RawFd = 31;
const MAX_AUDIO_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_ATTACHMENT_COUNT: usize = 4;
const MAX_ATTACHMENT_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const COPY_BUFFER_BYTES: usize = 64 * 1024;
const CS_OPS_CDHASH: libc::c_uint = 5;

unsafe extern "C" {
    fn csops(
        pid: libc::pid_t,
        operation: libc::c_uint,
        user_address: *mut c_void,
        user_size: libc::size_t,
    ) -> libc::c_int;
}

fn check_posix(status: libc::c_int) -> std::io::Result<()> {
    if status == 0 {
        Ok(())
    } else {
        Err(std::io::Error::from_raw_os_error(status))
    }
}

fn process_cdhash(pid: libc::pid_t) -> std::io::Result<[u8; 20]> {
    let mut cdhash = [0_u8; 20];
    let status = unsafe { csops(pid, CS_OPS_CDHASH, cdhash.as_mut_ptr().cast(), cdhash.len()) };
    if status == 0 && cdhash != [0_u8; 20] {
        Ok(cdhash)
    } else if status == 0 {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "audio child did not expose a CodeDirectory hash",
        ))
    } else {
        Err(std::io::Error::last_os_error())
    }
}

fn descriptor_above_stdio(descriptor: OwnedFd) -> std::io::Result<OwnedFd> {
    if descriptor.as_raw_fd() > libc::STDERR_FILENO {
        let flags = unsafe { libc::fcntl(descriptor.as_raw_fd(), libc::F_GETFD) };
        if flags < 0
            || unsafe {
                libc::fcntl(
                    descriptor.as_raw_fd(),
                    libc::F_SETFD,
                    flags | libc::FD_CLOEXEC,
                )
            } < 0
        {
            return Err(std::io::Error::last_os_error());
        }
        return Ok(descriptor);
    }
    let duplicate = unsafe { libc::fcntl(descriptor.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 3) };
    if duplicate < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(unsafe { OwnedFd::from_raw_fd(duplicate) })
    }
}

fn pipe_pair() -> std::io::Result<(OwnedFd, OwnedFd)> {
    let mut descriptors = [-1; 2];
    if unsafe { libc::pipe(descriptors.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let first = descriptor_above_stdio(unsafe { OwnedFd::from_raw_fd(descriptors[0]) })?;
    let second = descriptor_above_stdio(unsafe { OwnedFd::from_raw_fd(descriptors[1]) })?;
    Ok((first, second))
}

fn set_nonblocking(file: &std::fs::File) -> std::io::Result<()> {
    let descriptor = file.as_raw_fd();
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
    {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// A seekable kernel object with no filesystem entry and no live POSIX-shm
/// name. The name is removed before the first byte is written.
pub(crate) struct AnonymousShm {
    descriptor: OwnedFd,
    len: u64,
}

impl AnonymousShm {
    pub(crate) fn zeroed(len: u64) -> std::io::Result<Self> {
        if len > MAX_AUDIO_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "private audio exceeds the two-GiB transport limit",
            ));
        }
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random)
            .map_err(|error| std::io::Error::other(format!("audio shm nonce failed: {error}")))?;
        let name = format!(
            "/minutes-audio-{}-{}",
            unsafe { libc::getpid() },
            random
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        let name = CString::new(name).expect("hex POSIX shm name cannot contain NUL");
        let raw = unsafe {
            libc::shm_open(
                name.as_ptr(),
                libc::O_CREAT | libc::O_EXCL | libc::O_RDWR | libc::O_CLOEXEC,
                0o600,
            )
        };
        if raw < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let descriptor = unsafe { OwnedFd::from_raw_fd(raw) };
        if unsafe { libc::shm_unlink(name.as_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        if unsafe { libc::fchmod(descriptor.as_raw_fd(), 0o600) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        let length = libc::off_t::try_from(len).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "private audio length does not fit off_t",
            )
        })?;
        if unsafe { libc::ftruncate(descriptor.as_raw_fd(), length) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut metadata = unsafe { std::mem::zeroed::<libc::stat>() };
        if unsafe { libc::fstat(descriptor.as_raw_fd(), &mut metadata) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        if metadata.st_uid != unsafe { libc::geteuid() }
            || metadata.st_mode & libc::S_IFMT != libc::S_IFREG
            || metadata.st_mode & 0o777 != 0o600
            || metadata.st_size != length
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "private audio shared-memory identity was not owner-private and exact",
            ));
        }
        Ok(Self { descriptor, len })
    }

    pub(crate) fn snapshot_file(source: OwnedFd, total_budget: &mut u64) -> std::io::Result<Self> {
        let mut before = unsafe { std::mem::zeroed::<libc::stat>() };
        if unsafe { libc::fstat(source.as_raw_fd(), &mut before) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        if before.st_mode & libc::S_IFMT != libc::S_IFREG || before.st_size < 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "audio child attachment is not a regular file",
            ));
        }
        let len = u64::try_from(before.st_size).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "audio child attachment length is invalid",
            )
        })?;
        *total_budget = total_budget
            .checked_add(len)
            .filter(|total| *total <= MAX_ATTACHMENT_BYTES)
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "audio child attachments exceed the two-GiB snapshot limit",
                )
            })?;
        let snapshot = Self::zeroed(len)?;
        let mut offset = 0_u64;
        let mut buffer = Zeroizing::new(vec![0_u8; COPY_BUFFER_BYTES]);
        while offset < len {
            let take = usize::try_from((len - offset).min(buffer.len() as u64))
                .expect("bounded attachment chunk fits usize");
            let read = unsafe {
                libc::pread(
                    source.as_raw_fd(),
                    buffer.as_mut_ptr().cast(),
                    take,
                    libc::off_t::try_from(offset).unwrap_or(libc::off_t::MAX),
                )
            };
            if read <= 0 || read as usize != take {
                return Err(if read < 0 {
                    std::io::Error::last_os_error()
                } else {
                    std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "audio child attachment changed during snapshot",
                    )
                });
            }
            snapshot.write_at(offset, &buffer[..take])?;
            buffer[..take].zeroize();
            offset = offset
                .checked_add(take as u64)
                .ok_or_else(|| std::io::Error::other("attachment offset overflowed"))?;
        }
        let mut after = unsafe { std::mem::zeroed::<libc::stat>() };
        if unsafe { libc::fstat(source.as_raw_fd(), &mut after) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        if before.st_dev != after.st_dev
            || before.st_ino != after.st_ino
            || before.st_size != after.st_size
            || before.st_mtime != after.st_mtime
            || before.st_mtimespec.tv_nsec != after.st_mtimespec.tv_nsec
            || before.st_ctime != after.st_ctime
            || before.st_ctimespec.tv_nsec != after.st_ctimespec.tv_nsec
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "audio child attachment changed during exact snapshot",
            ));
        }
        Ok(snapshot)
    }

    pub(crate) fn write_at(&self, offset: u64, mut bytes: &[u8]) -> std::io::Result<()> {
        let end = offset
            .checked_add(bytes.len() as u64)
            .filter(|end| *end <= self.len)
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "private audio chunk exceeds declared length",
                )
            })?;
        let mut cursor = offset;
        while !bytes.is_empty() {
            let written = unsafe {
                libc::pwrite(
                    self.descriptor.as_raw_fd(),
                    bytes.as_ptr().cast(),
                    bytes.len(),
                    libc::off_t::try_from(cursor).unwrap_or(libc::off_t::MAX),
                )
            };
            if written <= 0 {
                return Err(if written < 0 {
                    std::io::Error::last_os_error()
                } else {
                    std::io::Error::new(
                        std::io::ErrorKind::WriteZero,
                        "private audio shared-memory write stopped",
                    )
                });
            }
            bytes = &bytes[written as usize..];
            cursor = cursor
                .checked_add(written as u64)
                .ok_or_else(|| std::io::Error::other("private audio offset overflowed"))?;
        }
        debug_assert_eq!(cursor, end);
        Ok(())
    }

    fn descriptor(&self) -> RawFd {
        self.descriptor.as_raw_fd()
    }
}

impl Drop for AnonymousShm {
    fn drop(&mut self) {
        let _ = unsafe { libc::ftruncate(self.descriptor.as_raw_fd(), 0) };
    }
}

struct SpawnFileActions(libc::posix_spawn_file_actions_t);

impl SpawnFileActions {
    fn new() -> std::io::Result<Self> {
        let mut actions = std::ptr::null_mut();
        check_posix(unsafe { libc::posix_spawn_file_actions_init(&mut actions) })?;
        Ok(Self(actions))
    }

    fn dup2(&mut self, source: RawFd, destination: RawFd) -> std::io::Result<()> {
        check_posix(unsafe {
            libc::posix_spawn_file_actions_adddup2(&mut self.0, source, destination)
        })
    }

    fn close(&mut self, descriptor: RawFd) -> std::io::Result<()> {
        check_posix(unsafe { libc::posix_spawn_file_actions_addclose(&mut self.0, descriptor) })
    }
}

impl Drop for SpawnFileActions {
    fn drop(&mut self) {
        unsafe {
            libc::posix_spawn_file_actions_destroy(&mut self.0);
        }
    }
}

struct SpawnAttributes(libc::posix_spawnattr_t);

impl SpawnAttributes {
    fn suspended_process_group() -> std::io::Result<Self> {
        let mut attributes = std::ptr::null_mut();
        check_posix(unsafe { libc::posix_spawnattr_init(&mut attributes) })?;
        let mut result = Self(attributes);
        check_posix(unsafe { libc::posix_spawnattr_setpgroup(&mut result.0, 0) })?;
        let flags = libc::POSIX_SPAWN_START_SUSPENDED
            | libc::POSIX_SPAWN_SETPGROUP
            | libc::POSIX_SPAWN_CLOEXEC_DEFAULT;
        check_posix(unsafe {
            libc::posix_spawnattr_setflags(
                &mut result.0,
                libc::c_short::try_from(flags).map_err(|_| {
                    std::io::Error::other("audio child spawn flags do not fit c_short")
                })?,
            )
        })?;
        Ok(result)
    }
}

impl Drop for SpawnAttributes {
    fn drop(&mut self) {
        unsafe {
            libc::posix_spawnattr_destroy(&mut self.0);
        }
    }
}

pub(crate) struct DelayedAudioChild {
    pid: libc::pid_t,
    stdout: Option<std::fs::File>,
    stderr: Option<std::fs::File>,
    audio: AnonymousShm,
    attachments: Vec<AnonymousShm>,
    expected_audio_len: u64,
    written_audio_len: u64,
    status: Option<ExitStatus>,
}

pub(crate) struct DelayedAudioOutput {
    pub(crate) status: ExitStatus,
    pub(crate) stdout: Zeroizing<Vec<u8>>,
    pub(crate) stderr: Zeroizing<Vec<u8>>,
    pub(crate) timed_out: bool,
}

impl DelayedAudioChild {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn spawn_attested(
        executable: &Path,
        expected_cdhash: &[u8; 20],
        arguments: &[String],
        environment: &[(String, String)],
        audio_len: u64,
        attachment_sources: Vec<OwnedFd>,
    ) -> std::io::Result<Self> {
        if attachment_sources.len() > MAX_ATTACHMENT_COUNT {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "audio child attachment count exceeds its fixed limit",
            ));
        }
        let audio = AnonymousShm::zeroed(audio_len)?;
        let mut attachment_budget = 0_u64;
        let mut attachments = Vec::with_capacity(attachment_sources.len());
        for source in attachment_sources {
            attachments.push(AnonymousShm::snapshot_file(source, &mut attachment_budget)?);
        }

        let executable = CString::new(executable.as_os_str().as_bytes()).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "audio child executable contains a NUL byte",
            )
        })?;
        let mut argument_storage = Vec::with_capacity(arguments.len() + 1);
        argument_storage.push(executable.clone());
        for argument in arguments {
            argument_storage.push(CString::new(argument.as_bytes()).map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "audio child argument contains a NUL byte",
                )
            })?);
        }
        let mut argv = argument_storage
            .iter()
            .map(|value| value.as_ptr().cast_mut())
            .collect::<Vec<_>>();
        argv.push(std::ptr::null_mut());

        let mut environment_storage = Vec::with_capacity(environment.len());
        for (name, value) in environment {
            if name.is_empty()
                || name.contains('=')
                || name.starts_with("DYLD_")
                || name.starts_with("LD_")
            {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "audio child environment key is not allowed",
                ));
            }
            environment_storage.push(CString::new(format!("{name}={value}")).map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "audio child environment contains a NUL byte",
                )
            })?);
        }
        let mut envp = environment_storage
            .iter()
            .map(|value| value.as_ptr().cast_mut())
            .collect::<Vec<_>>();
        envp.push(std::ptr::null_mut());

        let (parent_stdout, child_stdout) = pipe_pair()?;
        let (parent_stderr, child_stderr) = pipe_pair()?;
        let mut file_actions = SpawnFileActions::new()?;
        file_actions.dup2(child_stdout.as_raw_fd(), libc::STDOUT_FILENO)?;
        file_actions.dup2(child_stderr.as_raw_fd(), libc::STDERR_FILENO)?;
        file_actions.dup2(audio.descriptor(), AUDIO_DESCRIPTOR)?;
        for (index, attachment) in attachments.iter().enumerate() {
            file_actions.dup2(
                attachment.descriptor(),
                FIRST_ATTACHMENT_DESCRIPTOR + index as RawFd,
            )?;
        }
        for descriptor in [
            parent_stdout.as_raw_fd(),
            child_stdout.as_raw_fd(),
            parent_stderr.as_raw_fd(),
            child_stderr.as_raw_fd(),
        ] {
            file_actions.close(descriptor)?;
        }
        let attributes = SpawnAttributes::suspended_process_group()?;
        let mut pid = -1;
        let status = unsafe {
            libc::posix_spawn(
                &mut pid,
                executable.as_ptr(),
                &file_actions.0,
                &attributes.0,
                argv.as_ptr(),
                envp.as_ptr(),
            )
        };
        if status != 0 {
            return Err(std::io::Error::from_raw_os_error(status));
        }
        drop(child_stdout);
        drop(child_stderr);
        if pid <= 1 {
            return Err(std::io::Error::other(
                "audio child returned an invalid process identifier",
            ));
        }
        let mut child = Self {
            pid,
            stdout: Some(std::fs::File::from(parent_stdout)),
            stderr: Some(std::fs::File::from(parent_stderr)),
            audio,
            attachments,
            expected_audio_len: audio_len,
            written_audio_len: 0,
            status: None,
        };
        let group = unsafe { libc::getpgid(pid) };
        if group != pid {
            let observed = if group < 0 {
                format!("unavailable: {}", std::io::Error::last_os_error())
            } else {
                group.to_string()
            };
            let _ = child.retire_group_and_reap();
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("audio child process group was not isolated: {observed}"),
            ));
        }
        match process_cdhash(pid) {
            Ok(observed) if observed == *expected_cdhash => Ok(child),
            Ok(_) => {
                let _ = child.retire_group_and_reap();
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "audio child did not match the exact authorized executable",
                ))
            }
            Err(error) => {
                let _ = child.retire_group_and_reap();
                Err(error)
            }
        }
    }

    pub(crate) fn audio_descriptor_path() -> &'static str {
        "/dev/fd/30"
    }

    pub(crate) fn attachment_descriptor_path(index: usize) -> Option<String> {
        (index < MAX_ATTACHMENT_COUNT)
            .then(|| format!("/dev/fd/{}", FIRST_ATTACHMENT_DESCRIPTOR + index as RawFd))
    }

    pub(crate) fn append_audio(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        if bytes.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "private audio chunk is empty",
            ));
        }
        self.audio.write_at(self.written_audio_len, bytes)?;
        self.written_audio_len = self
            .written_audio_len
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| std::io::Error::other("private audio length overflowed"))?;
        Ok(())
    }

    pub(crate) fn finish_and_wait(
        mut self,
        deadline: Instant,
        max_stdout: usize,
        max_stderr: usize,
    ) -> std::io::Result<DelayedAudioOutput> {
        if self.written_audio_len != self.expected_audio_len {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "private audio stream length did not match its authenticated declaration",
            ));
        }
        let stdout = self
            .stdout
            .take()
            .ok_or_else(|| std::io::Error::other("audio child stdout was unavailable"))?;
        let stderr = self
            .stderr
            .take()
            .ok_or_else(|| std::io::Error::other("audio child stderr was unavailable"))?;
        let cancelled = Arc::new(AtomicBool::new(false));
        let stdout_cancelled = Arc::clone(&cancelled);
        let stderr_cancelled = Arc::clone(&cancelled);
        let stdout_reader =
            std::thread::spawn(move || read_bounded(stdout, max_stdout, &stdout_cancelled));
        let stderr_reader =
            std::thread::spawn(move || read_bounded(stderr, max_stderr, &stderr_cancelled));

        if unsafe { libc::kill(self.pid, libc::SIGCONT) } != 0 {
            cancelled.store(true, Ordering::Release);
            let error = std::io::Error::last_os_error();
            let _ = self.retire_group_and_reap();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err(error);
        }

        let mut timed_out = false;
        loop {
            let mut information = unsafe { std::mem::zeroed::<libc::siginfo_t>() };
            let wait = unsafe {
                libc::waitid(
                    libc::P_PID,
                    self.pid as libc::id_t,
                    &mut information,
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                )
            };
            if wait != 0 {
                cancelled.store(true, Ordering::Release);
                let error = std::io::Error::last_os_error();
                let _ = self.retire_group_and_reap();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(error);
            }
            if information.si_pid == self.pid {
                break;
            }
            if Instant::now() >= deadline {
                timed_out = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }

        if timed_out {
            cancelled.store(true, Ordering::Release);
        }
        self.retire_group_and_reap()?;
        cancelled.store(true, Ordering::Release);
        let stdout = stdout_reader
            .join()
            .map_err(|_| std::io::Error::other("audio child stdout reader panicked"))??;
        let stderr = stderr_reader
            .join()
            .map_err(|_| std::io::Error::other("audio child stderr reader panicked"))??;
        let status = self
            .status
            .ok_or_else(|| std::io::Error::other("audio child was not exactly reaped"))?;
        Ok(DelayedAudioOutput {
            status,
            stdout,
            stderr,
            timed_out,
        })
    }

    fn retire_group_and_reap(&mut self) -> std::io::Result<()> {
        if self.status.is_some() {
            return Ok(());
        }
        // The direct child remains unreaped while the process-group signal is
        // sent, so its numeric PID cannot be recycled as an unrelated PGID.
        if unsafe { libc::kill(-self.pid, libc::SIGKILL) } != 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(error);
            }
        }
        let mut raw_status = 0;
        loop {
            let waited = unsafe { libc::waitpid(self.pid, &mut raw_status, 0) };
            if waited == self.pid {
                self.status = Some(ExitStatus::from_raw(raw_status));
                break;
            }
            if waited < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error);
            }
        }
        // Any descendant that briefly survived the leader is still in the
        // same group. Do not report settlement until the kernel says the group
        // no longer exists.
        let settle_deadline = Instant::now() + Duration::from_millis(250);
        loop {
            if unsafe { libc::kill(-self.pid, 0) } != 0 {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() == Some(libc::ESRCH) {
                    return Ok(());
                }
                return Err(error);
            }
            let _ = unsafe { libc::kill(-self.pid, libc::SIGKILL) };
            if Instant::now() >= settle_deadline {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "audio child process group did not retire",
                ));
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

impl Drop for DelayedAudioChild {
    fn drop(&mut self) {
        let _ = self.retire_group_and_reap();
    }
}

fn read_bounded(
    mut file: std::fs::File,
    max_bytes: usize,
    cancelled: &AtomicBool,
) -> std::io::Result<Zeroizing<Vec<u8>>> {
    set_nonblocking(&file)?;
    let mut retained = Zeroizing::new(Vec::with_capacity(max_bytes.min(64 * 1024)));
    let mut buffer = Zeroizing::new(vec![0_u8; 16 * 1024]);
    loop {
        match file.read(&mut buffer) {
            Ok(0) => return Ok(retained),
            Ok(read) => {
                if retained
                    .len()
                    .checked_add(read)
                    .is_none_or(|total| total > max_bytes)
                {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::OutOfMemory,
                        "audio child output exceeded its byte budget",
                    ));
                }
                retained.extend_from_slice(&buffer[..read]);
                buffer[..read].zeroize();
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if cancelled.load(Ordering::Acquire) {
                    return Ok(retained);
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
}

pub(crate) fn private_reader_len(reader: &mut (impl Read + Seek)) -> std::io::Result<u64> {
    let current = reader.stream_position()?;
    let len = reader.seek(SeekFrom::End(0))?;
    reader.seek(SeekFrom::Start(current))?;
    Ok(len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_paths_are_fixed_and_bounded() {
        assert_eq!(DelayedAudioChild::audio_descriptor_path(), "/dev/fd/30");
        assert_eq!(
            DelayedAudioChild::attachment_descriptor_path(0).as_deref(),
            Some("/dev/fd/31")
        );
        assert_eq!(
            DelayedAudioChild::attachment_descriptor_path(3).as_deref(),
            Some("/dev/fd/34")
        );
        assert!(DelayedAudioChild::attachment_descriptor_path(4).is_none());
    }

    #[test]
    fn anonymous_shm_is_seekable_unlinked_owner_private_and_exact() {
        let object = AnonymousShm::zeroed(8).unwrap();
        object.write_at(0, b"private").unwrap();
        let mut file = unsafe { std::fs::File::from_raw_fd(libc::dup(object.descriptor())) };
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"private\0");
    }

    #[test]
    fn anonymous_shm_refuses_extension() {
        let object = AnonymousShm::zeroed(4).unwrap();
        assert!(object.write_at(1, b"four").is_err());
    }

    #[test]
    fn private_reader_len_restores_position() {
        let mut reader = std::io::Cursor::new(b"synthetic".to_vec());
        reader.set_position(3);
        assert_eq!(private_reader_len(&mut reader).unwrap(), 9);
        assert_eq!(reader.position(), 3);
    }
}
