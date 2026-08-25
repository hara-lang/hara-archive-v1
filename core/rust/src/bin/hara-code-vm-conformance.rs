use hara_wasm::spec_registry;
use hara_wasm::vm::conformance::{parse_corpus, run_embedded, validate_upstream, EMBEDDED_CORPUS};
use std::path::{Path, PathBuf};

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    validate_embedded_upstream()?;
    let command = std::env::args().nth(1).unwrap_or_else(|| "check".into());
    let report = run_embedded()?;
    match command.as_str() {
        "check" => {
            if !report.passed() {
                eprintln!("{}", report.to_json(true)?);
                return Err(format!(
                    "code.vm conformance failed with {} failed checks",
                    report.failed_checks()
                ));
            }
            println!("code.vm conformance passed: {} cases", report.cases.len());
            Ok(())
        }
        "report" => {
            println!("{}", report.to_json(true)?);
            Ok(())
        }
        "browser" => {
            println!("{}", report.browser_json(true)?);
            Ok(())
        }
        other => Err(format!(
            "unknown code.vm conformance command `{other}`; use check, report, or browser"
        )),
    }
}

fn validate_embedded_upstream() -> Result<(), String> {
    let corpus = parse_corpus(EMBEDDED_CORPUS)?;
    let relative = corpus
        .upstream
        .strip_prefix("hara-specs-registry/")
        .unwrap_or(&corpus.upstream);
    let candidates = spec_registry::resolve(relative)
        .into_iter()
        .chain([
            PathBuf::from(&corpus.upstream),
            Path::new("..").join(&corpus.upstream),
        ])
        .collect::<Vec<_>>();
    let path = candidates
        .iter()
        .find(|path| path.is_file())
        .ok_or_else(|| {
            format!(
                "code.vm upstream corpus is unavailable; checked {}",
                candidates
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;
    let source = std::fs::read_to_string(path).map_err(|error| {
        format!(
            "cannot read code.vm upstream corpus {}: {error}",
            path.display()
        )
    })?;
    validate_upstream(&corpus, &source)
}
