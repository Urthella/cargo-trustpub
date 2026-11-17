//! Runs the binary against a fake crates.io that serves real records saved by
//! scripts/fetch-fixtures.sh, with lockfiles built from those records' checksums.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

fn record(name: &str) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(format!("{FIXTURES}/{name}.json")).unwrap())
        .unwrap()
}

/// crates.io's checksum for a version, from the saved record.
fn checksum(name: &str, version: &str) -> String {
    let record = record(name);
    let v = record["versions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["num"] == version)
        .unwrap_or_else(|| panic!("{name} {version}"));
    v["checksum"].as_str().unwrap().to_string()
}

/// The newest version the record has.
fn newest(name: &str) -> String {
    let record = record(name);
    let versions = record["versions"].as_array().unwrap();
    versions
        .iter()
        .max_by_key(|v| v["created_at"].as_str().unwrap().to_string())
        .unwrap()["num"]
        .as_str()
        .unwrap()
        .to_string()
}

struct FakeRegistry {
    url: String,
    agents: Arc<Mutex<Vec<String>>>,
    times: Arc<Mutex<Vec<Instant>>>,
    limited: Arc<AtomicUsize>,
}

/// Serves /api/v1/crates/NAME from the fixtures, 404 for anything else, and a 429 for the next
/// `limited` requests.
fn registry() -> FakeRegistry {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/api/v1", listener.local_addr().unwrap());
    let agents = Arc::new(Mutex::new(Vec::new()));
    let times = Arc::new(Mutex::new(Vec::new()));
    let limited = Arc::new(AtomicUsize::new(0));
    let (a, t, l) = (agents.clone(), times.clone(), limited.clone());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request = String::new();
            reader.read_line(&mut request).unwrap();
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).unwrap();
                if let Some(agent) = header
                    .strip_prefix("user-agent: ")
                    .or_else(|| header.strip_prefix("User-Agent: "))
                {
                    a.lock().unwrap().push(agent.trim().to_string());
                }
                if header.trim().is_empty() {
                    break;
                }
            }
            t.lock().unwrap().push(Instant::now());
            let path = request.split_whitespace().nth(1).unwrap_or("").to_string();
            let (status, body) = if l.load(Ordering::SeqCst) > 0 {
                l.fetch_sub(1, Ordering::SeqCst);
                (
                    "429 Too Many Requests\r\nRetry-After: 0",
                    "slow down".to_string(),
                )
            } else {
                match path.strip_prefix("/api/v1/crates/").and_then(|name| {
                    std::fs::read_to_string(format!("{FIXTURES}/{name}.json")).ok()
                }) {
                    Some(body) => ("200 OK", body),
                    None => (
                        "404 Not Found",
                        r#"{"errors":[{"detail":"Not Found"}]}"#.to_string(),
                    ),
                }
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    FakeRegistry {
        url,
        agents,
        times,
        limited,
    }
}

fn lockfile(packages: &[(&str, &str, Option<String>)]) -> tempfile_path::Path {
    let mut text =
        String::from("version = 4\n\n[[package]]\nname = \"mine\"\nversion = \"0.1.0\"\n");
    for (name, version, checksum) in packages {
        text.push_str(&format!("\n[[package]]\nname = \"{name}\"\nversion = \"{version}\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n"));
        if let Some(c) = checksum {
            text.push_str(&format!("checksum = \"{c}\"\n"));
        }
    }
    tempfile_path::Path::with(&text)
}

/// A file in the system's temporary directory, removed when dropped.
mod tempfile_path {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    pub struct Path(pub std::path::PathBuf);
    impl Path {
        pub fn with(text: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "cargo-trustpub-{}-{}.lock",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::SeqCst)
            ));
            std::fs::write(&path, text).unwrap();
            Path(path)
        }
    }
    impl Drop for Path {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
}

fn run(registry: &FakeRegistry, lock: &tempfile_path::Path, extra: &[&str]) -> (i32, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-trustpub"))
        .args(["--api", &registry.url, "--interval", "0", "--lockfile"])
        .arg(&lock.0)
        .args(extra)
        .output()
        .unwrap();
    (
        output.status.code().unwrap(),
        String::from_utf8_lossy(&output.stdout).to_string()
            + &String::from_utf8_lossy(&output.stderr),
    )
}

#[test]
fn rand_going_back_to_a_token_after_trusted_publishing_fails() {
    let registry = registry();
    let lock = lockfile(&[
        ("rand", "0.10.0", Some(checksum("rand", "0.10.0"))),
        (
            "itoa",
            &newest("itoa"),
            Some(checksum("itoa", &newest("itoa"))),
        ),
    ]);
    let (code, out) = run(&registry, &lock, &[]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("2 crates.io packages in Cargo.lock: 0 published with Trusted Publishing, 2 with a token"), "{out}");
    assert!(
        out.contains("FAIL  rand 0.10.0 was published 2026-02-08 with a token by dhardy, after rand had used Trusted Publishing since 0.10.0-rc.8 (2026-01-26, from github rust-random/rand)"),
        "{out}"
    );
    assert!(out.contains("checksums: 2 of 2 match crates.io"), "{out}");
    assert!(
        !out.contains("FAIL  itoa"),
        "itoa has never used Trusted Publishing: {out}"
    );
}

