//! Isolated sherpa-onnx transcription plugin.
//!
//! # Why this is a separate dynamic library
//!
//! sherpa-onnx bundles its own ONNX Runtime (1.17.1) and its own
//! kaldi-native-fbank. pyannote, which Minutes uses for diarization, brings a
//! different ONNX Runtime (1.22.0 via ort-sys) and a different
//! kaldi-native-fbank under the same archive names. Statically linking both
//! into one executable gives the image one definition per symbol, so whichever
//! copy wins serves both consumers, and every possible resolution breaks one of
//! them: voice enrollment fails with an ORT API 17/22 mismatch, or aborts
//! inside feature extraction, or sherpa itself aborts (issue #683).
//!
//! Moving sherpa behind a `dlopen`ed boundary removes the conflict rather than
//! arbitrating it. A Rust `cdylib` exports only its own `#[no_mangle]` surface,
//! so the ONNX Runtime embedded here stays internal, and macOS two-level
//! namespaces bind each image's calls to its own copy. Verified on arm64: a
//! host holding ORT 1.22 initialized pyannote, loaded this plugin, transcribed
//! real audio through ORT 1.17, and initialized pyannote again afterwards
//! (issue #685).
//!
//! # Contract
//!
//! Every entry point is `extern "C"` and unwind-safe: a panic inside the plugin
//! must not cross back into the host, because unwinding across an FFI boundary
//! is undefined behavior. Each function therefore catches panics and reports
//! failure through its return value.
//!
//! Strings handed to the host are allocated here and must be returned with
//! [`minutes_sherpa_free_string`]; a handle from [`minutes_sherpa_create`] must
//! be returned with [`minutes_sherpa_destroy`]. Both tolerate null.

use std::ffi::{c_char, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};

use sherpa_rs::sherpa_rs_sys as sys;
use sherpa_rs::transducer::{TransducerConfig, TransducerRecognizer};

/// Version of the C surface below.
///
/// The host refuses to load a plugin reporting a different value. The plugin
/// is delivered separately from the binaries, so a mismatched pair is a real
/// possibility rather than a theoretical one, and failing to load is much
/// better than calling through a changed signature.
///
/// Bump on any change to a signature, to the ownership rules, or to the
/// meaning of a return value.
pub const MINUTES_SHERPA_ABI_VERSION: u32 = 2;

/// Report the ABI version this plugin implements.
#[no_mangle]
pub extern "C" fn minutes_sherpa_abi_version() -> u32 {
    MINUTES_SHERPA_ABI_VERSION
}

struct Recognizer {
    inner: TransducerRecognizer,
}

/// A genuinely incremental, fully local recognizer. Unlike the offline
/// `TransducerRecognizer` above, this owns sherpa's online model state and one
/// resettable stream. Audio is accepted once and decoded forward; no previous
/// audio is re-run to produce a partial.
struct StreamingRecognizer {
    recognizer: *const sys::SherpaOnnxOnlineRecognizer,
    stream: *const sys::SherpaOnnxOnlineStream,
}

impl Drop for StreamingRecognizer {
    fn drop(&mut self) {
        unsafe {
            if !self.stream.is_null() {
                sys::SherpaOnnxDestroyOnlineStream(self.stream);
                self.stream = std::ptr::null();
            }
            if !self.recognizer.is_null() {
                sys::SherpaOnnxDestroyOnlineRecognizer(self.recognizer);
                self.recognizer = std::ptr::null();
            }
        }
    }
}

