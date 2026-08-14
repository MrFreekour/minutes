//! Runtime loader for the isolated sherpa transcription plugin.
//!
//! sherpa-onnx and pyannote each vendor their own ONNX Runtime and their own
//! kaldi-native-fbank. One executable holds one definition per symbol, so
//! statically linking both breaks one of them no matter which copy wins
//! (issue #683). The plugin keeps sherpa's copies inside its own image, where
//! macOS two-level namespaces bind them to itself, so both stacks can run in
//! one process (issue #685).
//!
//! Everything here is failure-tolerant on purpose. A missing or unloadable
//! plugin must degrade exactly like a missing model already does: the caller
//! reports why, transcription falls back to Whisper, and a recording is never
//! lost. What it must never do is fall back silently.

use std::ffi::{c_char, c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::config::Config;

/// C surface version this build expects. Must match the plugin's own constant.
///
/// The plugin ships separately from the binaries, so a mismatched pair is a
/// real possibility. Refusing to load is far better than calling through a
/// changed signature.
const EXPECTED_ABI_VERSION: u32 = 2;

/// Bytes reserved for a failure message from the plugin.
const ERROR_BUFFER_BYTES: usize = 512;

type AbiVersionFn = unsafe extern "C" fn() -> u32;
type CreateFn = unsafe extern "C" fn(*const c_char, *mut c_char, usize) -> *mut c_void;
type TranscribeFn = unsafe extern "C" fn(*mut c_void, u32, *const f32, usize) -> *mut c_char;
type FreeStringFn = unsafe extern "C" fn(*mut c_char);
type DestroyFn = unsafe extern "C" fn(*mut c_void);
type StreamCreateFn = unsafe extern "C" fn(*const c_char, *mut c_char, usize) -> *mut c_void;
type StreamFeedFn = unsafe extern "C" fn(*mut c_void, u32, *const f32, usize) -> *mut c_char;
type StreamFinishFn = unsafe extern "C" fn(*mut c_void) -> *mut c_char;
type StreamResetFn = unsafe extern "C" fn(*mut c_void) -> bool;
type StreamDestroyFn = unsafe extern "C" fn(*mut c_void);

struct PluginApi {
    // Held so the library outlives every symbol taken from it. Never unloaded:
    // the embedded ONNX Runtime registers process-global state, and unloading
    // it while any of that is live is not worth the risk for a plugin a
    // process loads at most once.
    _library: libloading::Library,
    create: CreateFn,
    transcribe: TranscribeFn,
    free_string: FreeStringFn,
    destroy: DestroyFn,
    stream_create: StreamCreateFn,
    stream_feed: StreamFeedFn,
    stream_finish: StreamFinishFn,
    stream_reset: StreamResetFn,
    stream_destroy: StreamDestroyFn,
}

// The plugin's entry points are called from whichever thread runs a
// transcription, and `TransducerRecognizer` use is serialized by `&mut self`
// on the handle wrapper below.
unsafe impl Send for PluginApi {}
unsafe impl Sync for PluginApi {}

/// Successfully loaded plugin, cached for the life of the process.
///
/// Only success is cached. An earlier version cached the failure too, with a
/// comment claiming the answer could not change without a restart; that is
/// plainly false for a filesystem probe, and it meant a long-lived process
/// (an MCP server, the desktop app) could never pick up a plugin installed
/// after it started. Re-probing while unloaded costs a few `exists` calls,
/// since `dlopen` is only attempted for a path that is actually there.
static PLUGIN: OnceLock<&'static PluginApi> = OnceLock::new();

/// Platform-specific plugin file name.
fn plugin_file_name() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "libminutes_sherpa.dylib"
    }
    #[cfg(target_os = "windows")]
    {
        "minutes_sherpa.dll"
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        "libminutes_sherpa.so"
    }
}

/// Where the plugin is looked for, in order.
///
/// `MINUTES_SHERPA_PLUGIN` wins so a developer can point at a freshly built
/// dylib in `target/` without installing it. Otherwise it is looked for beside
/// the running executable, which is how a packaged app carries it, and then in
/// the Minutes install root, which is the intended install location once the
/// plugin ships as a release asset. Nothing installs it there automatically
/// yet, so a source build copies it or points the variable at `target/`.
pub fn candidate_paths(config: &Config) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(explicit) = std::env::var("MINUTES_SHERPA_PLUGIN") {
        if !explicit.trim().is_empty() {
            candidates.push(PathBuf::from(explicit));
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(plugin_file_name()));
        }
    }
    candidates.push(
        crate::sherpa_engine::installs_root(config)
            .join("lib")
            .join(plugin_file_name()),
    );
    candidates
}

