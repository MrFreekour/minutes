//! Authenticated macOS XPC transport for pathname-only audio engines.
//!
//! The parent pins the exact embedded service CodeDirectory hash before a
//! content-free handshake. The service pins the exact Minutes parent identity.
//! Engine metadata and attachment descriptors cross only after that handshake;
//! private audio crosses only after the service has launched a suspended child
//! and attested the live child's exact CodeDirectory hash.

use block2::{Block, RcBlock};
use serde::{Deserialize, Serialize};
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::io::Read;
use std::os::fd::{FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, MutexGuard, OnceLock, TryLockError};
use std::time::{Duration, Instant};
use zeroize::{Zeroize, Zeroizing};

type XpcObject = *mut c_void;
type PeerRequirementFn = unsafe extern "C" fn(XpcObject, *const c_char) -> c_int;

const XPC_SERVICE_NAME: &[u8] = b"com.useminutes.audio-worker\0";
const COMMAND_KEY: &[u8] = b"command\0";
const SEQUENCE_KEY: &[u8] = b"sequence\0";
const DATA_KEY: &[u8] = b"data\0";
const STDOUT_KEY: &[u8] = b"stdout\0";
const STDERR_KEY: &[u8] = b"stderr\0";
const EXIT_CODE_KEY: &[u8] = b"exit_code\0";
const EXIT_SIGNAL_KEY: &[u8] = b"exit_signal\0";
const TIMED_OUT_KEY: &[u8] = b"timed_out\0";
const OK_KEY: &[u8] = b"ok\0";
const READY_KEY: &[u8] = b"ready\0";
const BUSY_KEY: &[u8] = b"busy\0";
const TERMINAL_KEY: &[u8] = b"terminal\0";
const SERVICE_NONCE_KEY: &[u8] = b"service_nonce\0";
const COMMAND_BEGIN: &[u8] = b"begin\0";
const COMMAND_PREPARE: &[u8] = b"prepare\0";
const COMMAND_CHUNK: &[u8] = b"chunk\0";
const COMMAND_FINISH: &[u8] = b"finish\0";
const COMMAND_ABORT: &[u8] = b"abort\0";
const XPC_CHUNK_BYTES: usize = 60 * 1024;
const MAX_METADATA_BYTES: usize = 512 * 1024;
const MAX_ARGUMENTS: usize = 512;
const MAX_ARGUMENT_BYTES: usize = 256 * 1024;
const MAX_ENVIRONMENT_ENTRIES: usize = 32;
const MAX_ENVIRONMENT_BYTES: usize = 64 * 1024;
const MAX_STDOUT_BYTES: usize = 4 * 1024 * 1024;
const MAX_STDERR_BYTES: usize = 1024 * 1024;
const MAX_ATTACHMENTS: usize = 4;
const MAX_WALL_CLOCK_MS: u64 = 30 * 60 * 1000;
const SERVICE_PROCESS_WALL_CLOCK: Duration = Duration::from_secs(31 * 60);
const SERVICE_ADDRESS_SPACE_GROWTH: u64 = 8 * 1024 * 1024 * 1024;
const XPC_ERROR_CONNECTION_INTERRUPTED_SYMBOL: &[u8] = b"_xpc_error_connection_interrupted\0";
const XPC_ERROR_CONNECTION_INVALID_SYMBOL: &[u8] = b"_xpc_error_connection_invalid\0";
static XPC_SETTLEMENT_FAILED: AtomicBool = AtomicBool::new(false);
static XPC_PARENT_REQUEST_LOCK: Mutex<()> = Mutex::new(());
static XPC_PARENT_CALLBACK_QUEUE: OnceLock<usize> = OnceLock::new();

unsafe extern "C" {
    fn minutes_current_process_is_trusted_distribution() -> c_int;
    fn minutes_validate_graph_authority_bundle(
        authority_bundle_path: *const c_char,
        current_executable_path: *const c_char,
        running_parent_cdhash: *const u8,
        running_parent_cdhash_len: isize,
    ) -> c_int;
    fn minutes_static_code_cdhash(
        executable_path: *const c_char,
        output: *mut u8,
        output_len: isize,
    ) -> c_int;
    fn csops(
        pid: libc::pid_t,
        operation: libc::c_uint,
        user_address: *mut c_void,
        user_size: libc::size_t,
    ) -> c_int;
    fn SecRandomCopyBytes(random: *const c_void, count: usize, bytes: *mut u8) -> c_int;

    fn xpc_connection_create(name: *const c_char, target_queue: *mut c_void) -> XpcObject;
    fn dispatch_queue_create(label: *const c_char, attr: *const c_void) -> *mut c_void;
    fn xpc_connection_set_event_handler(connection: XpcObject, handler: &Block<dyn Fn(XpcObject)>);
    fn xpc_connection_send_message_with_reply(
        connection: XpcObject,
        message: XpcObject,
        reply_queue: *mut c_void,
        handler: &Block<dyn Fn(XpcObject)>,
    );
    fn xpc_connection_send_message(connection: XpcObject, message: XpcObject);
    fn xpc_connection_send_barrier(connection: XpcObject, barrier: &Block<dyn Fn()>);
    fn xpc_connection_resume(connection: XpcObject);
    fn xpc_connection_cancel(connection: XpcObject);
    fn xpc_dictionary_create(
        keys: *const *const c_char,
        values: *const XpcObject,
        count: usize,
    ) -> XpcObject;
    fn xpc_dictionary_create_reply(original: XpcObject) -> XpcObject;
    fn xpc_dictionary_set_string(dictionary: XpcObject, key: *const c_char, value: *const c_char);
    fn xpc_dictionary_get_string(dictionary: XpcObject, key: *const c_char) -> *const c_char;
    fn xpc_dictionary_set_uint64(dictionary: XpcObject, key: *const c_char, value: u64);
    fn xpc_dictionary_get_uint64(dictionary: XpcObject, key: *const c_char) -> u64;
    fn xpc_dictionary_set_int64(dictionary: XpcObject, key: *const c_char, value: i64);
    fn xpc_dictionary_get_int64(dictionary: XpcObject, key: *const c_char) -> i64;
    fn xpc_dictionary_set_bool(dictionary: XpcObject, key: *const c_char, value: bool);
    fn xpc_dictionary_get_bool(dictionary: XpcObject, key: *const c_char) -> bool;
    fn xpc_dictionary_set_data(
        dictionary: XpcObject,
        key: *const c_char,
        bytes: *const c_void,
        length: usize,
    );
    fn xpc_dictionary_get_data(
        dictionary: XpcObject,
        key: *const c_char,
        length: *mut usize,
    ) -> *const c_void;
    fn xpc_dictionary_set_fd(dictionary: XpcObject, key: *const c_char, descriptor: c_int);
    fn xpc_dictionary_dup_fd(dictionary: XpcObject, key: *const c_char) -> c_int;
    fn xpc_get_type(object: XpcObject) -> *const c_void;
    fn xpc_type_get_name(kind: *const c_void) -> *const c_char;
    fn xpc_retain(object: XpcObject) -> XpcObject;
    fn xpc_release(object: XpcObject);
    fn xpc_main(handler: &Block<dyn Fn(XpcObject)>) -> !;
}

