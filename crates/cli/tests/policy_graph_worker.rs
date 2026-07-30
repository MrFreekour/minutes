use std::fs;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(windows)]
struct InheritedHandleCanary(std::fs::File);

#[cfg(windows)]
impl InheritedHandleCanary {
    fn install(path: &std::path::Path) -> Self {
        use std::os::windows::io::AsRawHandle;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn SetHandleInformation(handle: *mut std::ffi::c_void, mask: u32, flags: u32) -> i32;
        }
        const HANDLE_FLAG_INHERIT: u32 = 1;

        let file = std::fs::File::create(path).unwrap();
        assert_ne!(
            unsafe {
                SetHandleInformation(
                    file.as_raw_handle(),
                    HANDLE_FLAG_INHERIT,
                    HANDLE_FLAG_INHERIT,
                )
            },
            0
        );
        unsafe {
            std::env::set_var(
                "MINUTES_INTERNAL_TEST_GRAPH_INHERITED_HANDLE",
                (file.as_raw_handle() as usize).to_string(),
            );
        }
        Self(file)
    }
}

#[cfg(windows)]
impl Drop for InheritedHandleCanary {
    fn drop(&mut self) {
        use std::os::windows::io::AsRawHandle;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn SetHandleInformation(handle: *mut std::ffi::c_void, mask: u32, flags: u32) -> i32;
        }
        const HANDLE_FLAG_INHERIT: u32 = 1;
        unsafe {
            std::env::remove_var("MINUTES_INTERNAL_TEST_GRAPH_INHERITED_HANDLE");
            let _ = SetHandleInformation(self.0.as_raw_handle(), HANDLE_FLAG_INHERIT, 0);
        }
    }
}

