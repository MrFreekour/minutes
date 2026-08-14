#[cfg(not(feature = "engine-sherpa"))]
fn main() {
    eprintln!("dictation_partial_benchmark requires --features engine-sherpa,streaming");
    std::process::exit(2);
}

#[cfg(feature = "engine-sherpa")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use minutes_core::config::Config;
    use minutes_core::partial_quality::{
        PartialQualityGate, TARGET_FIRST_USEFUL_PARTIAL_MS, TARGET_USEFUL_CADENCE_MAX_MS,
    };
    use minutes_core::sherpa_plugin::PluginStreamingRecognizer;
    use serde::{Deserialize, Serialize};
    use std::path::{Path, PathBuf};
    use std::time::Instant;

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Case {
        id: String,
        audio_path: PathBuf,
        reference_text: String,
        #[serde(default)]
        required_terms: Vec<String>,
        #[serde(default)]
        forbidden_terms: Vec<String>,
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct CaseReport {
        id: String,
        iterations: usize,
        audio_duration_ms: u64,
        p95_first_useful_partial_ms: Option<u64>,
        p95_useful_cadence_ms: Option<u64>,
        p95_decode_ms: Option<u64>,
        decode_realtime_factor: f64,
        stable_prefix_regressions: u32,
        wer: f64,
        punctuation_insensitive_wer: f64,
        required_terms_missing: Vec<String>,
        forbidden_terms_found: Vec<String>,
        transcript: String,
        passed: bool,
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Report {
        schema_version: u32,
        engine: &'static str,
        chunk_ms: u64,
        thresholds: serde_json::Value,
        cases: Vec<CaseReport>,
        passed: bool,
    }

    fn percentile(values: &[u64], p: usize) -> Option<u64> {
        if values.is_empty() {
            return None;
        }
        let mut values = values.to_vec();
        values.sort_unstable();
        values.get(((values.len() - 1) * p).div_ceil(100)).copied()
    }

    fn normalize_words(text: &str, punctuation: bool) -> Vec<String> {
        text.split_whitespace()
            .map(|word| {
                let word = word.to_lowercase();
                if punctuation {
                    word
                } else {
                    word.chars()
                        .filter(|ch| ch.is_alphanumeric() || *ch == '\'')
                        .collect()
                }
            })
            .filter(|word| !word.is_empty())
            .collect()
    }

    fn word_error_rate(reference: &str, hypothesis: &str, punctuation: bool) -> f64 {
        let reference = normalize_words(reference, punctuation);
        let hypothesis = normalize_words(hypothesis, punctuation);
        let mut previous: Vec<usize> = (0..=hypothesis.len()).collect();
        for (i, expected) in reference.iter().enumerate() {
            let mut current = vec![i + 1];
            for (j, actual) in hypothesis.iter().enumerate() {
                current.push(if expected == actual {
                    previous[j]
                } else {
                    1 + previous[j].min(previous[j + 1]).min(current[j])
                });
            }
            previous = current;
        }
        previous[hypothesis.len()] as f64 / reference.len().max(1) as f64
    }

    fn read_wav(path: &Path) -> Result<(u32, Vec<f32>), Box<dyn std::error::Error>> {
        let mut reader = hound::WavReader::open(path)?;
        let spec = reader.spec();
        if spec.channels != 1 {
            return Err(format!("{} is not mono", path.display()).into());
        }
        let samples = match spec.sample_format {
            hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<Vec<_>, _>>()?,
            hound::SampleFormat::Int if spec.bits_per_sample <= 16 => reader
                .samples::<i16>()
                .map(|sample| sample.map(|sample| sample as f32 / i16::MAX as f32))
                .collect::<Result<Vec<_>, _>>()?,
            _ => return Err(format!("{} has an unsupported WAV format", path.display()).into()),
        };
        Ok((spec.sample_rate, samples))
    }

    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let corpus_arg = args
        .first()
        .ok_or("usage: dictation_partial_benchmark CORPUS.json [ITERATIONS]")?;
    let iterations = args
        .get(1)
        .map(|value| value.parse::<usize>())
        .transpose()?
        .unwrap_or(10)
        .max(1);
    let corpus_path = PathBuf::from(corpus_arg);
    let corpus_root = corpus_path.parent().unwrap_or_else(|| Path::new("."));
    let cases: Vec<Case> = serde_json::from_slice(&std::fs::read(&corpus_path)?)?;
    let config = Config::load();
    let model_dir = minutes_core::sherpa_engine::streaming_model_dir(&config);
    let mut recognizer = PluginStreamingRecognizer::new(&model_dir, &config)?;
    let mut reports = Vec::new();

    for case in cases {
        let path = if case.audio_path.is_absolute() {
            case.audio_path.clone()
        } else {
            corpus_root.join(&case.audio_path)
        };
        let (sample_rate, samples) = read_wav(&path)?;
        let chunk_samples = (sample_rate as usize / 10).max(1);
        let duration_ms = samples.len() as u64 * 1000 / sample_rate as u64;
        let mut first_partials = Vec::new();
        let mut cadences = Vec::new();
        let mut decodes = Vec::new();
        let mut total_decode_ms = 0u64;
        let mut regressions = 0u32;
        let mut final_text = String::new();

        for _ in 0..iterations {
            let mut gate = PartialQualityGate::default();
            let mut decoder_available_ms = 0u64;
            for (index, chunk) in samples.chunks(chunk_samples).enumerate() {
                let arrival_ms = ((index + 1) * chunk_samples) as u64 * 1000 / sample_rate as u64;
                let started = Instant::now();
                let hypothesis = recognizer.feed(sample_rate, chunk)?;
                let decode_ms = started.elapsed().as_millis() as u64;
                total_decode_ms = total_decode_ms.saturating_add(decode_ms);
                decodes.push(decode_ms);
                decoder_available_ms = decoder_available_ms
                    .max(arrival_ms)
                    .saturating_add(decode_ms);
                if !hypothesis.trim().is_empty() {
                    let _ = gate.observe(&hypothesis, decoder_available_ms, decode_ms);
                }
            }
            final_text = recognizer.finish()?;
            let metrics = gate.metrics();
            if let Some(value) = metrics.first_useful_partial_ms {
                first_partials.push(value);
            }
            if let Some(value) = metrics.p95_update_cadence_ms {
                cadences.push(value);
            }
            regressions = regressions.saturating_add(metrics.stable_prefix_regressions);
            recognizer.reset()?;
        }

        let required_terms_missing = case
            .required_terms
            .into_iter()
            .filter(|term| !final_text.to_lowercase().contains(&term.to_lowercase()))
            .collect::<Vec<_>>();
        let forbidden_terms_found = case
            .forbidden_terms
            .into_iter()
            .filter(|term| final_text.to_lowercase().contains(&term.to_lowercase()))
            .collect::<Vec<_>>();
        let wer = word_error_rate(&case.reference_text, &final_text, true);
        let punctuation_insensitive_wer = word_error_rate(&case.reference_text, &final_text, false);
        let p95_first = percentile(&first_partials, 95);
        let p95_cadence = percentile(&cadences, 95);
        let p95_decode = percentile(&decodes, 95);
        let decode_realtime_factor =
            total_decode_ms as f64 / (duration_ms * iterations as u64).max(1) as f64;
        let passed = p95_first.is_some_and(|value| value < TARGET_FIRST_USEFUL_PARTIAL_MS)
            && p95_cadence.is_some_and(|value| value <= TARGET_USEFUL_CADENCE_MAX_MS)
            && decode_realtime_factor <= 0.5
            && punctuation_insensitive_wer <= 0.30
            && required_terms_missing.is_empty()
            && forbidden_terms_found.is_empty();
        reports.push(CaseReport {
            id: case.id,
            iterations,
            audio_duration_ms: duration_ms,
            p95_first_useful_partial_ms: p95_first,
            p95_useful_cadence_ms: p95_cadence,
            p95_decode_ms: p95_decode,
            decode_realtime_factor,
            stable_prefix_regressions: regressions,
            wer,
            punctuation_insensitive_wer,
            required_terms_missing,
            forbidden_terms_found,
            transcript: final_text,
            passed,
        });
    }

    let passed = !reports.is_empty() && reports.iter().all(|report| report.passed);
    let report = Report {
        schema_version: 1,
        engine: "sherpa-online-zipformer-en-20m-2023-02-17",
        chunk_ms: 100,
        thresholds: serde_json::json!({
            "p95FirstUsefulPartialMs": TARGET_FIRST_USEFUL_PARTIAL_MS,
            "p95UsefulCadenceMs": TARGET_USEFUL_CADENCE_MAX_MS,
            "maxDecodeRealtimeFactor": 0.5,
            "maxPunctuationInsensitiveWer": 0.30,
        }),
        cases: reports,
        passed,
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    if !passed {
        std::process::exit(1);
    }
    Ok(())
}