const CS_OPS_CDHASH: libc::c_uint = 5;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AudioChildMetadata {
    pub(crate) schema_version: u32,
    pub(crate) executable: PathBuf,
    pub(crate) executable_cdhash: [u8; 20],
    pub(crate) arguments: Vec<String>,
    pub(crate) environment: Vec<(String, String)>,
    pub(crate) audio_len: u64,
    pub(crate) attachment_count: usize,
    pub(crate) max_stdout: usize,
    pub(crate) max_stderr: usize,
    pub(crate) wall_clock_ms: u64,
}

pub(crate) struct AudioChildRequest {
    pub(crate) metadata: AudioChildMetadata,
    pub(crate) attachments: Vec<std::fs::File>,
}

pub(crate) struct AudioChildOutput {
    pub(crate) exit_code: Option<i32>,
    pub(crate) exit_signal: Option<i32>,
    pub(crate) stdout: Zeroizing<Vec<u8>>,
    pub(crate) stderr: Zeroizing<Vec<u8>>,
    pub(crate) timed_out: bool,
}

impl AudioChildMetadata {
    fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1
            || !self.executable.is_absolute()
            || self.executable.as_os_str().is_empty()
            || self.executable_cdhash == [0_u8; 20]
            || self.arguments.len() > MAX_ARGUMENTS
            || self.environment.len() > MAX_ENVIRONMENT_ENTRIES
            || self.attachment_count > MAX_ATTACHMENTS
            || self.max_stdout == 0
            || self.max_stdout > MAX_STDOUT_BYTES
            || self.max_stderr == 0
            || self.max_stderr > MAX_STDERR_BYTES
            || self.wall_clock_ms == 0
            || self.wall_clock_ms > MAX_WALL_CLOCK_MS
        {
            return Err("private audio child metadata was invalid".into());
        }
        let argument_bytes = self
            .arguments
            .iter()
            .try_fold(0_usize, |total, value| total.checked_add(value.len()));
        let environment_bytes = self
            .environment
            .iter()
            .try_fold(0_usize, |total, (name, value)| {
                total.checked_add(name.len())?.checked_add(value.len())
            });
        if argument_bytes.is_none_or(|bytes| bytes > MAX_ARGUMENT_BYTES)
            || environment_bytes.is_none_or(|bytes| bytes > MAX_ENVIRONMENT_BYTES)
        {
            return Err("private audio child metadata exceeded its byte budget".into());
        }
        let audio_path = crate::macos_audio_child::DelayedAudioChild::audio_descriptor_path();
        if self
            .arguments
            .iter()
            .filter(|argument| argument.as_str() == audio_path)
            .count()
            != 1
        {
            return Err("private audio child must receive exactly one audio descriptor".into());
        }
        for index in 0..self.attachment_count {
            let path =
                crate::macos_audio_child::DelayedAudioChild::attachment_descriptor_path(index)
                    .ok_or_else(|| "private audio attachment index was invalid".to_string())?;
            if self
                .arguments
                .iter()
                .filter(|argument| argument.as_str() == path)
                .count()
                != 1
            {
                return Err(
                    "private audio child must receive every attachment exactly once".into(),
                );
            }
        }
        Ok(())
    }
}

fn parent_callback_queue() -> Result<*mut c_void, String> {
    let queue = *XPC_PARENT_CALLBACK_QUEUE.get_or_init(|| {
        let label = b"com.useminutes.audio-worker.parent\0";
        unsafe { dispatch_queue_create(label.as_ptr().cast(), std::ptr::null()) as usize }
    }) as *mut c_void;
    if queue.is_null() {
        Err("private audio XPC callback queue could not be created".into())
    } else {
        Ok(queue)
    }
}

fn ensure_transport_available(poisoned: &AtomicBool) -> Result<(), String> {
    if poisoned.load(Ordering::Acquire) {
        Err(
            "private audio XPC transport requires an application restart after an unconfirmed service exit"
                .into(),
        )
    } else {
        Ok(())
    }
}

fn lock_parent_request<'a>(
    lock: &'a Mutex<()>,
    poisoned: &AtomicBool,
    deadline: Instant,
) -> Result<MutexGuard<'a, ()>, String> {
    loop {
        match lock.try_lock() {
            Ok(guard) => {
                ensure_transport_available(poisoned)?;
                return Ok(guard);
            }
            Err(TryLockError::Poisoned(_)) => {
                return Err("private audio XPC parent request lock was poisoned".into());
            }
            Err(TryLockError::WouldBlock) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(
                        "private audio XPC request admission exceeded its wall-clock budget".into(),
                    );
                }
                std::thread::sleep(remaining.min(Duration::from_millis(2)));
            }
        }
    }
}

pub(crate) fn current_process_is_trusted_distribution() -> bool {
    unsafe { minutes_current_process_is_trusted_distribution() == 1 }
}

fn current_process_cdhash() -> std::io::Result<[u8; 20]> {
    let mut cdhash = [0_u8; 20];
    let status = unsafe {
        csops(
            libc::getpid(),
            CS_OPS_CDHASH,
            cdhash.as_mut_ptr().cast(),
            cdhash.len(),
        )
    };
    if status == 0 && cdhash != [0_u8; 20] {
        Ok(cdhash)
    } else if status == 0 {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "current process did not expose a CodeDirectory hash",
        ))
    } else {
        Err(std::io::Error::last_os_error())
    }
}

pub(crate) fn static_code_cdhash(path: &Path) -> Result<[u8; 20], String> {
    let path = cstring_path(path, "audio child executable path")?;
    let mut cdhash = [0_u8; 20];
    let status = unsafe {
        minutes_static_code_cdhash(path.as_ptr(), cdhash.as_mut_ptr(), cdhash.len() as isize)
    };
    if status == 0 && cdhash != [0_u8; 20] {
        Ok(cdhash)
    } else {
        Err("audio child executable was not valid signed code".into())
    }
}

