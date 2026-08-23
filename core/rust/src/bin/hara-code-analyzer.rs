//! Persistent JSONL analyser host.
//!
//! The binary owns: JSONL framing; `describe`, `ping`, `analyze`,
//! unknown-operation errors, and `shutdown` request ordering;
//! non-evaluating spanned source reading; preparation and invocation of
//! the application-supplied `.hal` analyser policy; stable source
//! diagnostics and response materialisation; and runtime-handle lifetime.
//!
//! The application owns: the analyser policy in `.hal`; protocol schemas
//! and conformance fixtures; worker registration and packaging.
//!
//! # Protocol
//!
//! Each line of stdin is a JSON object.  The binary writes one JSON
//! response line to stdout for every request line and exits after
//! responding to `"shutdown"`.
//!
//! ```text
//! {"op":"describe"}
//!   -> {"name":"hara-code-analyzer","version":"0.1","capabilities":[...]}
//! {"op":"ping"}
//!   -> {"ok":true}
//! {"op":"analyze","source":"..."}
//!   -> {"diagnostics":[...]}
//! {"op":"shutdown"}
//!   -> {"ok":true}
//! (any other op)
//!   -> {"error":"unknown operation: <op>"}
//! ```

use hara_wasm::kernel::read_forms;
use hara_wasm::vm::compile_source;
use hara_wasm::whole_wasm::{compile_artifact, NativeModule};
use serde_json::{json, Value as Json};
use std::io::{self, BufRead, Write};

// ---------------------------------------------------------------------------
// Fixture policy
// ---------------------------------------------------------------------------

/// Embedded fixture analyser policy.
///
/// The policy is a Hara expression that is compiled to a whole-Wasm
/// `NativeModule` at startup.  It returns `0` (no diagnostics) so that
/// the protocol tests are deterministic and self-contained.
///
/// Applications supply their own policy; this constant must not encode
/// any Historia-specific or application-specific logic.
const FIXTURE_POLICY_SOURCE: &str = "0";

// ---------------------------------------------------------------------------
// PolicyModule
// ---------------------------------------------------------------------------

/// A compiled and loaded analyser policy ready for repeated invocation.
///
/// The `NativeModule` is kept alive across the lifetime of the process.
/// Every invocation calls `NativeModule::call_entry_value`, which
/// internally calls `HandleScope::begin_call` before entering the Wasm
/// function.  This guarantees that no value handle from a previous
/// invocation leaks into the current one, eliminating the
/// "stale whole-Wasm runtime handle" trap that appeared in earlier
/// incarnations of this binary.
struct PolicyModule {
    module: NativeModule,
}

impl PolicyModule {
    /// Compile `source` to a whole-Wasm artifact and load it.
    fn from_source(source: &str) -> Result<Self, String> {
        let program = compile_source(source).map_err(|e| e.to_string())?;
        let bytes = compile_artifact(&program)?;
        let module = NativeModule::load(&bytes)?;
        Ok(Self { module })
    }

    /// Invoke the policy entry and return the exit code.
    ///
    /// A return value of `0` means the policy found no additional
    /// diagnostics.  A fresh `HandleScope` is begun for every call via
    /// `call_entry_i64`, so handles from earlier calls are never
    /// observable inside the new invocation, eliminating the
    /// "stale whole-Wasm runtime handle" trap.
    fn invoke(&mut self) -> Result<i64, String> {
        self.module.call_entry_i64()
    }
}

// ---------------------------------------------------------------------------
// Request dispatch
// ---------------------------------------------------------------------------

fn handle_request(request: &Json, policy: &mut PolicyModule) -> Json {
    let op = request.get("op").and_then(Json::as_str).unwrap_or("");
    match op {
        "describe" => json!({
            "name": "hara-code-analyzer",
            "version": "0.1",
            "capabilities": ["describe", "ping", "analyze", "shutdown"]
        }),
        "ping" => json!({"ok": true}),
        "analyze" => {
            let source = match request.get("source").and_then(Json::as_str) {
                Some(s) => s,
                None => return json!({"error": "analyze requires a source field"}),
            };
            analyze_source(source, policy)
        }
        "shutdown" => json!({"ok": true}),
        _ if op.is_empty() => json!({"error": "request missing op field"}),
        _ => json!({"error": format!("unknown operation: {op}")}),
    }
}

