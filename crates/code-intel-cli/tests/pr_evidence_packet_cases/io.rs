use std::fs;
use std::io::Write;
use std::process::Stdio;

use super::super::{assert_success, fixture, run_request, Temp};

#[test]
fn malformed_or_duplicate_claim_evidence_is_refused_without_an_output_file() {
    let temp = Temp::new();
    let mut request = fixture("review-ready.request.json");
    let duplicate_claim = request["claims"][0].clone();
    request["claims"]
        .as_array_mut()
        .unwrap()
        .push(duplicate_claim);
    let (output, path) = run_request(&temp.0, "duplicate-id", &request);
    assert_eq!(output.status.code(), Some(65));
    assert!(!path.exists());
    assert!(String::from_utf8_lossy(&output.stderr).contains("repeats id"));

    let request_path = temp.0.join("duplicate-key.request.json");
    let output_path = temp.0.join("duplicate-key.packet.json");
    fs::write(
        &request_path,
        r#"{"schema":"code-intel-pr-evidence-request.v1","schema":"code-intel-pr-evidence-request.v1","subject":{},"claims":[]}"#,
    )
    .unwrap();
    let duplicate = super::super::common::cli()
        .args(["pr", "evidence", "--request"])
        .arg(&request_path)
        .arg("--out")
        .arg(&output_path)
        .output()
        .unwrap();
    assert_eq!(duplicate.status.code(), Some(65));
    assert!(!output_path.exists());
    assert!(String::from_utf8_lossy(&duplicate.stderr).contains("duplicate JSON object key"));
}

#[test]
fn packet_bytes_are_deterministic_when_claim_input_order_changes() {
    let temp = Temp::new();
    let original = fixture("review-ready.request.json");
    let mut reversed = original.clone();
    reversed["claims"].as_array_mut().unwrap().reverse();

    let (first_output, first_path) = run_request(&temp.0, "first", &original);
    let (second_output, second_path) = run_request(&temp.0, "second", &reversed);
    let first = assert_success(&first_output);
    let second = assert_success(&second_output);
    assert_eq!(first["packetId"], second["packetId"]);
    assert_eq!(
        fs::read(first_path).unwrap(),
        fs::read(second_path).unwrap()
    );
}

#[test]
fn stdin_request_writes_the_same_packet_to_stdout_and_out() {
    let temp = Temp::new();
    let output_path = temp.0.join("stdin.packet.json");
    let mut child = super::super::common::cli()
        .args(["pr", "evidence", "--request", "-", "--out"])
        .arg(&output_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin
        .write_all(&serde_json::to_vec(&fixture("review-ready.request.json")).unwrap())
        .unwrap();
    drop(stdin);

    let output = child.wait_with_output().unwrap();
    let packet = assert_success(&output);
    assert_eq!(packet["decision"]["state"], "ready_for_human_merge_review");
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        String::from_utf8(output.stdout).unwrap().trim_end()
    );
}

#[test]
fn bare_output_file_does_not_require_a_parent_directory() {
    let temp = Temp::new();
    let request_path = temp.0.join("request.json");
    fs::write(
        &request_path,
        serde_json::to_vec(&fixture("review-ready.request.json")).unwrap(),
    )
    .unwrap();

    let output = super::super::common::cli()
        .current_dir(&temp.0)
        .args(["pr", "evidence", "--request"])
        .arg(&request_path)
        .args(["--out", "packet.json"])
        .output()
        .unwrap();
    let packet = assert_success(&output);
    assert_eq!(packet["decision"]["state"], "ready_for_human_merge_review");
    assert!(temp.0.join("packet.json").is_file());
}