pub(crate) fn peer_requirement_api_available() -> bool {
    load_peer_requirement().is_some()
}

fn load_peer_requirement() -> Option<PeerRequirementFn> {
    let symbol = b"xpc_connection_set_peer_code_signing_requirement\0";
    let address = unsafe { libc::dlsym(libc::RTLD_DEFAULT, symbol.as_ptr().cast()) };
    (!address.is_null()).then(|| unsafe { std::mem::transmute(address) })
}

fn set_peer_requirement(connection: XpcObject, requirement: &CStr) -> Result<(), String> {
    let set_requirement = load_peer_requirement()
        .ok_or_else(|| "authenticated private-audio XPC is unavailable".to_string())?;
    let status = unsafe { set_requirement(connection, requirement.as_ptr()) };
    if status == 0 {
        Ok(())
    } else {
        Err("the private-audio XPC code-signing requirement was rejected".into())
    }
}

fn xpc_type_is(object: XpcObject, expected: &str) -> bool {
    if object.is_null() {
        return false;
    }
    let kind = unsafe { xpc_get_type(object) };
    if kind.is_null() {
        return false;
    }
    let name = unsafe { xpc_type_get_name(kind) };
    !name.is_null() && unsafe { CStr::from_ptr(name) }.to_bytes() == expected.as_bytes()
}

fn xpc_is_connection_end(event: XpcObject) -> bool {
    if event.is_null() {
        return false;
    }
    [
        XPC_ERROR_CONNECTION_INTERRUPTED_SYMBOL,
        XPC_ERROR_CONNECTION_INVALID_SYMBOL,
    ]
    .into_iter()
    .any(|symbol| {
        let object = unsafe { libc::dlsym(libc::RTLD_DEFAULT, symbol.as_ptr().cast()) };
        !object.is_null() && event == object
    })
}

fn cstring_path(path: &Path, description: &str) -> Result<CString, String> {
    use std::os::unix::ffi::OsStrExt;
    CString::new(path.as_os_str().as_bytes())
        .map_err(|_| format!("{description} contained a NUL byte"))
}

fn validate_authority_bundle(authority_bundle: &Path) -> Result<(), String> {
    let authority_bundle = cstring_path(authority_bundle, "audio worker authority bundle path")?;
    let current_executable =
        std::env::current_exe().map_err(|_| "current executable path was unavailable")?;
    let current_executable = cstring_path(&current_executable, "current executable path")?;
    let running_parent_cdhash =
        current_process_cdhash().map_err(|_| "current executable identity was unavailable")?;
    let status = unsafe {
        minutes_validate_graph_authority_bundle(
            authority_bundle.as_ptr(),
            current_executable.as_ptr(),
            running_parent_cdhash.as_ptr(),
            running_parent_cdhash.len() as isize,
        )
    };
    if status == 0 {
        Ok(())
    } else {
        Err("the application bundle did not seal the audio worker authority".into())
    }
}

struct OwnedXpc(XpcObject);

impl OwnedXpc {
    fn dictionary() -> Result<Self, String> {
        let object = unsafe { xpc_dictionary_create(std::ptr::null(), std::ptr::null(), 0) };
        if object.is_null() {
            Err("private audio XPC could not allocate a request".into())
        } else {
            Ok(Self(object))
        }
    }
}

impl Drop for OwnedXpc {
    fn drop(&mut self) {
        unsafe { xpc_release(self.0) };
    }
}

struct Connection {
    object: XpcObject,
    invalidated: mpsc::Receiver<()>,
    service_nonce: Mutex<Option<[u8; 16]>>,
    transport_failed: Arc<AtomicBool>,
    terminal_acknowledged: AtomicBool,
}

impl Connection {
    fn wait_for_service_exit(&self, deadline: Instant) -> Result<(), String> {
        if !self.terminal_acknowledged.load(Ordering::Acquire) {
            return Err("private audio XPC terminal settlement was not acknowledged".into());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("private audio XPC service exit exceeded its wall-clock budget".into());
        }
        self.invalidated.recv_timeout(remaining).map_err(|_| {
            "private audio XPC service exit exceeded its wall-clock budget".to_string()
        })
    }

    fn send_with_reply(&self, message: XpcObject, deadline: Instant) -> Result<OwnedXpc, String> {
        if self.transport_failed.load(Ordering::Acquire) {
            return Err("private audio XPC transport ended before the next request".into());
        }
        if let Some(nonce) = *self
            .service_nonce
            .lock()
            .map_err(|_| "private audio XPC service nonce lock was poisoned")?
        {
            unsafe {
                xpc_dictionary_set_data(
                    message,
                    SERVICE_NONCE_KEY.as_ptr().cast(),
                    nonce.as_ptr().cast(),
                    nonce.len(),
                );
            }
        }
        let reply = match send_with_reply(self.object, parent_callback_queue()?, message, deadline)
        {
            Ok(reply) => reply,
            Err(error) => {
                self.transport_failed.store(true, Ordering::Release);
                return Err(error);
            }
        };
        let service_nonce = match service_nonce_from_reply(reply.0) {
            Ok(nonce) => nonce,
            Err(error) => {
                self.transport_failed.store(true, Ordering::Release);
                return Err(error);
            }
        };
        let mut expected_nonce = self
            .service_nonce
            .lock()
            .map_err(|_| "private audio XPC service nonce lock was poisoned")?;
        if let Err(error) = bind_service_nonce(&mut expected_nonce, service_nonce) {
            self.transport_failed.store(true, Ordering::Release);
            return Err(error);
        }
        let terminal = unsafe { xpc_dictionary_get_bool(reply.0, TERMINAL_KEY.as_ptr().cast()) };
        if terminal {
            self.terminal_acknowledged.store(true, Ordering::Release);
        }
        if self.transport_failed.load(Ordering::Acquire) && !terminal {
            return Err("private audio XPC transport ended before a terminal reply".into());
        }
        Ok(reply)
    }

