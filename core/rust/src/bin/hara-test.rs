#[cfg(feature = "direct-native")]
use hara_wasm::direct_native::NativeEngine;
use hara_wasm::kernel::{parse, parse_forms, Form};
use hara_wasm::project;
use hara_wasm::{SessionId, SessionKernel};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

// The test runner is normally a debug binary, whose tree-evaluator frames are
// larger than the optimized production frames. Release verification still
// exercises the production 8 MiB ceiling.
const TEST_STACK_SIZE: usize = if cfg!(debug_assertions) {
    64 * 1024 * 1024
} else {
    8 * 1024 * 1024
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestSummary {
    pub path: PathBuf,
    pub passed: bool,
    pub facts: usize,
    pub checks: usize,
    pub passed_checks: usize,
    pub failed_checks: usize,
    pub errors: usize,
    pub timeouts: usize,
    pub native_telemetry: Option<NativeTelemetry>,
    pub raw: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NativeTelemetry {
    pub bytecode_functions: usize,
    pub bytecode_instructions: usize,
    pub native_target_calls: usize,
    pub invocations: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TestBackend {
    Interpreter,
    Native,
}

impl Default for TestBackend {
    fn default() -> Self {
        Self::Native
    }
}

impl TestBackend {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "interpreter" => Ok(Self::Interpreter),
            "native" => Ok(Self::Native),
            _ => Err(format!(
                "unknown test backend {value}; expected interpreter or native"
            )),
        }
    }

    fn runtime_name(self) -> &'static str {
        match self {
            Self::Interpreter => "interpreter",
            Self::Native => "direct-native",
        }
    }
}

impl TestSummary {
    pub fn failure_message(&self) -> String {
        format!("{} failed: {}", self.path.display(), self.raw)
    }
}

pub fn run_file(root: &Path, file: &Path) -> Result<TestSummary, String> {
    run_file_with_backend(root, file, TestBackend::Interpreter)
}