/// Load the plugin from `path`, checking the ABI before taking any symbol.
fn load_from(path: &Path) -> Result<PluginApi, String> {
    // SAFETY: dlopen runs the library's initializers. The path is one Minutes
    // controls (an env override, the executable's own directory, or the
    // install root), and the file is our own build artifact.
    let library = unsafe { libloading::Library::new(path) }
        .map_err(|e| format!("could not load {}: {e}", path.display()))?;

    unsafe {
        let abi: libloading::Symbol<AbiVersionFn> =
            library.get(b"minutes_sherpa_abi_version").map_err(|e| {
                format!(
                    "{} is missing minutes_sherpa_abi_version: {e}",
                    path.display()
                )
            })?;
        let found = abi();
        if found != EXPECTED_ABI_VERSION {
            return Err(format!(
                "{} reports plugin ABI {found}, but this build speaks {EXPECTED_ABI_VERSION}. \
                 Rebuild the plugin from the same checkout as this binary.",
                path.display()
            ));
        }

        let create: libloading::Symbol<CreateFn> = library
            .get(b"minutes_sherpa_create")
            .map_err(|e| format!("{} is missing minutes_sherpa_create: {e}", path.display()))?;
        let transcribe: libloading::Symbol<TranscribeFn> =
            library.get(b"minutes_sherpa_transcribe").map_err(|e| {
                format!(
                    "{} is missing minutes_sherpa_transcribe: {e}",
                    path.display()
                )
            })?;
        let free_string: libloading::Symbol<FreeStringFn> =
            library.get(b"minutes_sherpa_free_string").map_err(|e| {
                format!(
                    "{} is missing minutes_sherpa_free_string: {e}",
                    path.display()
                )
            })?;
        let destroy: libloading::Symbol<DestroyFn> = library
            .get(b"minutes_sherpa_destroy")
            .map_err(|e| format!("{} is missing minutes_sherpa_destroy: {e}", path.display()))?;

        let stream_create: libloading::Symbol<StreamCreateFn> =
            library.get(b"minutes_sherpa_stream_create").map_err(|e| {
                format!(
                    "{} is missing minutes_sherpa_stream_create: {e}",
                    path.display()
                )
            })?;
        let stream_feed: libloading::Symbol<StreamFeedFn> =
            library.get(b"minutes_sherpa_stream_feed").map_err(|e| {
                format!(
                    "{} is missing minutes_sherpa_stream_feed: {e}",
                    path.display()
                )
            })?;
        let stream_finish: libloading::Symbol<StreamFinishFn> =
            library.get(b"minutes_sherpa_stream_finish").map_err(|e| {
                format!(
                    "{} is missing minutes_sherpa_stream_finish: {e}",
                    path.display()
                )
            })?;
        let stream_reset: libloading::Symbol<StreamResetFn> =
            library.get(b"minutes_sherpa_stream_reset").map_err(|e| {
                format!(
                    "{} is missing minutes_sherpa_stream_reset: {e}",
                    path.display()
                )
            })?;
        let stream_destroy: libloading::Symbol<StreamDestroyFn> =
            library.get(b"minutes_sherpa_stream_destroy").map_err(|e| {
                format!(
                    "{} is missing minutes_sherpa_stream_destroy: {e}",
                    path.display()
                )
            })?;

        let (
            create,
            transcribe,
            free_string,
            destroy,
            stream_create,
            stream_feed,
            stream_finish,
            stream_reset,
            stream_destroy,
        ) = (
            *create,
            *transcribe,
            *free_string,
            *destroy,
            *stream_create,
            *stream_feed,
            *stream_finish,
            *stream_reset,
            *stream_destroy,
        );
        Ok(PluginApi {
            _library: library,
            create,
            transcribe,
            free_string,
            destroy,
            stream_create,
            stream_feed,
            stream_finish,
            stream_reset,
            stream_destroy,
        })
    }
}

/// Load the plugin once per process, remembering the outcome either way.
///
/// Retrying a failed dlopen on every recording would repeat the same slow
/// failure and log noise; the answer cannot change without a restart.
fn plugin(config: &Config) -> Result<&'static PluginApi, String> {
    if let Some(api) = PLUGIN.get() {
        return Ok(api);
    }
    match try_load(config) {
        Ok(api) => {
            // Leaked rather than dropped: if two threads race here the loser's
            // library would otherwise unload while its ONNX Runtime globals are
            // initialized. Leaking one copy in a rare race is cheaper than
            // reasoning about that teardown.
            let leaked: &'static PluginApi = Box::leak(Box::new(api));
            Ok(PLUGIN.get_or_init(|| leaked))
        }
        Err(reason) => Err(reason),
    }
}