fn streaming_recognizer(model_dir: &str) -> Result<StreamingRecognizer, String> {
    let path = |candidates: &[&str]| {
        let file = candidates
            .iter()
            .map(|file| std::path::Path::new(model_dir).join(file))
            .find(|path| path.is_file())
            .ok_or_else(|| {
                format!(
                    "streaming model in {model_dir} is missing one of: {}",
                    candidates.join(", ")
                )
            })?;
        CString::new(file.to_string_lossy().as_ref())
            .map_err(|_| "sherpa model path contained an interior null".to_string())
    };
    let encoder = path(&["encoder.int8.onnx", "encoder-epoch-99-avg-1.int8.onnx"])?;
    let decoder = path(&["decoder.int8.onnx", "decoder-epoch-99-avg-1.onnx"])?;
    let joiner = path(&["joiner.int8.onnx", "joiner-epoch-99-avg-1.int8.onnx"])?;
    let tokens = path(&["tokens.txt"])?;
    let provider = CString::new("cpu").expect("static string has no null");
    let model_type = CString::new("").expect("static string has no null");
    let decoding_method = CString::new("greedy_search").expect("static string has no null");

    // The sherpa C API intentionally treats a zero-initialized config as its
    // baseline. Set every field that is load-bearing for an online transducer;
    // unused model families remain null.
    let mut config: sys::SherpaOnnxOnlineRecognizerConfig = unsafe { std::mem::zeroed() };
    config.feat_config.sample_rate = 16_000;
    config.feat_config.feature_dim = 80;
    config.model_config.transducer.encoder = encoder.as_ptr();
    config.model_config.transducer.decoder = decoder.as_ptr();
    config.model_config.transducer.joiner = joiner.as_ptr();
    config.model_config.tokens = tokens.as_ptr();
    config.model_config.num_threads = 4;
    config.model_config.provider = provider.as_ptr();
    config.model_config.debug = 0;
    // Empty asks sherpa to auto-detect the supported online transducer family.
    config.model_config.model_type = model_type.as_ptr();
    config.decoding_method = decoding_method.as_ptr();
    config.max_active_paths = 4;
    config.enable_endpoint = 0;

    let recognizer = unsafe { sys::SherpaOnnxCreateOnlineRecognizer(&config) };
    if recognizer.is_null() {
        return Err("failed to load sherpa streaming model".to_string());
    }
    let stream = unsafe { sys::SherpaOnnxCreateOnlineStream(recognizer) };
    if stream.is_null() {
        unsafe { sys::SherpaOnnxDestroyOnlineRecognizer(recognizer) };
        return Err("failed to create sherpa streaming session".to_string());
    }
    Ok(StreamingRecognizer { recognizer, stream })
}

unsafe fn streaming_text(recognizer: &StreamingRecognizer) -> Option<CString> {
    let result = sys::SherpaOnnxGetOnlineStreamResult(recognizer.recognizer, recognizer.stream);
    if result.is_null() {
        return None;
    }
    let text = if (*result).text.is_null() {
        None
    } else {
        CString::new(CStr::from_ptr((*result).text).to_bytes()).ok()
    };
    sys::SherpaOnnxDestroyOnlineRecognizerResult(result);
    text
}

/// Write a message into the host's error buffer, always null-terminated.
fn write_error(err: *mut c_char, err_len: usize, message: &str) {
    if err.is_null() || err_len == 0 {
        return;
    }
    let bytes = message.as_bytes();
    // Leave room for the terminator, and never split a UTF-8 sequence: the
    // host reads this with CStr and then from_utf8_lossy.
    //
    // `take < bytes.len()` is load-bearing. Without it, a message that fits
    // (the common case) indexes bytes[bytes.len()] and panics, and this runs
    // outside catch_unwind, so that panic would cross the FFI boundary.
    let mut take = bytes.len().min(err_len - 1);
    while take > 0 && take < bytes.len() && (bytes[take] & 0b1100_0000) == 0b1000_0000 {
        take -= 1;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), err as *mut u8, take);
        *err.add(take) = 0;
    }
}

/// Load the parakeet model in `model_dir` and return an opaque recognizer.
///
/// Returns null on failure, writing a reason into `err`. `model_dir` must be a
/// valid null-terminated UTF-8 path.
///
/// # Safety
///
/// `model_dir` must point to a null-terminated string. `err` must either be
/// null or point to at least `err_len` writable bytes.
#[no_mangle]
pub unsafe extern "C" fn minutes_sherpa_create(
    model_dir: *const c_char,
    err: *mut c_char,
    err_len: usize,
) -> *mut std::ffi::c_void {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if model_dir.is_null() {
            return Err("model_dir was null".to_string());
        }
        let dir = CStr::from_ptr(model_dir)
            .to_str()
            .map_err(|_| "model_dir was not valid UTF-8".to_string())?
            .to_string();
        let path = |file: &str| format!("{dir}/{file}");
        let cfg = TransducerConfig {
            encoder: path("encoder.int8.onnx"),
            decoder: path("decoder.int8.onnx"),
            joiner: path("joiner.int8.onnx"),
            tokens: path("tokens.txt"),
            num_threads: 4,
            decoding_method: "greedy_search".into(),
            // Empty model_type -> sherpa auto-detects the NeMo parakeet-TDT
            // loader. The default "transducer" forces the generic loader,
            // which fails with "vocab_size does not exist in the metadata".
            model_type: String::new(),
            debug: false,
            ..Default::default()
        };
        TransducerRecognizer::new(cfg)
            .map(|inner| Box::new(Recognizer { inner }))
            .map_err(|e| format!("failed to load sherpa model: {e}"))
    }));

    // Reporting the failure is itself wrapped: write_error is panic-free by
    // construction, but "by construction" is exactly what the bounds bug above
    // disproved once, and a panic escaping here is undefined behavior rather
    // than a bad error message.
    let report = |message: &str| {
        let _ = catch_unwind(AssertUnwindSafe(|| write_error(err, err_len, message)));
    };
    match result {
        Ok(Ok(recognizer)) => Box::into_raw(recognizer) as *mut std::ffi::c_void,
        Ok(Err(message)) => {
            report(&message);
            std::ptr::null_mut()
        }
        Err(_) => {
            report("sherpa plugin panicked while loading the model");
            std::ptr::null_mut()
        }
    }
}