fn run_file_with_backend(
    root: &Path,
    file: &Path,
    backend: TestBackend,
) -> Result<TestSummary, String> {
    let prepared = prepare_test_file(root, file)?;
    let execution = std::thread::Builder::new()
        .name("hara-test-file".into())
        .stack_size(TEST_STACK_SIZE)
        .spawn(move || execute_test_file(prepared, backend, None))
        .map_err(|error| format!("cannot start test thread: {error}"))?;

    match execution.join() {
        Ok(result) => result,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

struct PreparedTestFile {
    root: PathBuf,
    file: PathBuf,
    root_text: String,
    test_namespace: Option<String>,
    prefix: Vec<(String, bool)>,
    final_form: Form,
}

fn prepare_test_file(root: &Path, file: &Path) -> Result<PreparedTestFile, String> {
    let root = root
        .canonicalize()
        .map_err(|error| format!("cannot resolve {}: {error}", root.display()))?;
    let file = file
        .canonicalize()
        .map_err(|error| format!("cannot resolve {}: {error}", file.display()))?;
    if file.extension().and_then(|value| value.to_str()) != Some("hal") {
        return Err(format!(
            "test file must use the .hal extension: {}",
            file.display()
        ));
    }
    let source = fs::read_to_string(&file)
        .map_err(|error| format!("cannot read {}: {error}", file.display()))?;
    let test_namespace = declared_namespace(&source);
    let mut forms = parse_forms(&source)
        .map_err(|error| format!("cannot parse {}: {error}", file.display()))?;
    let final_form = forms
        .pop()
        .ok_or_else(|| format!("{} contains no forms", file.display()))?;
    let prefix = forms
        .into_iter()
        .map(|form| {
            let declares_namespace = declared_namespace_form(&form).is_some();
            (form.to_string(), declares_namespace)
        })
        .collect::<Vec<_>>();
    let root_text = root
        .to_str()
        .ok_or_else(|| format!("project root is not UTF-8: {}", root.display()))?
        .to_owned();
    Ok(PreparedTestFile {
        root,
        file,
        root_text,
        test_namespace,
        prefix,
        final_form,
    })
}

fn execute_test_file(
    prepared: PreparedTestFile,
    backend: TestBackend,
    #[cfg(feature = "direct-native")] shared_engine: Option<NativeEngine>,
    #[cfg(not(feature = "direct-native"))] _shared_engine: Option<()>,
) -> Result<TestSummary, String> {
    let PreparedTestFile {
        root,
        file,
        root_text,
        test_namespace,
        prefix,
        final_form,
    } = prepared;
    let mut kernel = {
        #[cfg(feature = "direct-native")]
        if let Some(engine) = shared_engine {
            SessionKernel::new_with_native_engine(engine)
        } else {
            SessionKernel::new()
        }
        #[cfg(not(feature = "direct-native"))]
        {
            SessionKernel::new()
        }
    };
    kernel.set_test_runner("native")?;
    kernel.set_execution_backend(backend.runtime_name())?;
    register_project_sources(&root, &mut kernel)?;
    let root_session = SessionId::parse("ROOT")?;
    let mount = kernel.create_native_filesystem(&root_text);
    kernel.attach_filesystem(&root_session, mount)?;
    let session = kernel.session_mut(&root_session)?;
    session.install_native_socket_provider();
    session.install_native_process_provider();
    let mut namespace_ready = false;
    for (form, declares_namespace) in prefix {
        let source = if backend == TestBackend::Interpreter {
            // Preserve the interpreter runner's historical namespace handoff:
            // prefix forms after `ns` are explicitly evaluated in the test
            // namespace, while forms before it run in ROOT.
            if namespace_ready {
                let namespace = test_namespace
                    .as_deref()
                    .expect("a namespace declaration made the test namespace available");
                format!("(eval-in-ns (quote {namespace}) (quote [(do {form} nil)]))")
            } else {
                format!("(do {form} nil)")
            }
        } else if declares_namespace || is_management_form(&form) {
            // Keep the namespace declaration at the runtime's management
            // boundary. The same applies to a top-level require: wrapping it
            // in `do` would make the direct backend treat the management form
            // as ordinary bytecode before the native namespace loader sees it.
            form
        } else {
            format!("(do {form} nil)")
        };
        kernel
            .eval(&root_session, &source)
            .map_err(|error| format!("{}: {error}", file.display()))?;
        if backend == TestBackend::Interpreter {
            namespace_ready |= declares_namespace;
        }
    }
    let final_source = if backend == TestBackend::Interpreter {
        match test_namespace.as_deref() {
            Some(namespace) => {
                format!("(pr-str (eval-in-ns (quote {namespace}) (quote [{final_form}])))")
            }
            None => format!("(pr-str {final_form})"),
        }
    } else {
        // A namespace declaration leaves the session selected in that
        // namespace. Evaluate the fact/run form there directly so native
        // tests do not cross back through evaluator-only `eval-in-ns`.
        format!("(pr-str {final_form})")
    };
    let output = kernel
        .eval(&root_session, &final_source)
        .map_err(|error| format!("{}: {error}", file.display()))?;
    let initial_error = match parse_summary(file.clone(), &output) {
        Ok(summary) => {
            return Ok(with_native_telemetry(
                summary,
                backend,
                &kernel,
                &root_session,
            ))
        }
        Err(error) => error,
    };
    let Some(namespace) = test_namespace else {
        return Err(format!("{}: {initial_error}", file.display()));
    };
    let output = kernel
        .eval(&root_session, &test_run_source(&namespace))
        .map_err(|error| {
            format!(
                "{}: cannot execute test namespace {namespace}: {error}",
                file.display()
            )
        })?;
    let summary = parse_summary(file.clone(), &output)
        .map_err(|error| format!("{}: {error}", file.display()))?;
    Ok(with_native_telemetry(
        summary,
        backend,
        &kernel,
        &root_session,
    ))
}

fn is_management_form(source: &str) -> bool {
    let Ok(form) = parse(source) else {
        return false;
    };
    match form {
        Form::Metadata(_, value) => is_management_form(&value.to_string()),
        Form::List(items) => matches!(
            items.first(),
            Some(Form::Symbol(operator))
                if matches!(operator.as_str(), "ns" | "ns+" | "require" | "in-ns")
        ),
        _ => false,
    }
}

fn declared_namespace(source: &str) -> Option<String> {
    parse_forms(source)
        .ok()?
        .iter()
        .find_map(declared_namespace_form)
}

fn declared_namespace_form(form: &Form) -> Option<String> {
    match form {
        Form::Metadata(_, value) => declared_namespace_form(value),
        Form::List(items) => match items.as_slice() {
            [Form::Symbol(head), Form::Symbol(namespace), ..]
                if (head == "ns" || head == "ns+") && !namespace.contains('/') =>
            {
                Some(namespace.clone())
            }
            _ => None,
        },
        _ => None,
    }
}

fn test_run_source(namespace: &str) -> String {
    format!(
        "(pr-str (let [summary (code.test/run {{:namespace \"{namespace}\"}}) failures (filter (fn [result] (not= :passed (:status result))) (:results summary)) diagnostic (map (fn [result] (let [check (first (filter (fn [item] (not (:pass item))) (:checks result))) error (or (:error result) (:error check))] {{:name (:name result) :status (:status result) :error (if error (apply str (take 2000 error)) nil) :actual (:actual check) :expected (:expected check)}})) failures)] (assoc (dissoc summary :results :report) :results (str diagnostic))))"
    )
}

fn register_project_sources(root: &Path, kernel: &mut SessionKernel) -> Result<(), String> {
    let current_project = project::read(root)?;
    let catalog = project::source_catalog(&current_project)?;
    kernel.register_source_catalog(&catalog);
    Ok(())
}

pub fn run_paths(root: &Path, paths: &[PathBuf]) -> Result<Vec<TestSummary>, String> {
    run_paths_with_backend(root, paths, TestBackend::Interpreter)
}

fn run_paths_with_backend(
    root: &Path,
    paths: &[PathBuf],
    backend: TestBackend,
) -> Result<Vec<TestSummary>, String> {
    let mut files = Vec::new();
    for path in paths {
        collect(path, &mut files)?;
    }
    files.sort();
    files.dedup();
    if files.is_empty() {
        return Err("no .hal test files found".into());
    }
    #[cfg(feature = "direct-native")]
    if backend == TestBackend::Native {
        let root = root.to_owned();
        let execution = std::thread::Builder::new()
            .name("hara-test-native-suite".into())
            .stack_size(TEST_STACK_SIZE)
            .spawn(move || {
                let engine = NativeEngine::new();
                Ok(files
                    .iter()
                    .map(|file| {
                        let before = engine.telemetry();
                        let result = prepare_test_file(&root, file).and_then(|prepared| {
                            execute_test_file(prepared, backend, Some(engine.clone()))
                        });
                        let mut summary = match result {
                            Ok(summary) => summary,
                            Err(error) => TestSummary {
                                path: file.clone(),
                                passed: false,
                                facts: 0,
                                checks: 0,
                                passed_checks: 0,
                                failed_checks: 0,
                                errors: 1,
                                timeouts: 0,
                                native_telemetry: None,
                                raw: error,
                            },
                        };
                        let after = engine.telemetry();
                        if backend == TestBackend::Native {
                            summary.native_telemetry = Some(NativeTelemetry {
                                bytecode_functions: after
                                    .bytecode_functions
                                    .saturating_sub(before.bytecode_functions),
                                bytecode_instructions: after
                                    .bytecode_instructions
                                    .saturating_sub(before.bytecode_instructions),
                                native_target_calls: after
                                    .native_target_calls
                                    .saturating_sub(before.native_target_calls),
                                invocations: after.invocations.saturating_sub(before.invocations),
                            });
                        }
                        summary
                    })
                    .collect())
            })
            .map_err(|error| format!("cannot start native test thread: {error}"))?;
        return match execution.join() {
            Ok(result) => result,
            Err(panic) => std::panic::resume_unwind(panic),
        };
    }
    Ok(files
        .iter()
        .map(|file| match run_file_with_backend(root, file, backend) {
            Ok(summary) => summary,
            Err(error) => TestSummary {
                path: file.clone(),
                passed: false,
                facts: 0,
                checks: 0,
                passed_checks: 0,
                failed_checks: 0,
                errors: 1,
                timeouts: 0,
                native_telemetry: None,
                raw: error,
            },
        })
        .collect())
}

fn with_native_telemetry(
    mut summary: TestSummary,
    backend: TestBackend,
    kernel: &SessionKernel,
    session: &SessionId,
) -> TestSummary {
    if backend == TestBackend::Native {
        #[cfg(feature = "direct-native")]
        if let Ok(telemetry) = kernel.native_execution_telemetry(session) {
            summary.native_telemetry = Some(NativeTelemetry {
                bytecode_functions: telemetry.bytecode_functions,
                bytecode_instructions: telemetry.bytecode_instructions,
                native_target_calls: telemetry.native_target_calls,
                invocations: telemetry.invocations,
            });
        }
    }
    summary
}

fn collect(path: &Path, output: &mut Vec<PathBuf>) -> Result<(), String> {
    if path.is_file() {
        if path.extension().and_then(|value| value.to_str()) == Some("hal") {
            output.push(path.to_path_buf());
        }
        return Ok(());
    }
    if !path.is_dir() {
        return Err(format!("test path not found: {}", path.display()));
    }
    let mut entries = fs::read_dir(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?
        .map(|entry| {
            entry
                .map(|value| value.path())
                .map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort();
    for entry in entries {
        collect(&entry, output)?;
    }
    Ok(())
}

fn parse_summary(path: PathBuf, output: &str) -> Result<TestSummary, String> {
    let mut form = parse(output).map_err(|error| format!("invalid test result: {error}"))?;
    // SessionKernel renders string results, while some legacy test files also
    // return `(pr-str (run ...))`; unwrap both transport and test-owned layers.
    for _ in 0..8 {
        let Form::String(encoded) = form else {
            break;
        };
        form = parse(&encoded).map_err(|error| format!("invalid encoded test result: {error}"))?;
    }

    match form {
        Form::Map(entries) => parse_code_test_summary(path, entries, output),
        Form::Vector(items) | Form::List(items) => parse_direct_results(path, items, output),
        _ => Err("test file must return a code.test/run summary or test result vector".into()),
    }
}

fn parse_code_test_summary(
    path: PathBuf,
    entries: Vec<(Form, Form)>,
    raw: &str,
) -> Result<TestSummary, String> {
    let status = match map_get(&entries, "status") {
        Some(Form::Keyword(status)) => status,
        _ => return Err("code.test/run result is missing keyword :status".into()),
    };
    let counts = match map_get(&entries, "counts") {
        Some(Form::Map(counts)) => counts,
        _ => return Err("code.test/run result is missing map :counts".into()),
    };
    let passed_facts = map_number(counts, "passed", 0)?;
    let failed_facts = map_number(counts, "failed", 0)?;
    let errors = map_number(counts, "error", 0)?;
    let timeouts = map_number(counts, "timeout", 0)?;
    let skipped = map_number(counts, "skipped", 0)?;
    let cancelled = map_number(counts, "cancelled", 0)?;
    let facts = map_number(
        &entries,
        "facts",
        passed_facts + failed_facts + errors + timeouts + skipped + cancelled,
    )?;
    let passed_checks = map_number(&entries, "passed", passed_facts)?;
    let failed_checks = map_number(&entries, "failed", failed_facts)?;
    let checks = map_number(&entries, "checks", passed_checks + failed_checks)?;
    Ok(TestSummary {
        path,
        passed: status == "passed",
        facts,
        checks,
        passed_checks,
        failed_checks,
        errors,
        timeouts,
        native_telemetry: None,
        raw: raw.to_owned(),
    })
}

fn parse_direct_results(path: PathBuf, items: Vec<Form>, raw: &str) -> Result<TestSummary, String> {
    let mut passed = 0usize;
    let mut failed = 0usize;
    for item in items {
        let Form::Tagged(tag, fields) = item else {
            return Err("direct test result must be a native Result".into());
        };
        if tag != "hara/Result" {
            return Err("direct test result must be a native Result".into());
        }
        let Form::Vector(fields) = fields.as_ref() else {
            return Err("native Result must contain status, data, error, and context".into());
        };
        match fields.as_slice() {
            [Form::Keyword(status), Form::Bool(true), _, _] if status == "success" => passed += 1,
            [Form::Keyword(status), Form::Bool(false), _, _] if status == "success" => failed += 1,
            [Form::Keyword(status), _, _, _] if status == "error" => failed += 1,
            _ => return Err("test Result must contain a boolean success value or error".into()),
        }
    }
    Ok(TestSummary {
        path,
        passed: failed == 0,
        facts: passed + failed,
        checks: passed + failed,
        passed_checks: passed,
        failed_checks: failed,
        errors: 0,
        timeouts: 0,
        native_telemetry: None,
        raw: raw.to_owned(),
    })
}

fn map_get<'a>(entries: &'a [(Form, Form)], key: &str) -> Option<&'a Form> {
    entries
        .iter()
        .find_map(|(candidate, value)| match candidate {
            Form::Keyword(name) if name == key => Some(value),
            _ => None,
        })
}

fn map_number(entries: &[(Form, Form)], key: &str, fallback: usize) -> Result<usize, String> {
    match map_get(entries, key) {
        None => Ok(fallback),
        Some(Form::Number(value)) if *value >= 0 => Ok(*value as usize),
        Some(_) => Err(format!(
            "code.test/run result :{key} must be a non-negative number"
        )),
    }
}

fn parse_arguments() -> Result<(PathBuf, TestBackend, Vec<PathBuf>), String> {
    parse_arguments_from(env::args().skip(1))
}

fn parse_arguments_from<I>(arguments: I) -> Result<(PathBuf, TestBackend, Vec<PathBuf>), String>
where
    I: IntoIterator<Item = String>,
{
    let mut root = env::current_dir().map_err(|error| error.to_string())?;
    let mut backend = TestBackend::default();
    let mut paths = Vec::new();
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        if argument == "--root" {
            let value = arguments.next().ok_or("--root requires a path")?;
            root = PathBuf::from(value);
        } else if argument == "--backend" {
            let value = arguments
                .next()
                .ok_or("--backend requires interpreter or native")?;
            backend = TestBackend::parse(&value)?;
        } else if argument == "--help" || argument == "-h" {
            return Err(
                "usage: hara-test [--root ROOT] [--backend interpreter|native (default: native)] FILE_OR_DIRECTORY..."
                    .into(),
            );
        } else {
            paths.push(PathBuf::from(argument));
        }
    }
    if paths.is_empty() {
        let default = root.join("test");
        if default.exists() {
            paths.push(default);
        } else {
            return Err(
                "usage: hara-test [--root ROOT] [--backend interpreter|native (default: native)] FILE_OR_DIRECTORY..."
                    .into(),
            );
        }
    }
    let paths = paths
        .into_iter()
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                root.join(path)
            }
        })
        .collect();
    Ok((root, backend, paths))
}