fn run_minutes(root: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_minutes"))
        .args(args)
        .env("HOME", root.join("home"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("MINUTES_HOME", root.join("state"))
        .output()
        .unwrap()
}

fn run_policy_minutes(root: &std::path::Path, corpus: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_minutes"))
        .args(args)
        .env("HOME", root.join("home"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("MINUTES_HOME", root.join("state"))
        .env("MINUTES_CLI_RESTRICTED_POLICY", "deny")
        .env("MINUTES_POLICY_GRAPH_CORPUS_ROOT", corpus)
        .output()
        .unwrap()
}

#[cfg(target_os = "macos")]
#[test]
fn dedicated_macos_xpc_worker_refuses_direct_stream_execution() {
    let output = Command::new(env!("CARGO_BIN_EXE_minutes-graph-worker"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        output.stdout.is_empty(),
        "direct execution must not expose the removed stdin/stdout worker protocol"
    );
}

#[test]
fn graph_worker_projects_map_profile_and_commitments_from_one_policy_boundary() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "minutes-cli-policy-graph-worker-{}-{nonce}",
        std::process::id()
    ));
    let home = root.join("home");
    let state = root.join("state");
    let meetings = root.join("meetings");
    let config_dir = root.join("config/minutes");
    for directory in [&home, &state, &meetings, &config_dir] {
        fs::create_dir_all(directory).unwrap();
    }
    #[cfg(windows)]
    let _inherited_handle_canary =
        InheritedHandleCanary::install(&root.join("ambient-handle-canary.txt"));
    fs::write(
        config_dir.join("config.toml"),
        format!("output_dir = {:?}\n", meetings),
    )
    .unwrap();
    fs::write(
        state.join("vocabulary.toml"),
        r#"[[entries]]
kind = "person"
canonical = "Avery Stone"
aliases = ["Avery", "A. Stone"]
"#,
    )
    .unwrap();
    fs::write(
        meetings.join("normal.md"),
        r#"---
title: Synthetic Product Review
type: meeting
date: 2026-07-20T12:00:00Z
duration: 30m
status: complete
attendees: [Avery Stone]
tags: [roadmap]
action_items:
  - assignee: Avery Stone
    task: Publish the synthetic follow-up
    due: "2026-07-25"
    status: open
decisions:
  - text: Use the synthetic rollout plan
    topic: roadmap
    authority: high
---

## Transcript
[Avery Stone 0:00] We will publish the synthetic follow-up.
"#,
    )
    .unwrap();
    fs::write(
        meetings.join("restricted.md"),
        r#"---
title: Restricted Canary
type: meeting
date: 2026-07-21T12:00:00Z
duration: 10m
status: complete
sensitivity: restricted
attendees: [Private Canary]
action_items:
  - assignee: Private Canary
    task: RESTRICTED_GRAPH_WORKER_CANARY
    status: open
---

## Transcript
[Private Canary 0:00] RESTRICTED_GRAPH_WORKER_CANARY
"#,
    )
    .unwrap();

    let people = run_minutes(&root, &["people", "--json", "--limit", "15"]);
    assert!(
        people.status.success(),
        "people failed: {}",
        String::from_utf8_lossy(&people.stderr)
    );
    let people_json: serde_json::Value = serde_json::from_slice(&people.stdout).unwrap();
    let people_text = people_json.to_string();
    assert!(people_text.contains("Avery Stone"));
    assert!(!people_text.contains("Private Canary"));
    assert!(!people_text.contains("RESTRICTED_GRAPH_WORKER_CANARY"));

    let rebuilt = run_minutes(&root, &["people", "--rebuild", "--json", "--limit", "15"]);
    assert!(
        rebuilt.status.success(),
        "supervised rebuild failed: {}",
        String::from_utf8_lossy(&rebuilt.stderr)
    );
    assert!(!String::from_utf8_lossy(&rebuilt.stdout).contains("RESTRICTED_GRAPH_WORKER_CANARY"));

    let profile = run_minutes(&root, &["person", "A. Stone"]);
    assert!(
        profile.status.success(),
        "profile failed: {}",
        String::from_utf8_lossy(&profile.stderr)
    );
    let profile_json: serde_json::Value = serde_json::from_slice(&profile.stdout).unwrap();
    assert_eq!(profile_json["name"], "Avery Stone");
    assert_eq!(profile_json["recent_meetings"].as_array().unwrap().len(), 1);
    assert_eq!(profile_json["open_intents"].as_array().unwrap().len(), 1);
    assert_eq!(
        profile_json["recent_decisions"].as_array().unwrap().len(),
        1
    );
    assert!(!profile_json
        .to_string()
        .contains("RESTRICTED_GRAPH_WORKER_CANARY"));

    let commitments = run_minutes(&root, &["commitments", "--json", "--person", "Avery"]);
    assert!(
        commitments.status.success(),
        "commitments failed: {}",
        String::from_utf8_lossy(&commitments.stderr)
    );
    let commitment_json: serde_json::Value = serde_json::from_slice(&commitments.stdout).unwrap();
    assert_eq!(commitment_json.as_array().unwrap().len(), 1);
    assert!(commitment_json.to_string().contains("synthetic follow-up"));
    assert!(!commitment_json
        .to_string()
        .contains("RESTRICTED_GRAPH_WORKER_CANARY"));

    // Limits are enforced inside the supervised worker, before stdout is
    // serialized, so the MCP's 50-row contract cannot turn a 51st commitment
    // into a total response failure.
    let many_actions = (0..51)
        .map(|index| {
            format!(
                "  - assignee: Avery Stone\n    task: Synthetic bounded task {index:02}\n    status: open"
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(
        meetings.join("many-actions.md"),
        format!(
            "---\ntitle: Synthetic Bounded Actions\ntype: meeting\ndate: 2026-07-22T12:00:00Z\nstatus: complete\nattendees: [Avery Stone]\naction_items:\n{many_actions}\n---\n"
        ),
    )
    .unwrap();
    let bounded = run_minutes(
        &root,
        &[
            "commitments",
            "--json",
            "--person",
            "Avery",
            "--limit",
            "50",
        ],
    );
    assert!(
        bounded.status.success(),
        "bounded commitments failed: {}",
        String::from_utf8_lossy(&bounded.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&bounded.stdout)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        50
    );

    let many_people = (0..30)
        .map(|index| format!("Synthetic Person {index:02}"))
        .collect::<Vec<_>>();
    fs::write(
        meetings.join("many-people.md"),
        format!(
            "---\ntitle: Synthetic Bounded People\ntype: meeting\ndate: 2026-07-23T12:00:00Z\nstatus: complete\nattendees: [{}]\n---\n",
            many_people.join(", ")
        ),
    )
    .unwrap();
    let bounded_people = run_minutes(&root, &["people", "--json", "--limit", "15"]);
    assert!(
        bounded_people.status.success(),
        "bounded people failed: {}",
        String::from_utf8_lossy(&bounded_people.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&bounded_people.stdout)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        15
    );

    // An agent-provided corpus capability must dominate the user's configured
    // root for every graph surface. Otherwise MCP could disclose a real corpus
    // while claiming to answer from its isolated/overridden meetings root.
    let decoy = root.join("configured-decoy");
    fs::create_dir_all(&decoy).unwrap();
    fs::write(
        decoy.join("decoy.md"),
        r#"---
title: Configured Decoy
type: meeting
date: 2026-07-22T13:00:00Z
status: complete
attendees: [Configured Decoy Person]
action_items:
  - assignee: Configured Decoy Person
    task: CONFIGURED_ROOT_CANARY
    status: open
---
"#,
    )
    .unwrap();
    fs::write(
        config_dir.join("config.toml"),
        format!("output_dir = {:?}\n", decoy),
    )
    .unwrap();
    for args in [
        vec!["people", "--json", "--limit", "15"],
        vec!["person", "Avery Stone"],
        vec![
            "commitments",
            "--json",
            "--person",
            "Avery",
            "--limit",
            "50",
        ],
    ] {
        let result = run_policy_minutes(&root, &meetings, &args);
        assert!(
            result.status.success(),
            "policy-root command failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let output = String::from_utf8_lossy(&result.stdout);
        assert!(output.contains("Avery"));
        assert!(!output.contains("Configured Decoy"));
        assert!(!output.contains("CONFIGURED_ROOT_CANARY"));
    }

    fs::remove_dir_all(root).ok();
}