/// Load a true streaming transducer recognizer and create its first stream.
///
/// This is deliberately a separate handle from the offline recognizer: a
/// caller that only performs batch transcription does not pay for a second
/// model, while dictation can retain this handle across sessions and reset its
/// small stream state without reloading model weights.
#[no_mangle]
pub unsafe extern "C" fn minutes_sherpa_stream_create(
    model_dir: *const c_char,
    err: *mut c_char,
    err_len: usize,
) -> *mut std::ffi::c_void {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if model_dir.is_null() {
            return Err("model_dir was null".to_string());
        }
        let dir = CStr::from_ptr(model_dir)
            .to_str()
            .map_err(|_| "model_dir was not valid UTF-8".to_string())?;
        streaming_recognizer(dir).map(Box::new)
    }));
    let report = |message: &str| {
        let _ = catch_unwind(AssertUnwindSafe(|| write_error(err, err_len, message)));
    };
    match result {
        Ok(Ok(recognizer)) => Box::into_raw(recognizer) as *mut std::ffi::c_void,
        Ok(Err(message)) => {
            report(&message);
            std::ptr::null_mut()
        }
        Err(_) => {
            report("sherpa plugin panicked while loading the streaming model");
            std::ptr::null_mut()
        }
    }
}

/// Accept one audio chunk, decode every newly-ready frame, and return the
/// current provisional text. The result is an owned plugin string; an empty
/// string means the decoder does not yet have a useful token.
#[no_mangle]
pub unsafe extern "C" fn minutes_sherpa_stream_feed(
    handle: *mut std::ffi::c_void,
    sample_rate: u32,
    samples: *const f32,
    len: usize,
) -> *mut c_char {
    if handle.is_null() || (samples.is_null() && len != 0) || len > i32::MAX as usize {
        return std::ptr::null_mut();
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        let recognizer = &mut *(handle as *mut StreamingRecognizer);
        sys::SherpaOnnxOnlineStreamAcceptWaveform(
            recognizer.stream,
            sample_rate as i32,
            samples,
            len as i32,
        );
        while sys::SherpaOnnxIsOnlineStreamReady(recognizer.recognizer, recognizer.stream) != 0 {
            sys::SherpaOnnxDecodeOnlineStream(recognizer.recognizer, recognizer.stream);
        }
        streaming_text(recognizer).unwrap_or_else(|| CString::new("").unwrap())
    }));
    match result {
        Ok(text) => text.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Flush the final decoder frames and return the final online hypothesis.
#[no_mangle]
pub unsafe extern "C" fn minutes_sherpa_stream_finish(
    handle: *mut std::ffi::c_void,
) -> *mut c_char {
    if handle.is_null() {
        return std::ptr::null_mut();
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        let recognizer = &mut *(handle as *mut StreamingRecognizer);
        sys::SherpaOnnxOnlineStreamInputFinished(recognizer.stream);
        while sys::SherpaOnnxIsOnlineStreamReady(recognizer.recognizer, recognizer.stream) != 0 {
            sys::SherpaOnnxDecodeOnlineStream(recognizer.recognizer, recognizer.stream);
        }
        streaming_text(recognizer).unwrap_or_else(|| CString::new("").unwrap())
    }));
    match result {
        Ok(text) => text.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Replace the finished stream while retaining the expensive model.
#[no_mangle]
pub unsafe extern "C" fn minutes_sherpa_stream_reset(handle: *mut std::ffi::c_void) -> bool {
    if handle.is_null() {
        return false;
    }
    catch_unwind(AssertUnwindSafe(|| {
        let recognizer = &mut *(handle as *mut StreamingRecognizer);
        let replacement = sys::SherpaOnnxCreateOnlineStream(recognizer.recognizer);
        if replacement.is_null() {
            return false;
        }
        sys::SherpaOnnxDestroyOnlineStream(recognizer.stream);
        recognizer.stream = replacement;
        true
    }))
    .unwrap_or(false)
}

/// Release the retained streaming model and its current stream.
#[no_mangle]
pub unsafe extern "C" fn minutes_sherpa_stream_destroy(handle: *mut std::ffi::c_void) {
    if handle.is_null() {
        return;
    }
    let _ = catch_unwind(AssertUnwindSafe(|| {
        drop(Box::from_raw(handle as *mut StreamingRecognizer));
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run write_error against a fixed buffer and read back what the host would.
    fn write_and_read(message: &str, buffer_len: usize) -> String {
        let mut buffer = vec![0 as c_char; buffer_len];
        write_error(buffer.as_mut_ptr(), buffer.len(), message);
        unsafe { CStr::from_ptr(buffer.as_ptr()) }
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn a_message_that_fits_is_written_whole() {
        // The case that panicked: take == bytes.len(), so the truncation loop
        // indexed one past the end. This is the COMMON path, reached by every
        // ordinary model-load failure.
        assert_eq!(
            write_and_read("failed to load sherpa model: bad tokens", 512),
            "failed to load sherpa model: bad tokens"
        );
    }

    #[test]
    fn an_oversized_message_is_truncated_and_terminated() {
        let long = "x".repeat(100);
        let out = write_and_read(&long, 16);
        assert_eq!(out.len(), 15, "must leave room for the terminator");
        assert!(out.chars().all(|c| c == 'x'));
    }

    #[test]
    fn truncation_never_splits_a_multibyte_character() {
        // Three-byte characters against a buffer whose cut lands mid-sequence.
        // A split would make the host's CStr read produce replacement
        // characters instead of a shorter but correct message.
        for buffer_len in 2..12 {
            let out = write_and_read("日本語テキスト", buffer_len);
            assert!(
                !out.contains('\u{FFFD}'),
                "buffer {buffer_len} produced a split character: {out:?}"
            );
        }
    }

    #[test]
    fn degenerate_buffers_and_messages_are_safe() {
        assert_eq!(write_and_read("", 8), "");
        assert_eq!(write_and_read("anything", 1), "");
        // A null buffer must be ignored rather than dereferenced.
        write_error(std::ptr::null_mut(), 32, "ignored");
        // A zero length must be ignored even with a real pointer.
        let mut buffer = vec![7 as c_char; 4];
        write_error(buffer.as_mut_ptr(), 0, "ignored");
        assert_eq!(buffer[0], 7, "nothing may be written when err_len is 0");
    }
}

/// Transcribe `samples` and return a null-terminated UTF-8 string.
///
/// Returns null on failure. The caller owns the result and must release it
/// with [`minutes_sherpa_free_string`].
///
/// # Safety
///
/// `handle` must come from [`minutes_sherpa_create`] and not have been
/// destroyed. `samples` must point to `len` readable floats.
#[no_mangle]
pub unsafe extern "C" fn minutes_sherpa_transcribe(
    handle: *mut std::ffi::c_void,
    sample_rate: u32,
    samples: *const f32,
    len: usize,
) -> *mut c_char {
    if handle.is_null() || (samples.is_null() && len != 0) {
        return std::ptr::null_mut();
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        let recognizer = &mut *(handle as *mut Recognizer);
        let audio = if len == 0 {
            &[][..]
        } else {
            std::slice::from_raw_parts(samples, len)
        };
        let text = recognizer.inner.transcribe(sample_rate, audio);
        // Interior nulls cannot survive a C string; treat them as corruption
        // rather than silently truncating a transcript.
        CString::new(text).ok()
    }));

    match result {
        Ok(Some(text)) => text.into_raw(),
        _ => std::ptr::null_mut(),
    }
}

/// Release a string returned by [`minutes_sherpa_transcribe`].
///
/// # Safety
///
/// `text` must be null, or a pointer returned by
/// [`minutes_sherpa_transcribe`] that has not already been freed.
#[no_mangle]
pub unsafe extern "C" fn minutes_sherpa_free_string(text: *mut c_char) {
    if text.is_null() {
        return;
    }
    drop(CString::from_raw(text));
}

/// Release a recognizer from [`minutes_sherpa_create`].
///
/// # Safety
///
/// `handle` must be null, or a handle from [`minutes_sherpa_create`] that has
/// not already been destroyed.
#[no_mangle]
pub unsafe extern "C" fn minutes_sherpa_destroy(handle: *mut std::ffi::c_void) {
    if handle.is_null() {
        return;
    }
    // A panic in the model's own teardown must not unwind into the host.
    let _ = catch_unwind(AssertUnwindSafe(|| {
        drop(Box::from_raw(handle as *mut Recognizer));
    }));
}