    fn settle(&self, abort: bool, deadline: Instant) -> Result<(), String> {
        if self.transport_failed.load(Ordering::Acquire)
            && !self.terminal_acknowledged.load(Ordering::Acquire)
        {
            return Err(
                "private audio XPC transport failed before terminal acknowledgement".into(),
            );
        }
        if abort && !self.terminal_acknowledged.load(Ordering::Acquire) {
            let message = OwnedXpc::dictionary()
                .map_err(|_| "private audio XPC terminal abort could not be created")?;
            set_command(message.0, COMMAND_ABORT);
            let reply = self
                .send_with_reply(message.0, deadline)
                .map_err(|_| "private audio XPC terminal abort was not acknowledged")?;
            if !unsafe { xpc_dictionary_get_bool(reply.0, OK_KEY.as_ptr().cast()) } {
                return Err("private audio XPC terminal abort was rejected".into());
            }
        }
        if !self.terminal_acknowledged.load(Ordering::Acquire) {
            return Err("private audio XPC terminal settlement was not acknowledged".into());
        }
        self.wait_for_service_exit(deadline)
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        unsafe {
            xpc_connection_cancel(self.object);
            xpc_release(self.object);
        }
    }
}

fn set_command(message: XpcObject, command: &[u8]) {
    unsafe {
        xpc_dictionary_set_string(
            message,
            COMMAND_KEY.as_ptr().cast(),
            command.as_ptr().cast(),
        );
    }
}

fn service_nonce_from_reply(reply: XpcObject) -> Result<[u8; 16], String> {
    let mut length = 0_usize;
    let data =
        unsafe { xpc_dictionary_get_data(reply, SERVICE_NONCE_KEY.as_ptr().cast(), &mut length) };
    if data.is_null() || length != 16 {
        return Err("private audio XPC reply lacked its exact process nonce".into());
    }
    let mut nonce = [0_u8; 16];
    nonce.copy_from_slice(unsafe { std::slice::from_raw_parts(data.cast::<u8>(), length) });
    Ok(nonce)
}

fn bind_service_nonce(expected: &mut Option<[u8; 16]>, observed: [u8; 16]) -> Result<(), String> {
    match *expected {
        None => *expected = Some(observed),
        Some(current) if current == observed => {}
        Some(_) => return Err("private audio XPC service generation changed mid-request".into()),
    }
    Ok(())
}

fn send_with_reply(
    connection: XpcObject,
    reply_queue: *mut c_void,
    message: XpcObject,
    deadline: Instant,
) -> Result<OwnedXpc, String> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err("private audio XPC operation exceeded its wall-clock budget".into());
    }
    let (sender, receiver) = mpsc::sync_channel(1);
    let handler = RcBlock::new(move |reply: XpcObject| {
        let retained = unsafe { xpc_retain(reply) };
        if sender.send(retained as usize).is_err() {
            unsafe { xpc_release(retained) };
        }
    });
    unsafe {
        xpc_connection_send_message_with_reply(connection, message, reply_queue, &handler);
    }
    let reply = receiver
        .recv_timeout(remaining)
        .map_err(|_| "private audio XPC operation exceeded its wall-clock budget".to_string())?;
    let reply = reply as XpcObject;
    if !xpc_type_is(reply, "dictionary") {
        unsafe { xpc_release(reply) };
        return Err("private audio XPC peer was unavailable or unauthenticated".into());
    }
    Ok(OwnedXpc(reply))
}

fn open_authenticated_connection(
    exact_cdhash: &[u8; 20],
    trusted_distribution: bool,
    deadline: Instant,
) -> Result<Connection, String> {
    let callback_queue = parent_callback_queue()?;
    let connection =
        unsafe { xpc_connection_create(XPC_SERVICE_NAME.as_ptr().cast(), callback_queue) };
    if connection.is_null() {
        return Err("private audio XPC service could not be created".into());
    }
    let (invalidated_sender, invalidated) = mpsc::channel();
    let transport_failed = Arc::new(AtomicBool::new(false));
    let connection = Connection {
        object: connection,
        invalidated,
        service_nonce: Mutex::new(None),
        transport_failed: Arc::clone(&transport_failed),
        terminal_acknowledged: AtomicBool::new(false),
    };
    let encoded = exact_cdhash
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let mut requirement =
        format!("identifier \"com.useminutes.audio-worker\" and cdhash H\"{encoded}\"");
    if trusted_distribution {
        requirement.push_str(
            " and anchor apple generic and certificate leaf[subject.OU] = \"63TMLKT8HN\"",
        );
    }
    let requirement =
        CString::new(requirement).map_err(|_| "private audio XPC requirement was malformed")?;
    set_peer_requirement(connection.object, &requirement)?;
    let events = RcBlock::new(move |event: XpcObject| {
        if xpc_is_connection_end(event) {
            transport_failed.store(true, Ordering::Release);
            let _ = invalidated_sender.send(());
        }
    });
    unsafe {
        xpc_connection_set_event_handler(connection.object, &events);
        xpc_connection_resume(connection.object);
    }

    let begin_result = (|| {
        let begin = OwnedXpc::dictionary()?;
        set_command(begin.0, COMMAND_BEGIN);
        let reply = connection.send_with_reply(begin.0, deadline)?;
        if unsafe { xpc_dictionary_get_bool(reply.0, BUSY_KEY.as_ptr().cast()) } {
            return Ok(false);
        }
        if !unsafe { xpc_dictionary_get_bool(reply.0, OK_KEY.as_ptr().cast()) } {
            return Err("private audio XPC rejected its content-free handshake".into());
        }
        Ok(true)
    })();
    match begin_result {
        Ok(true) => Ok(connection),
        Ok(false) => Err("private audio XPC service is busy".into()),
        Err(error) if connection.terminal_acknowledged.load(Ordering::Acquire) => {
            match connection.settle(false, deadline) {
                Ok(()) => Err(error),
                Err(settlement) => {
                    XPC_SETTLEMENT_FAILED.store(true, Ordering::Release);
                    Err(format!("{error}; {settlement}"))
                }
            }
        }
        Err(error) => {
            XPC_SETTLEMENT_FAILED.store(true, Ordering::Release);
            Err(format!(
                "{error}; private audio XPC handshake had no terminal acknowledgement"
            ))
        }
    }
}