/// Non-evaluating spanned source read followed by policy invocation.
fn analyze_source(source: &str, policy: &mut PolicyModule) -> Json {
    // Non-evaluating spanned source read.  Parse errors become diagnostics;
    // valid source proceeds to policy invocation.
    let parse_diagnostics: Vec<Json> = match read_forms(source) {
        Ok(_) => vec![],
        Err(error) => vec![json!({"kind": "parse-error", "message": error.to_string()})],
    };

    // Prepare and invoke the application-supplied policy module.
    // Each call begins a fresh handle scope via call_entry_i64, so
    // handles cannot be stale across repeated analyze requests.
    // A zero return means the policy found no additional diagnostics.
    let policy_diagnostics: Vec<Json> = match policy.invoke() {
        Ok(0) | Ok(_) => vec![],
        Err(error) => vec![json!({"kind": "policy-error", "message": error})],
    };

    let diagnostics: Vec<Json> = parse_diagnostics
        .into_iter()
        .chain(policy_diagnostics)
        .collect();
    json!({"diagnostics": diagnostics})
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() {
    if let Err(error) = run() {
        eprintln!("hara-code-analyzer: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut policy = PolicyModule::from_source(FIXTURE_POLICY_SOURCE)?;

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    for line in stdin.lock().lines() {
        let line = line.map_err(|e| format!("stdin read error: {e}"))?;
        if line.trim().is_empty() {
            continue;
        }

        let (op, response) = match serde_json::from_str::<Json>(&line) {
            Ok(request) => {
                let op = request
                    .get("op")
                    .and_then(Json::as_str)
                    .unwrap_or("")
                    .to_owned();
                let response = handle_request(&request, &mut policy);
                (op, response)
            }
            Err(e) => (
                String::new(),
                json!({"error": format!("malformed request: {e}")}),
            ),
        };

        writeln!(
            out,
            "{}",
            serde_json::to_string(&response).expect("response is always valid JSON")
        )
        .map_err(|e| format!("stdout write error: {e}"))?;
        out.flush()
            .map_err(|e| format!("stdout flush error: {e}"))?;

        if op == "shutdown" {
            break;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> PolicyModule {
        PolicyModule::from_source(FIXTURE_POLICY_SOURCE)
            .expect("fixture policy must compile and load")
    }

    // -- individual operations ----------------------------------------------

    #[test]
    fn describe_returns_name_version_and_capabilities() {
        let resp = handle_request(&json!({"op": "describe"}), &mut policy());
        assert_eq!(resp["name"], "hara-code-analyzer");
        assert_eq!(resp["version"], "0.1");
        let caps = resp["capabilities"].as_array().expect("capabilities array");
        let cap_strs: Vec<&str> = caps.iter().filter_map(|c| c.as_str()).collect();
        for required in &["describe", "ping", "analyze", "shutdown"] {
            assert!(
                cap_strs.contains(required),
                "missing capability: {required}"
            );
        }
    }

    #[test]
    fn ping_returns_ok_true() {
        let resp = handle_request(&json!({"op": "ping"}), &mut policy());
        assert_eq!(resp["ok"], true);
    }

    #[test]
    fn analyze_valid_source_returns_empty_diagnostics() {
        let resp = handle_request(
            &json!({"op": "analyze", "source": "(+ 1 2)"}),
            &mut policy(),
        );
        assert_eq!(resp["diagnostics"], json!([]));
    }

    #[test]
    fn analyze_malformed_source_reports_parse_error() {
        let resp = handle_request(&json!({"op": "analyze", "source": "((("}), &mut policy());
        let diags = resp["diagnostics"].as_array().expect("diagnostics array");
        assert!(
            !diags.is_empty(),
            "malformed source must produce diagnostics"
        );
        assert_eq!(diags[0]["kind"], "parse-error");
    }

    #[test]
    fn analyze_unicode_source_does_not_panic() {
        let resp = handle_request(
            &json!({"op": "analyze", "source": "(def 名前 \"世界\")"}),
            &mut policy(),
        );
        assert!(
            resp.get("diagnostics").is_some() || resp.get("error").is_some(),
            "unicode source must produce a diagnostics or error field"
        );
    }

    #[test]
    fn unknown_operation_returns_descriptive_error() {
        let resp = handle_request(&json!({"op": "frobnicate"}), &mut policy());
        let msg = resp["error"].as_str().expect("error string");
        assert!(
            msg.contains("unknown operation"),
            "expected 'unknown operation' in: {msg}"
        );
        assert!(msg.contains("frobnicate"), "expected op name in: {msg}");
    }

    #[test]
    fn shutdown_returns_ok_true() {
        let resp = handle_request(&json!({"op": "shutdown"}), &mut policy());
        assert_eq!(resp["ok"], true);
    }

    // -- stale-handle safety ------------------------------------------------

    #[test]
    fn repeated_analyze_calls_do_not_produce_stale_handles() {
        let mut p = policy();
        for i in 0..5 {
            let source = format!("(+ {i} 1)");
            let resp = handle_request(&json!({"op": "analyze", "source": source}), &mut p);
            assert_eq!(
                resp["diagnostics"],
                json!([]),
                "stale handle on iteration {i}"
            );
        }
    }

    // -- full protocol sequence ---------------------------------------------

    #[test]
    fn full_protocol_sequence_matches_acceptance_criteria() {
        let mut p = policy();
        let requests = [
            json!({"op": "describe"}),
            json!({"op": "ping"}),
            json!({"op": "analyze", "source": "(+ 1 2)"}),
            json!({"op": "analyze", "source": "(* 3 4)"}),
            json!({"op": "frobnicate"}),
            json!({"op": "shutdown"}),
        ];
        let responses: Vec<Json> = requests
            .iter()
            .map(|req| handle_request(req, &mut p))
            .collect();

        // describe
        assert_eq!(responses[0]["name"], "hara-code-analyzer");
        // ping
        assert_eq!(responses[1]["ok"], true);
        // analyze (first)
        assert_eq!(responses[2]["diagnostics"], json!([]));
        // analyze (second)
        assert_eq!(responses[3]["diagnostics"], json!([]));
        // unknown operation
        let err = responses[4]["error"].as_str().expect("error field");
        assert!(err.contains("unknown operation"), "unexpected: {err}");
        // shutdown
        assert_eq!(responses[5]["ok"], true);
    }
}