/// Try every candidate path once, reporting why each was rejected.
fn try_load(config: &Config) -> Result<PluginApi, String> {
    {
        let candidates = candidate_paths(config);
        let mut reasons = Vec::new();
        for candidate in &candidates {
            if !candidate.exists() {
                reasons.push(format!("{}: not found", candidate.display()));
                continue;
            }
            match load_from(candidate) {
                Ok(api) => {
                    tracing::info!(plugin = %candidate.display(), "loaded sherpa plugin");
                    return Ok(api);
                }
                Err(reason) => reasons.push(reason),
            }
        }
        // Deliberately does not name `minutes setup --sherpa`: that
        // command installs the model, not the plugin. Pointing at it would
        // send someone round a loop that cannot fix this.
        Err(format!(
            "the sherpa transcription plugin could not be loaded. Build it with \
                 `(cd crates/sherpa-plugin && cargo build --release)` and copy the library \
                 beside the executable or into the Minutes lib directory, or point \
                 MINUTES_SHERPA_PLUGIN at it. Tried: {}",
            reasons.join("; ")
        ))
    }
}

/// A loaded recognizer. Releases the plugin handle on drop.
pub struct PluginRecognizer {
    api: &'static PluginApi,
    handle: *mut c_void,
}

// The handle is only reachable through `&mut self`, so no two threads can be
// inside the recognizer at once.
unsafe impl Send for PluginRecognizer {}

impl PluginRecognizer {
    /// Load the parakeet model in `model_dir` through the plugin.
    pub fn new(model_dir: &Path, config: &Config) -> Result<Self, String> {
        let api = plugin(config)?;
        let dir = CString::new(model_dir.to_string_lossy().as_ref())
            .map_err(|_| "sherpa model path contained an interior null".to_string())?;
        let mut error = vec![0i8 as c_char; ERROR_BUFFER_BYTES];
        // SAFETY: `dir` is a valid null-terminated string, and `error` has
        // ERROR_BUFFER_BYTES writable bytes.
        let handle = unsafe { (api.create)(dir.as_ptr(), error.as_mut_ptr(), error.len()) };
        if handle.is_null() {
            // SAFETY: the plugin always null-terminates within the buffer.
            let message = unsafe { CStr::from_ptr(error.as_ptr()) }
                .to_string_lossy()
                .into_owned();
            let message = if message.is_empty() {
                "the sherpa plugin failed to load the model".to_string()
            } else {
                message
            };
            return Err(message);
        }
        Ok(Self { api, handle })
    }

    /// Transcribe 16 kHz mono f32 samples.
    pub fn transcribe(&mut self, sample_rate: u32, samples: &[f32]) -> Result<String, String> {
        // SAFETY: `handle` came from the plugin and is still alive; `samples`
        // is a valid slice for its length.
        let raw = unsafe {
            (self.api.transcribe)(self.handle, sample_rate, samples.as_ptr(), samples.len())
        };
        if raw.is_null() {
            return Err("the sherpa plugin failed to transcribe a window".to_string());
        }
        // SAFETY: a non-null result is a plugin-allocated C string, freed
        // below through the plugin's own allocator rather than this one's.
        let text = unsafe { CStr::from_ptr(raw) }
            .to_string_lossy()
            .into_owned();
        unsafe { (self.api.free_string)(raw) };
        Ok(text)
    }
}

impl Drop for PluginRecognizer {
    fn drop(&mut self) {
        if self.handle.is_null() {
            return;
        }
        // SAFETY: destroyed exactly once, and the handle is not used again.
        unsafe { (self.api.destroy)(self.handle) };
        self.handle = std::ptr::null_mut();
    }
}

/// Retained true-streaming recognizer. The model stays resident while
/// `reset()` swaps only the cheap online stream between utterances/sessions.
pub struct PluginStreamingRecognizer {
    api: &'static PluginApi,
    handle: *mut c_void,
}

unsafe impl Send for PluginStreamingRecognizer {}