pub(crate) fn run(
    authority_bundle: &Path,
    exact_worker_cdhash: &[u8; 20],
    trusted_distribution: bool,
    request: AudioChildRequest,
    mut audio: impl Read,
) -> Result<AudioChildOutput, String> {
    request.metadata.validate()?;
    if request.attachments.len() != request.metadata.attachment_count {
        return Err("private audio XPC attachment count did not match metadata".into());
    }
    ensure_transport_available(&XPC_SETTLEMENT_FAILED)?;
    let wall_clock = Duration::from_millis(request.metadata.wall_clock_ms)
        .checked_add(Duration::from_secs(5))
        .ok_or_else(|| "private audio XPC deadline overflowed".to_string())?;
    let deadline = Instant::now() + wall_clock;
    let _request_guard =
        lock_parent_request(&XPC_PARENT_REQUEST_LOCK, &XPC_SETTLEMENT_FAILED, deadline)?;
    validate_authority_bundle(authority_bundle)?;
    let connection =
        open_authenticated_connection(exact_worker_cdhash, trusted_distribution, deadline)?;

    let outcome = (|| {
        let metadata = serde_json::to_vec(&request.metadata)
            .map_err(|_| "audio metadata serialization failed")?;
        if metadata.len() > MAX_METADATA_BYTES {
            return Err("private audio XPC metadata exceeded its byte budget".into());
        }
        let prepare = OwnedXpc::dictionary()?;
        set_command(prepare.0, COMMAND_PREPARE);
        unsafe {
            xpc_dictionary_set_data(
                prepare.0,
                DATA_KEY.as_ptr().cast(),
                metadata.as_ptr().cast(),
                metadata.len(),
            );
        }
        for (index, attachment) in request.attachments.iter().enumerate() {
            let key = CString::new(format!("attachment_{index}"))
                .expect("numeric attachment key cannot contain NUL");
            unsafe {
                xpc_dictionary_set_fd(prepare.0, key.as_ptr(), attachment.as_raw_fd());
            }
        }
        let reply = connection.send_with_reply(prepare.0, deadline)?;
        if !unsafe { xpc_dictionary_get_bool(reply.0, OK_KEY.as_ptr().cast()) }
            || !unsafe { xpc_dictionary_get_bool(reply.0, READY_KEY.as_ptr().cast()) }
        {
            return Err(
                "private audio XPC child was not exactly attested before publication".into(),
            );
        }

        let mut sequence = 0_u64;
        let mut total = 0_u64;
        let mut buffer = Zeroizing::new(vec![0_u8; XPC_CHUNK_BYTES]);
        loop {
            let read = audio
                .read(&mut buffer)
                .map_err(|_| "private audio XPC input could not be read".to_string())?;
            if read == 0 {
                break;
            }
            total = total
                .checked_add(read as u64)
                .filter(|total| *total <= request.metadata.audio_len)
                .ok_or_else(|| {
                    "private audio XPC input exceeded its declared length".to_string()
                })?;
            let chunk = OwnedXpc::dictionary()?;
            set_command(chunk.0, COMMAND_CHUNK);
            unsafe {
                xpc_dictionary_set_uint64(chunk.0, SEQUENCE_KEY.as_ptr().cast(), sequence);
                xpc_dictionary_set_data(
                    chunk.0,
                    DATA_KEY.as_ptr().cast(),
                    buffer.as_ptr().cast(),
                    read,
                );
            }
            let reply = connection.send_with_reply(chunk.0, deadline)?;
            buffer[..read].zeroize();
            if !unsafe { xpc_dictionary_get_bool(reply.0, OK_KEY.as_ptr().cast()) } {
                return Err("private audio XPC service rejected an audio chunk".into());
            }
            sequence = sequence
                .checked_add(1)
                .ok_or_else(|| "private audio XPC sequence overflowed".to_string())?;
        }
        if total != request.metadata.audio_len {
            return Err("private audio XPC input was truncated".into());
        }

        let finish = OwnedXpc::dictionary()?;
        set_command(finish.0, COMMAND_FINISH);
        unsafe {
            xpc_dictionary_set_uint64(finish.0, SEQUENCE_KEY.as_ptr().cast(), sequence);
        }
        let reply = connection.send_with_reply(finish.0, deadline)?;
        if !unsafe { xpc_dictionary_get_bool(reply.0, OK_KEY.as_ptr().cast()) } {
            return Err("private audio XPC child failed closed".into());
        }
        let stdout = data_from_reply(reply.0, STDOUT_KEY, request.metadata.max_stdout)?;
        let stderr = data_from_reply(reply.0, STDERR_KEY, request.metadata.max_stderr)?;
        let exit_code = i32::try_from(unsafe {
            xpc_dictionary_get_int64(reply.0, EXIT_CODE_KEY.as_ptr().cast())
        })
        .ok()
        .filter(|value| *value >= 0);
        let exit_signal = i32::try_from(unsafe {
            xpc_dictionary_get_int64(reply.0, EXIT_SIGNAL_KEY.as_ptr().cast())
        })
        .ok()
        .filter(|value| *value > 0);
        let timed_out = unsafe { xpc_dictionary_get_bool(reply.0, TIMED_OUT_KEY.as_ptr().cast()) };
        if exit_code.is_none() && exit_signal.is_none() {
            return Err("private audio XPC child returned no exact termination status".into());
        }
        Ok(AudioChildOutput {
            exit_code,
            exit_signal,
            stdout,
            stderr,
            timed_out,
        })
    })();
    let settlement = connection.settle(outcome.is_err(), deadline);
    match (outcome, settlement) {
        (Ok(output), Ok(())) => Ok(output),
        (Err(error), Ok(())) => Err(error),
        (outcome, Err(settlement)) => {
            XPC_SETTLEMENT_FAILED.store(true, Ordering::Release);
            let context = outcome
                .err()
                .unwrap_or_else(|| "private audio XPC result was ready".to_string());
            Err(format!("{context}; {settlement}"))
        }
    }
}

use std::os::fd::AsRawFd;

fn data_from_reply(
    reply: XpcObject,
    key: &[u8],
    max_bytes: usize,
) -> Result<Zeroizing<Vec<u8>>, String> {
    let mut length = 0_usize;
    let data = unsafe { xpc_dictionary_get_data(reply, key.as_ptr().cast(), &mut length) };
    if length > max_bytes || (data.is_null() && length != 0) {
        return Err("private audio XPC child output exceeded its exact byte budget".into());
    }
    let mut output = Zeroizing::new(Vec::with_capacity(length));
    if length != 0 {
        output.extend_from_slice(unsafe { std::slice::from_raw_parts(data.cast::<u8>(), length) });
    }
    Ok(output)
}

enum ServicePhase {
    AwaitingBegin,
    AwaitingPrepare,
    Receiving {
        next_sequence: u64,
        child: crate::macos_audio_child::DelayedAudioChild,
        deadline: Instant,
        max_stdout: usize,
        max_stderr: usize,
    },
    Processing,
    Done,
}

impl ServicePhase {
    fn begin(&mut self) -> bool {
        if !matches!(self, Self::AwaitingBegin) {
            return false;
        }
        *self = Self::AwaitingPrepare;
        true
    }

