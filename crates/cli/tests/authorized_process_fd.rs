#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn minimal_wav() -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&38_u32.to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&16_000_u32.to_le_bytes());
    bytes.extend_from_slice(&32_000_u32.to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&2_u32.to_le_bytes());
    bytes.extend_from_slice(&0_i16.to_le_bytes());
    bytes
}

#[test]
fn authorized_process_accepts_an_ordinary_0644_source_end_to_end() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "minutes-cli-authorized-fd-{}-{nonce}",
        std::process::id()
    ));
    let home = root.join("home");
    let config_dir = root.join("config").join("minutes");
    let meetings = root.join("meetings");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&config_dir).unwrap();
    fs::create_dir_all(&meetings).unwrap();
    let source = root.join("ordinary.wav");
    fs::write(&source, minimal_wav()).unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o644)).unwrap();
    fs::write(
        config_dir.join("config.toml"),
        format!(
            "output_dir = {:?}\n\n[transcription]\nengine = \"whisper\"\nmodel = \"definitely-missing-model\"\n",
            meetings
        ),
    )
    .unwrap();

    let binary = env!("CARGO_BIN_EXE_minutes");
    let script = "exec 4< <(sleep 30); supervisor=$!; MINUTES_MCP_OUTER_PROCESS_GROUP=$$ \"$MINUTES_TEST_BIN\" process authorized-input.wav -t memo --authorized-input-fd 3 --authorized-input-bytes \"$MINUTES_TEST_BYTES\" --authorized-input-format wav 3<\"$MINUTES_TEST_SOURCE\"; status=$?; kill \"$supervisor\" 2>/dev/null || true; wait \"$supervisor\" 2>/dev/null || true; exit $status";
    let mut command = Command::new("bash");
    command
        .arg("-c")
        .arg(script)
        .env("MINUTES_TEST_BIN", binary)
        .env("MINUTES_TEST_SOURCE", &source)
        .env(
            "MINUTES_TEST_BYTES",
            fs::metadata(&source).unwrap().len().to_string(),
        )
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", root.join("config"))
        .process_group(0);
    let output = command.output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();

    assert!(
        !output.status.success(),
        "model-free regression must stop at the configured downstream model check"
    );
    assert!(
        stderr.contains("transcription model not found"),
        "ordinary 0644 input must cross authorization and reach transcription: {stderr}"
    );
    for forbidden in [
        "authorized process input",
        "outer process containment",
        "caller-owned single-link",
        "owner-private",
    ] {
        assert!(
            !stderr.contains(forbidden),
            "unexpected boundary failure: {stderr}"
        );
    }

    fs::remove_dir_all(root).ok();
}

#[test]
fn authorized_process_rejects_marker_without_supervisor_capability() {
    let root = std::env::temp_dir().join(format!(
        "minutes-cli-authorized-no-supervisor-{}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    let source = root.join("ordinary.wav");
    fs::write(&source, minimal_wav()).unwrap();
    let binary = env!("CARGO_BIN_EXE_minutes");
    let script = "MINUTES_MCP_OUTER_PROCESS_GROUP=$$ \"$MINUTES_TEST_BIN\" process authorized-input.wav -t memo --authorized-input-fd 3 --authorized-input-bytes \"$MINUTES_TEST_BYTES\" --authorized-input-format wav 3<\"$MINUTES_TEST_SOURCE\" 4<&-";
    let output = Command::new("sh")
        .arg("-c")
        .arg(script)
        .env("MINUTES_TEST_BIN", binary)
        .env("MINUTES_TEST_SOURCE", &source)
        .env(
            "MINUTES_TEST_BYTES",
            fs::metadata(&source).unwrap().len().to_string(),
        )
        .process_group(0)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(
        stderr.contains("outer process supervisor capability was unavailable"),
        "unexpected missing-capability failure: {stderr}"
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn authorized_process_rejects_invalid_outer_group_topology() {
    let root = std::env::temp_dir().join(format!(
        "minutes-cli-authorized-invalid-topology-{}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    let source = root.join("ordinary.wav");
    fs::write(&source, minimal_wav()).unwrap();
    let binary = env!("CARGO_BIN_EXE_minutes");
    let script = "exec 4< <(sleep 30); supervisor=$!; MINUTES_MCP_OUTER_PROCESS_GROUP=$(( $$ + 1 )) \"$MINUTES_TEST_BIN\" process authorized-input.wav -t memo --authorized-input-fd 3 --authorized-input-bytes \"$MINUTES_TEST_BYTES\" --authorized-input-format wav 3<\"$MINUTES_TEST_SOURCE\"; status=$?; kill \"$supervisor\" 2>/dev/null || true; wait \"$supervisor\" 2>/dev/null || true; exit $status";
    let output = Command::new("bash")
        .arg("-c")
        .arg(script)
        .env("MINUTES_TEST_BIN", binary)
        .env("MINUTES_TEST_SOURCE", &source)
        .env(
            "MINUTES_TEST_BYTES",
            fs::metadata(&source).unwrap().len().to_string(),
        )
        .process_group(0)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(stderr.contains("outer process containment topology was not verified"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn ambient_outer_group_marker_without_authorized_fd_fails_closed() {
    let output = Command::new(env!("CARGO_BIN_EXE_minutes"))
        .arg("status")
        .env("MINUTES_MCP_OUTER_PROCESS_GROUP", "12345")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(stderr.contains("outer process containment requires authorized input"));
}