impl PluginStreamingRecognizer {
    pub fn new(model_dir: &Path, config: &Config) -> Result<Self, String> {
        let api = plugin(config)?;
        let dir = CString::new(model_dir.to_string_lossy().as_ref())
            .map_err(|_| "sherpa model path contained an interior null".to_string())?;
        let mut error = vec![0i8 as c_char; ERROR_BUFFER_BYTES];
        let handle = unsafe { (api.stream_create)(dir.as_ptr(), error.as_mut_ptr(), error.len()) };
        if handle.is_null() {
            let message = unsafe { CStr::from_ptr(error.as_ptr()) }
                .to_string_lossy()
                .into_owned();
            return Err(if message.is_empty() {
                "the sherpa plugin failed to load the streaming model".to_string()
            } else {
                message
            });
        }
        Ok(Self { api, handle })
    }

    fn take_plugin_text(&self, raw: *mut c_char, operation: &str) -> Result<String, String> {
        if raw.is_null() {
            return Err(format!("the sherpa plugin failed to {operation}"));
        }
        let text = unsafe { CStr::from_ptr(raw) }
            .to_string_lossy()
            .into_owned();
        unsafe { (self.api.free_string)(raw) };
        Ok(text)
    }

    /// Feed one 16 kHz mono chunk. This advances the model state instead of
    /// re-transcribing the accumulated utterance.
    pub fn feed(&mut self, sample_rate: u32, samples: &[f32]) -> Result<String, String> {
        let raw = unsafe {
            (self.api.stream_feed)(self.handle, sample_rate, samples.as_ptr(), samples.len())
        };
        self.take_plugin_text(raw, "decode a streaming chunk")
    }

    pub fn finish(&mut self) -> Result<String, String> {
        let raw = unsafe { (self.api.stream_finish)(self.handle) };
        self.take_plugin_text(raw, "finish the streaming utterance")
    }

    pub fn reset(&mut self) -> Result<(), String> {
        if unsafe { (self.api.stream_reset)(self.handle) } {
            Ok(())
        } else {
            Err("the sherpa plugin failed to reset its streaming session".to_string())
        }
    }
}

impl Drop for PluginStreamingRecognizer {
    fn drop(&mut self) {
        if self.handle.is_null() {
            return;
        }
        unsafe { (self.api.stream_destroy)(self.handle) };
        self.handle = std::ptr::null_mut();
    }
}

/// Whether a usable plugin is present, for setup and health reporting.
pub fn is_available(config: &Config) -> bool {
    plugin(config).is_ok()
}

/// Why the plugin is unavailable, or `None` when it loaded.
pub fn unavailable_reason(config: &Config) -> Option<String> {
    plugin(config).err()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_paths_prefer_the_environment_override() {
        // A developer pointing at a freshly built dylib must win over an
        // installed copy, or local iteration silently tests the stale one.
        let config = Config::default();
        temp_env_var(
            "MINUTES_SHERPA_PLUGIN",
            Some("/tmp/explicit-plugin.dylib"),
            || {
                let paths = candidate_paths(&config);
                assert_eq!(
                    paths.first().unwrap(),
                    Path::new("/tmp/explicit-plugin.dylib")
                );
            },
        );
    }

    #[test]
    fn candidate_paths_ignore_a_blank_override_and_still_search() {
        // An exported-but-empty variable is a common shell accident; treating
        // it as a path would make every candidate fail on "".
        let config = Config::default();
        temp_env_var("MINUTES_SHERPA_PLUGIN", Some("   "), || {
            let paths = candidate_paths(&config);
            assert!(!paths.is_empty());
            assert!(paths.iter().all(|p| p.file_name().is_some()));
        });
    }

    #[test]
    fn candidate_paths_include_the_install_root() {
        let config = Config::default();
        temp_env_var("MINUTES_SHERPA_PLUGIN", None, || {
            let paths = candidate_paths(&config);
            let root = crate::sherpa_engine::installs_root(&config).join("lib");
            assert!(
                paths.iter().any(|p| p.starts_with(&root)),
                "install root {} missing from {paths:?}",
                root.display()
            );
        });
    }

    #[test]
    fn loading_a_file_that_is_not_a_library_reports_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join(plugin_file_name());
        std::fs::write(&fake, b"not a dylib").unwrap();
        // `unwrap_err` would require PluginApi: Debug, and deriving Debug on a
        // struct of raw function pointers adds nothing worth reading.
        let error = match load_from(&fake) {
            Err(error) => error,
            Ok(_) => panic!("a text file must not load as a plugin"),
        };
        assert!(
            error.contains(&fake.display().to_string()),
            "error should name the file it tried: {error}"
        );
    }

    fn temp_env_var(key: &str, value: Option<&str>, body: impl FnOnce()) {
        let previous = std::env::var(key).ok();
        match value {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
        body();
        match previous {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }
}