    fn prepare(&mut self, metadata: AudioChildMetadata, attachments: Vec<OwnedFd>) -> bool {
        if !matches!(self, Self::AwaitingPrepare) || metadata.validate().is_err() {
            return false;
        }
        let deadline = Instant::now() + Duration::from_millis(metadata.wall_clock_ms);
        let child = crate::macos_audio_child::DelayedAudioChild::spawn_attested(
            &metadata.executable,
            &metadata.executable_cdhash,
            &metadata.arguments,
            &metadata.environment,
            metadata.audio_len,
            attachments,
        );
        let Ok(child) = child else {
            return false;
        };
        *self = Self::Receiving {
            next_sequence: 0,
            child,
            deadline,
            max_stdout: metadata.max_stdout,
            max_stderr: metadata.max_stderr,
        };
        true
    }

    fn append_chunk(&mut self, sequence: u64, data: &[u8]) -> bool {
        let Self::Receiving {
            next_sequence,
            child,
            ..
        } = self
        else {
            return false;
        };
        if sequence != *next_sequence
            || data.is_empty()
            || data.len() > XPC_CHUNK_BYTES
            || child.append_audio(data).is_err()
        {
            return false;
        }
        let Some(next) = next_sequence.checked_add(1) else {
            return false;
        };
        *next_sequence = next;
        true
    }

    fn finish(&mut self, sequence: u64) -> Option<crate::macos_audio_child::DelayedAudioOutput> {
        let current = std::mem::replace(self, Self::Processing);
        let Self::Receiving {
            next_sequence,
            child,
            deadline,
            max_stdout,
            max_stderr,
        } = current
        else {
            *self = current;
            return None;
        };
        if sequence != next_sequence {
            return None;
        }
        let output = child
            .finish_and_wait(deadline, max_stdout, max_stderr)
            .ok()?;
        *self = Self::Done;
        Some(output)
    }

    fn abort(&mut self) -> bool {
        if matches!(self, Self::Done) {
            return false;
        }
        *self = Self::Done;
        true
    }
}

fn service_parent_requirement() -> Result<CString, String> {
    let identifiers =
        "(identifier \"com.useminutes.desktop\" or identifier \"com.useminutes.desktop.dev\")";
    let requirement = if current_process_is_trusted_distribution() {
        format!(
            "{identifiers} and anchor apple generic and certificate leaf[subject.OU] = \"63TMLKT8HN\""
        )
    } else {
        identifiers.to_string()
    };
    CString::new(requirement)
        .map_err(|_| "private audio XPC parent requirement was malformed".to_string())
}

fn service_reply(message: XpcObject, ok: bool) -> Option<OwnedXpc> {
    let reply = unsafe { xpc_dictionary_create_reply(message) };
    if reply.is_null() {
        return None;
    }
    unsafe { xpc_dictionary_set_bool(reply, OK_KEY.as_ptr().cast(), ok) };
    Some(OwnedXpc(reply))
}

fn service_command(message: XpcObject) -> Option<&'static str> {
    let command = unsafe { xpc_dictionary_get_string(message, COMMAND_KEY.as_ptr().cast()) };
    if command.is_null() {
        return None;
    }
    match unsafe { CStr::from_ptr(command) }.to_bytes() {
        b"begin" => Some("begin"),
        b"prepare" => Some("prepare"),
        b"chunk" => Some("chunk"),
        b"finish" => Some("finish"),
        b"abort" => Some("abort"),
        _ => None,
    }
}

fn handle_service_message(message: XpcObject, state: &Mutex<ServicePhase>) -> Option<OwnedXpc> {
    if !xpc_type_is(message, "dictionary") {
        return None;
    }
    let command = service_command(message)?;
    let mut phase = state.lock().ok()?;
    if command == "abort" {
        return service_reply(message, phase.abort());
    }
    match (&mut *phase, command) {
        (ServicePhase::AwaitingBegin, "begin") => {
            let ok = phase.begin();
            service_reply(message, ok)
        }
        (ServicePhase::AwaitingPrepare, "prepare") => {
            let mut length = 0_usize;
            let data =
                unsafe { xpc_dictionary_get_data(message, DATA_KEY.as_ptr().cast(), &mut length) };
            if data.is_null() || length == 0 || length > MAX_METADATA_BYTES {
                return service_reply(message, false);
            }
            let metadata: AudioChildMetadata = serde_json::from_slice(unsafe {
                std::slice::from_raw_parts(data.cast::<u8>(), length)
            })
            .ok()?;
            if metadata.validate().is_err() {
                return service_reply(message, false);
            }
            let mut attachments = Vec::with_capacity(metadata.attachment_count);
            for index in 0..metadata.attachment_count {
                let key = CString::new(format!("attachment_{index}"))
                    .expect("numeric attachment key cannot contain NUL");
                let descriptor = unsafe { xpc_dictionary_dup_fd(message, key.as_ptr()) };
                if descriptor < 0 {
                    return service_reply(message, false);
                }
                attachments.push(unsafe { OwnedFd::from_raw_fd(descriptor) });
            }
            let ok = phase.prepare(metadata, attachments);
            let reply = service_reply(message, ok)?;
            unsafe { xpc_dictionary_set_bool(reply.0, READY_KEY.as_ptr().cast(), ok) };
            Some(reply)
        }
        (ServicePhase::Receiving { .. }, "chunk") => {
            let sequence =
                unsafe { xpc_dictionary_get_uint64(message, SEQUENCE_KEY.as_ptr().cast()) };
            let mut length = 0_usize;
            let data =
                unsafe { xpc_dictionary_get_data(message, DATA_KEY.as_ptr().cast(), &mut length) };
            if data.is_null() {
                return service_reply(message, false);
            }
            let data = unsafe { std::slice::from_raw_parts(data.cast::<u8>(), length) };
            service_reply(message, phase.append_chunk(sequence, data))
        }
        (ServicePhase::Receiving { .. }, "finish") => {
            let sequence =
                unsafe { xpc_dictionary_get_uint64(message, SEQUENCE_KEY.as_ptr().cast()) };
            let Some(mut output) = phase.finish(sequence) else {
                return service_reply(message, false);
            };
            let reply = service_reply(message, true)?;
            unsafe {
                xpc_dictionary_set_data(
                    reply.0,
                    STDOUT_KEY.as_ptr().cast(),
                    output.stdout.as_ptr().cast(),
                    output.stdout.len(),
                );
                xpc_dictionary_set_data(
                    reply.0,
                    STDERR_KEY.as_ptr().cast(),
                    output.stderr.as_ptr().cast(),
                    output.stderr.len(),
                );
                xpc_dictionary_set_int64(
                    reply.0,
                    EXIT_CODE_KEY.as_ptr().cast(),
                    output.status.code().map(i64::from).unwrap_or(-1),
                );
                use std::os::unix::process::ExitStatusExt;
                xpc_dictionary_set_int64(
                    reply.0,
                    EXIT_SIGNAL_KEY.as_ptr().cast(),
                    output.status.signal().map(i64::from).unwrap_or(-1),
                );
                xpc_dictionary_set_bool(reply.0, TIMED_OUT_KEY.as_ptr().cast(), output.timed_out);
            }
            output.stdout.zeroize();
            output.stderr.zeroize();
            Some(reply)
        }
        _ => service_reply(message, false),
    }
}