fn main() {
    let (root, backend, paths) = match parse_arguments() {
        Ok(value) => value,
        Err(error) => {
            eprintln!("hara-test: {error}");
            std::process::exit(2);
        }
    };
    let summaries = match run_paths_with_backend(&root, &paths, backend) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("hara-test: {error}");
            std::process::exit(2);
        }
    };

    let mut failed = 0usize;
    for summary in &summaries {
        if summary.passed {
            println!(
                "PASS  {} ({} facts, {} checks)",
                summary.path.display(),
                summary.facts,
                summary.checks
            );
        } else {
            failed += 1;
            println!("FAIL  {}", summary.path.display());
            println!("      {}", summary.raw);
        }
    }
    println!(
        "\nHara test summary: {} passed, {} failed, {} total",
        summaries.len() - failed,
        failed,
        summaries.len()
    );
    if backend == TestBackend::Native {
        let telemetry = summaries
            .iter()
            .filter_map(|summary| summary.native_telemetry)
            .fold(NativeTelemetry::default(), |total, current| {
                NativeTelemetry {
                    bytecode_functions: total.bytecode_functions + current.bytecode_functions,
                    bytecode_instructions: total.bytecode_instructions
                        + current.bytecode_instructions,
                    native_target_calls: total.native_target_calls + current.native_target_calls,
                    invocations: total.invocations + current.invocations,
                }
            });
        println!(
            "Hara native telemetry: backend=bytecode-vm-native-substrate bytecode-functions={} bytecode-instructions={} native-target-calls={} invocations={}",
            telemetry.bytecode_functions,
            telemetry.bytecode_instructions,
            telemetry.native_target_calls,
            telemetry.invocations
        );
    }
    if failed != 0 {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        declared_namespace, parse_arguments_from, run_file, run_file_with_backend, run_paths,
        TestBackend,
    };
    use std::fs;
    use std::path::Path;

    fn repository_root() -> &'static Path {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("rust crate must have repository parent")
    }

    #[test]
    fn discovers_bare_and_metadata_wrapped_namespaces() {
        assert_eq!(
            declared_namespace("(ns example.bare)"),
            Some("example.bare".into())
        );
        assert_eq!(
            declared_namespace("^{:seedgen/skip true}\n(ns example.metadata)"),
            Some("example.metadata".into())
        );
    }

    #[test]
    fn defaults_the_cli_runner_to_native_and_keeps_interpreter_override() {
        let (_, backend, _) = parse_arguments_from(["fixture.hal".into()]).unwrap();
        assert_eq!(backend, TestBackend::Native);

        let (_, backend, _) = parse_arguments_from([
            "--backend".into(),
            "interpreter".into(),
            "fixture.hal".into(),
        ])
        .unwrap();
        assert_eq!(backend, TestBackend::Interpreter);
    }

    #[test]
    fn runs_passing_code_test_summary() {
        let root = repository_root();
        let summary = run_file(
            root,
            &root.join("lib/test-fixtures/std/native/test_runner_pass.hal"),
        )
        .unwrap();
        assert!(summary.passed, "{}", summary.failure_message());
        assert_eq!(summary.facts, 1);
        assert_eq!(summary.checks, 1);
        assert_eq!(summary.passed_checks, 1);
        assert_eq!(summary.failed_checks, 0);
    }

    #[test]
    fn loads_project_namespaces_from_source_paths() {
        let root = repository_root();
        let suffix = std::process::id();
        let source = root.join(format!("lib/src/hara/test_source_{suffix}.hal"));
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(
            &source,
            format!("(ns hara.test-source-{suffix}) (defn answer [] 40)"),
        )
        .unwrap();
        let test = std::env::temp_dir().join(format!("hara-source-backed-test-{suffix}.hal"));
        fs::write(
            &test,
            format!(
                "(ns hara.source-backed-test-{suffix}\n  (:use code.test)\n  (:require [hara.test-source-{suffix} :as helper]))\n\n(fact \"requires a project source file\"\n  (helper/answer) => 40)\n\n(run {{:namespace \"hara.source-backed-test-{suffix}\"}})\n"
            ),
        )
        .unwrap();

        let mut backends = vec![TestBackend::Interpreter];
        #[cfg(feature = "direct-native")]
        backends.push(TestBackend::Native);
        let mut results = Vec::new();
        for backend in backends {
            results.push((backend, run_file_with_backend(root, &test, backend)));
        }
        fs::remove_file(&test).unwrap();
        fs::remove_file(&source).unwrap();
        for (backend, result) in results {
            let summary = result.unwrap();
            assert!(summary.passed, "{}: {summary:?}", backend.runtime_name());
            assert_eq!(summary.facts, 1);
            assert_eq!(summary.checks, 1);
            assert_eq!(summary.passed_checks, 1);
        }
    }

    #[test]
    fn runs_shared_native_test_result_api_corpus() {
        let root = repository_root();
        let summary = run_file(
            root,
            &root.join("lib/test-fixtures/std/native/test_result_api.hal"),
        )
        .unwrap();
        assert!(summary.passed, "{}", summary.failure_message());
        assert_eq!(summary.facts, 3);
        assert_eq!(summary.checks, 3);
        assert_eq!(summary.passed_checks, 3);
        assert_eq!(summary.failed_checks, 0);
    }

    #[cfg(feature = "direct-native")]
    #[test]
    fn runs_passing_code_test_summary_through_direct_native() {
        let root = repository_root();
        let summary = run_file_with_backend(
            root,
            &root.join("lib/test-fixtures/std/native/test_runner_pass.hal"),
            TestBackend::Native,
        )
        .unwrap();
        assert!(summary.passed, "{}", summary.failure_message());
        assert_eq!(summary.facts, 1);
        assert_eq!(summary.checks, 1);
        assert_eq!(summary.passed_checks, 1);
        assert_eq!(summary.failed_checks, 0);
        let telemetry = summary
            .native_telemetry
            .expect("native runs expose execution telemetry");
        assert!(telemetry.bytecode_functions > 0);
        assert!(telemetry.bytecode_instructions > 0);
        assert!(telemetry.native_target_calls > 0);
        assert!(telemetry.invocations > 0);
    }

    #[test]
    fn preserves_namespace_aliases_across_top_level_test_forms() {
        let root = repository_root();
        let summary = run_file(root, &root.join("lib/test/std/block/heal/edit_test.hal")).unwrap();
        assert!(summary.passed, "{}", summary.failure_message());
        assert_eq!(summary.checks, 11);
        assert_eq!(summary.passed_checks, 11);
    }

    #[test]
    fn runs_metadata_wrapped_code_test_namespace() {
        let root = repository_root();
        let path = std::env::temp_dir().join(format!(
            "hara-metadata-wrapped-test-{}.hal",
            std::process::id()
        ));
        fs::write(
            &path,
            "^{:seedgen/skip true}\n(ns std.native.metadata-wrapped-test\n  (:use code.test))\n\n(fact \"runs metadata-wrapped namespaces\"\n  (+ 20 22) => 42)\n",
        )
        .unwrap();
        let summary = run_file(root, &path);
        fs::remove_file(&path).unwrap();
        let summary = summary.unwrap();
        assert!(summary.passed, "{}", summary.failure_message());
        assert_eq!(summary.facts, 1);
        assert_eq!(summary.checks, 1);
        assert_eq!(summary.passed_checks, 1);
        assert_eq!(summary.failed_checks, 0);
    }

    #[test]
    fn reports_namespace_execution_errors_instead_of_using_invalid_output() {
        let root = repository_root();
        let path = std::env::temp_dir().join(format!(
            "hara-invalid-test-namespace-{}.hal",
            std::process::id()
        ));
        fs::write(
            &path,
            "^{:seedgen/skip true}\n(ns std.native.invalid-test-namespace)\n\n:not-a-test-result\n",
        )
        .unwrap();
        let error = run_file(root, &path).unwrap_err();
        fs::remove_file(&path).unwrap();
        assert!(error.contains("cannot execute test namespace"), "{error}");
        assert!(error.contains("unbound symbol: code.test/run"), "{error}");
    }

    #[test]
    fn run_paths_preserves_file_errors_and_continues_the_inventory() {
        let root = repository_root();
        let path = std::env::temp_dir().join(format!(
            "hara-runtime-error-test-{}.hal",
            std::process::id()
        ));
        fs::write(&path, "(missing-runtime-function)").unwrap();
        let summaries = run_paths(
            root,
            &[
                path.clone(),
                root.join("lib/test-fixtures/std/native/test_runner_pass.hal"),
            ],
        )
        .unwrap();
        fs::remove_file(&path).unwrap();
        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries.iter().filter(|summary| summary.passed).count(), 1);
        let failure = summaries
            .iter()
            .find(|summary| !summary.passed)
            .expect("runtime error summary");
        assert_eq!(failure.errors, 1);
        assert!(failure.raw.contains("unbound symbol"), "{}", failure.raw);
    }

    #[test]
    fn preserves_failing_summary_for_cargo() {
        let root = repository_root();
        let summary = run_file(
            root,
            &root.join("lib/test-fixtures/std/native/test_runner_fail.hal"),
        )
        .unwrap();
        assert!(!summary.passed);
        assert_eq!(summary.facts, 1);
        assert_eq!(summary.checks, 1);
        assert_eq!(summary.passed_checks, 0);
        assert_eq!(summary.failed_checks, 1);
        assert!(summary.failure_message().contains(":failed"));
    }
}