#[test]
fn trusted_versions_and_steady_tokens_pass() {
    let registry = registry();
    let (jiff, rand, itoa) = (newest("jiff"), newest("rand"), newest("itoa"));
    let lock = lockfile(&[
        ("jiff", &jiff, Some(checksum("jiff", &jiff))),
        ("rand", &rand, Some(checksum("rand", &rand))),
        ("itoa", &itoa, None),
    ]);
    let (code, out) = run(&registry, &lock, &["--all"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("3 crates.io packages in Cargo.lock: 2 published with Trusted Publishing, 1 with a token"), "{out}");
    assert!(
        out.contains("checksums: 2 of 2 match crates.io"),
        "a lockfile entry without a checksum isn't counted: {out}"
    );
    let record = record("jiff");
    let data = &record["versions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["num"] == jiff.as_str())
        .unwrap()["trustpub_data"];
    let (sha, run_id) = (
        data["sha"].as_str().unwrap(),
        data["run_id"].as_str().unwrap(),
    );
    assert!(
        out.contains(&format!(
            "trusted  jiff {jiff}  github BurntSushi/jiff@{}, run {run_id}\n",
            &sha[..7]
        )),
        "the commit, shortened to 7 characters, and the run: {out}"
    );
    assert!(
        out.contains(&format!("token    itoa {itoa}  by dtolnay")),
        "{out}"
    );
}

#[test]
fn json_has_every_entry_with_how_it_was_published() {
    let registry = registry();
    let lock = lockfile(&[("rand", "0.10.0", None), ("rand", "0.10.4", None)]);
    let (code, out) = run(&registry, &lock, &["--json"]);
    assert_eq!(code, 1, "{out}");
    let entries: Vec<serde_json::Value> = serde_json::from_str(&out).unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["version"], "0.10.0");
    assert_eq!(entries[0]["how"]["kind"], "token");
    assert_eq!(entries[0]["back_to_token"]["version"], "0.10.0-rc.8");
    assert_eq!(entries[1]["how"]["kind"], "trusted_publishing");
    assert_eq!(entries[1]["how"]["data"]["repository"], "rust-random/rand");
    assert_eq!(
        registry.times.lock().unwrap().len(),
        1,
        "two versions of one crate share a request"
    );
}

#[test]
fn a_tampered_checksum_fails() {
    let registry = registry();
    let lock = lockfile(&[("jiff", "1.0.0", Some("0".repeat(64)))]);
    let (code, out) = run(&registry, &lock, &[]);
    assert_eq!(code, 1, "{out}");
    assert!(
        out.contains("FAIL  jiff 1.0.0: Cargo.lock's checksum isn't the one crates.io has"),
        "{out}"
    );
    assert!(out.contains("checksums: 0 of 1 match crates.io"), "{out}");
}

#[test]
fn a_crate_or_version_crates_io_doesnt_have_is_an_error() {
    let registry = registry();
    let lock = lockfile(&[("no-such-crate", "1.0.0", None), ("jiff", "99.0.0", None)]);
    let (code, out) = run(&registry, &lock, &[]);
    assert_eq!(code, 2, "{out}");
    assert!(
        out.contains("ERROR no-such-crate 1.0.0: no-such-crate isn't on crates.io"),
        "{out}"
    );
    assert!(
        out.contains("ERROR jiff 99.0.0: jiff 99.0.0 isn't on crates.io"),
        "{out}"
    );
}

#[test]
fn a_rate_limit_is_waited_out_and_the_contact_is_sent() {
    let registry = registry();
    registry.limited.store(2, Ordering::SeqCst);
    let lock = lockfile(&[("jiff", "1.0.0", None)]);
    let (code, out) = run(&registry, &lock, &["--contact", "ops@example.com"]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(registry.times.lock().unwrap().len(), 3, "asked again twice");
    let agents = registry.agents.lock().unwrap();
    assert!(
        agents
            .iter()
            .all(|a| a.starts_with("cargo-trustpub/") && a.ends_with("(ops@example.com)")),
        "{agents:?}"
    );
}

#[test]
fn requests_are_spaced_by_the_interval() {
    let registry = registry();
    let lock = lockfile(&[
        ("jiff", "1.0.0", None),
        ("itoa", &newest("itoa"), None),
        ("rand", "0.10.4", None),
    ]);
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-trustpub"))
        .args(["--api", &registry.url, "--interval", "300", "--lockfile"])
        .arg(&lock.0)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let times = registry.times.lock().unwrap();
    assert_eq!(times.len(), 3);
    for pair in times.windows(2) {
        assert!(
            pair[1] - pair[0] >= Duration::from_millis(290),
            "{:?}",
            pair[1] - pair[0]
        );
    }
    let agents = registry.agents.lock().unwrap();
    assert_eq!(
        agents[0],
        format!("cargo-trustpub/{}", env!("CARGO_PKG_VERSION")),
        "no contact unless given"
    );
}

#[test]
fn runs_as_a_cargo_subcommand_and_rejects_bad_arguments() {
    let registry = registry();
    let lock = lockfile(&[("jiff", "1.0.0", None)]);
    let as_subcommand = Command::new(env!("CARGO_BIN_EXE_cargo-trustpub"))
        .args([
            "trustpub",
            "--api",
            &registry.url,
            "--interval",
            "0",
            "--lockfile",
        ])
        .arg(&lock.0)
        .output()
        .unwrap();
    assert_eq!(
        as_subcommand.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&as_subcommand.stderr)
    );
    for args in [
        vec!["--nope"],
        vec!["--interval", "soon"],
        vec!["--lockfile"],
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_cargo-trustpub"))
            .args(&args)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("usage: cargo trustpub"),
            "{args:?}"
        );
    }
    let missing = Command::new(env!("CARGO_BIN_EXE_cargo-trustpub"))
        .args(["--lockfile", "no/such/Cargo.lock"])
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&missing.stderr).starts_with("cargo-trustpub: no/such/Cargo.lock")
    );
}