fn claim_service_process(claimed: &AtomicBool) -> bool {
    claimed
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}

fn service_request_nonce_matches(message: XpcObject, expected: &[u8; 16]) -> bool {
    let mut length = 0_usize;
    let observed =
        unsafe { xpc_dictionary_get_data(message, SERVICE_NONCE_KEY.as_ptr().cast(), &mut length) };
    !observed.is_null()
        && length == expected.len()
        && unsafe { std::slice::from_raw_parts(observed.cast::<u8>(), length) } == expected
}

#[derive(Debug, PartialEq, Eq)]
enum ServicePeerEvent {
    HandleMessage,
    CancelPeer,
    ExitProcess,
}

fn classify_service_peer_event(
    is_dictionary: bool,
    owns_process_claim: bool,
    was_rejected: bool,
) -> ServicePeerEvent {
    if is_dictionary && !was_rejected {
        ServicePeerEvent::HandleMessage
    } else if owns_process_claim {
        ServicePeerEvent::ExitProcess
    } else {
        ServicePeerEvent::CancelPeer
    }
}

fn awaiting_command_can_claim(command: Option<&str>) -> bool {
    command == Some("begin")
}

fn new_service_process_nonce() -> Result<[u8; 16], String> {
    let mut nonce = [0_u8; 16];
    if unsafe { SecRandomCopyBytes(std::ptr::null(), nonce.len(), nonce.as_mut_ptr()) } == 0 {
        Ok(nonce)
    } else {
        Err("private audio XPC service nonce generation failed".into())
    }
}

fn prepare_service_process() -> Result<(), String> {
    unsafe extern "C" {
        fn setitimer(
            which: libc::c_int,
            new_value: *const libc::itimerval,
            old_value: *mut libc::itimerval,
        ) -> libc::c_int;
    }
    unsafe { libc::umask(0o077) };
    let core = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    if unsafe { libc::setrlimit(libc::RLIMIT_CORE, &core) } != 0 {
        return Err("private audio worker could not disable core dumps".into());
    }
    let baseline = macos_virtual_size()?;
    let address_space_limit = baseline
        .checked_add(SERVICE_ADDRESS_SPACE_GROWTH)
        .ok_or_else(|| "private audio worker address-space ceiling overflowed".to_string())?;
    let address_space = libc::rlimit {
        rlim_cur: address_space_limit,
        rlim_max: address_space_limit,
    };
    if unsafe { libc::setrlimit(libc::RLIMIT_AS, &address_space) } != 0 {
        return Err("private audio worker could not install its address-space ceiling".into());
    }
    let timer = libc::itimerval {
        it_interval: libc::timeval {
            tv_sec: 0,
            tv_usec: 0,
        },
        it_value: libc::timeval {
            tv_sec: SERVICE_PROCESS_WALL_CLOCK.as_secs() as libc::time_t,
            tv_usec: 0,
        },
    };
    if unsafe { setitimer(libc::ITIMER_REAL, &timer, std::ptr::null_mut()) } != 0 {
        return Err("private audio worker could not install its wall-clock ceiling".into());
    }
    Ok(())
}

fn macos_virtual_size() -> Result<u64, String> {
    use mach2::kern_return::KERN_SUCCESS;
    use mach2::task::task_info;
    use mach2::task_info::{
        task_basic_info_64, task_info_t, TASK_BASIC_INFO_64, TASK_BASIC_INFO_64_COUNT,
    };
    use mach2::traps::mach_task_self;

    let mut info = task_basic_info_64::default();
    let mut count = TASK_BASIC_INFO_64_COUNT;
    let status = unsafe {
        task_info(
            mach_task_self(),
            TASK_BASIC_INFO_64,
            (&mut info as *mut task_basic_info_64).cast::<libc::c_int>() as task_info_t,
            &mut count,
        )
    };
    if status != KERN_SUCCESS || count != TASK_BASIC_INFO_64_COUNT {
        return Err("private audio worker could not measure its address space".into());
    }
    Ok(info.virtual_size)
}

pub fn run_service_main() -> ! {
    if prepare_service_process().is_err() {
        unsafe { libc::_exit(70) };
    }
    let service_nonce = match new_service_process_nonce() {
        Ok(nonce) => Arc::new(nonce),
        Err(_) => unsafe { libc::_exit(70) },
    };
    let claimed = Arc::new(AtomicBool::new(false));
    let connections = RcBlock::new(move |peer: XpcObject| {
        if !xpc_type_is(peer, "connection") {
            return;
        }
        let Ok(requirement) = service_parent_requirement() else {
            unsafe { xpc_connection_cancel(peer) };
            return;
        };
        if set_peer_requirement(peer, &requirement).is_err() {
            unsafe { xpc_connection_cancel(peer) };
            return;
        }
        let state = Arc::new(Mutex::new(ServicePhase::AwaitingBegin));
        let message_state = Arc::clone(&state);
        let message_claimed = Arc::clone(&claimed);
        let message_service_nonce = Arc::clone(&service_nonce);
        let peer_owns_process_claim = Arc::new(AtomicBool::new(false));
        let message_peer_owns_process_claim = Arc::clone(&peer_owns_process_claim);
        let peer_was_rejected = Arc::new(AtomicBool::new(false));
        let message_peer_was_rejected = Arc::clone(&peer_was_rejected);
        let peer_address = peer as usize;
        let messages = RcBlock::new(move |message: XpcObject| {
            match classify_service_peer_event(
                xpc_type_is(message, "dictionary"),
                message_peer_owns_process_claim.load(Ordering::Acquire),
                message_peer_was_rejected.load(Ordering::Acquire),
            ) {
                ServicePeerEvent::HandleMessage => {}
                ServicePeerEvent::CancelPeer => {
                    unsafe { xpc_connection_cancel(peer_address as XpcObject) };
                    return;
                }
                ServicePeerEvent::ExitProcess => unsafe { libc::_exit(72) },
            }
            let command = service_command(message);
            let awaiting_begin = message_state
                .lock()
                .is_ok_and(|phase| matches!(*phase, ServicePhase::AwaitingBegin));
            if awaiting_begin && !awaiting_command_can_claim(command) {
                let Some(reply) = service_reply(message, false) else {
                    unsafe { xpc_connection_cancel(peer_address as XpcObject) };
                    return;
                };
                unsafe {
                    xpc_dictionary_set_bool(reply.0, TERMINAL_KEY.as_ptr().cast(), false);
                    xpc_dictionary_set_data(
                        reply.0,
                        SERVICE_NONCE_KEY.as_ptr().cast(),
                        message_service_nonce.as_ptr().cast(),
                        message_service_nonce.len(),
                    );
                    xpc_connection_send_message(peer_address as XpcObject, reply.0);
                }
                return;
            }
            if awaiting_begin && !claim_service_process(&message_claimed) {
                message_peer_was_rejected.store(true, Ordering::Release);
                let Some(reply) = service_reply(message, false) else {
                    unsafe { xpc_connection_cancel(peer_address as XpcObject) };
                    return;
                };
                unsafe {
                    xpc_dictionary_set_bool(reply.0, BUSY_KEY.as_ptr().cast(), true);
                    xpc_dictionary_set_bool(reply.0, TERMINAL_KEY.as_ptr().cast(), false);
                    xpc_dictionary_set_data(
                        reply.0,
                        SERVICE_NONCE_KEY.as_ptr().cast(),
                        message_service_nonce.as_ptr().cast(),
                        message_service_nonce.len(),
                    );
                    xpc_connection_send_message(peer_address as XpcObject, reply.0);
                }
                return;
            }
            if awaiting_begin {
                message_peer_owns_process_claim.store(true, Ordering::Release);
            } else if !service_request_nonce_matches(message, &message_service_nonce) {
                let Some(reply) = service_reply(message, false) else {
                    unsafe { libc::_exit(71) };
                };
                unsafe {
                    xpc_dictionary_set_bool(reply.0, TERMINAL_KEY.as_ptr().cast(), true);
                    xpc_dictionary_set_data(
                        reply.0,
                        SERVICE_NONCE_KEY.as_ptr().cast(),
                        message_service_nonce.as_ptr().cast(),
                        message_service_nonce.len(),
                    );
                    xpc_connection_send_message(peer_address as XpcObject, reply.0);
                }
                let exit_after_send = RcBlock::new(|| unsafe { libc::_exit(71) });
                unsafe {
                    xpc_connection_send_barrier(peer_address as XpcObject, &exit_after_send);
                }
                return;
            }
            let Some(reply) = handle_service_message(message, &message_state) else {
                unsafe { libc::_exit(71) };
            };
            let ok = unsafe { xpc_dictionary_get_bool(reply.0, OK_KEY.as_ptr().cast()) };
            let terminal = !ok
                || message_state
                    .lock()
                    .is_ok_and(|phase| matches!(*phase, ServicePhase::Done));
            unsafe {
                xpc_dictionary_set_bool(reply.0, TERMINAL_KEY.as_ptr().cast(), terminal);
                xpc_dictionary_set_data(
                    reply.0,
                    SERVICE_NONCE_KEY.as_ptr().cast(),
                    message_service_nonce.as_ptr().cast(),
                    message_service_nonce.len(),
                );
                xpc_connection_send_message(peer_address as XpcObject, reply.0);
            }
            if terminal {
                let exit_after_send = RcBlock::new(|| unsafe { libc::_exit(0) });
                unsafe {
                    xpc_connection_send_barrier(peer_address as XpcObject, &exit_after_send);
                }
            }
        });
        unsafe {
            xpc_connection_set_event_handler(peer, &messages);
            xpc_connection_resume(peer);
        }
    });
    unsafe { xpc_main(&connections) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata() -> AudioChildMetadata {
        AudioChildMetadata {
            schema_version: 1,
            executable: PathBuf::from("/usr/bin/true"),
            executable_cdhash: [7_u8; 20],
            arguments: vec![
                crate::macos_audio_child::DelayedAudioChild::audio_descriptor_path().into(),
                crate::macos_audio_child::DelayedAudioChild::attachment_descriptor_path(0).unwrap(),
            ],
            environment: vec![("LANG".into(), "C".into())],
            audio_len: 4,
            attachment_count: 1,
            max_stdout: 1024,
            max_stderr: 1024,
            wall_clock_ms: 1000,
        }
    }

    #[test]
    fn metadata_requires_exact_audio_and_attachment_descriptor_use() {
        let valid = metadata();
        assert!(valid.validate().is_ok());

        let mut duplicate_audio = valid.clone();
        duplicate_audio
            .arguments
            .push(crate::macos_audio_child::DelayedAudioChild::audio_descriptor_path().into());
        assert!(duplicate_audio.validate().is_err());

        let mut missing_attachment = valid.clone();
        missing_attachment.arguments.pop();
        assert!(missing_attachment.validate().is_err());
    }

    #[test]
    fn metadata_rejects_loader_environment_and_unbounded_outputs() {
        let mut loader = metadata();
        loader
            .environment
            .push(("DYLD_INSERT_LIBRARIES".into(), "/tmp/hostile".into()));
        // Structural validation permits serialization but the exact child
        // constructor rejects loader keys before spawn.
        assert!(loader.validate().is_ok());

        let mut output = metadata();
        output.max_stdout = MAX_STDOUT_BYTES + 1;
        assert!(output.validate().is_err());
    }

    #[test]
    fn settlement_binds_every_request_to_one_service_process_nonce() {
        let first = [1_u8; 16];
        let second = [2_u8; 16];
        let mut expected = None;
        bind_service_nonce(&mut expected, first).unwrap();
        bind_service_nonce(&mut expected, first).unwrap();
        assert!(bind_service_nonce(&mut expected, second).is_err());
    }

    #[test]
    fn service_admission_rejects_stale_abort_without_claiming_fresh_process() {
        assert!(awaiting_command_can_claim(Some("begin")));
        assert!(!awaiting_command_can_claim(Some("abort")));
        assert!(!awaiting_command_can_claim(Some("prepare")));
        assert!(!awaiting_command_can_claim(None));
    }

    #[test]
    fn busy_peer_disconnect_does_not_retire_owner() {
        assert_eq!(
            classify_service_peer_event(false, false, true),
            ServicePeerEvent::CancelPeer
        );
        assert_eq!(
            classify_service_peer_event(false, true, false),
            ServicePeerEvent::ExitProcess
        );
    }
}
