use hara_wasm::cli_app;
use hara_wasm::project as project_model;
use std::env;
use std::io::{self, Read};
use std::path::PathBuf;

#[path = "cli/build.rs"]
mod build;
#[path = "cli/build_check.rs"]
mod build_check;
#[path = "cli/form.rs"]
mod form;
#[path = "cli/hara.rs"]
mod hara;
#[path = "cli/metaspec.rs"]
mod metaspec;
#[path = "cli/project.rs"]
mod project;
#[path = "cli/spec.rs"]
mod spec;

#[derive(Default)]
pub(crate) struct Options {
    pub(crate) root: Option<PathBuf>,
    pub(crate) project: Option<PathBuf>,
    pub(crate) lite_project: Option<PathBuf>,
    pub(crate) native_sockets: bool,
    pub(crate) allow_file: bool,
    pub(crate) allow_process: bool,
    pub(crate) allow_postgres: bool,
    pub(crate) log_requests: bool,
    pub(crate) offline: bool,
    pub(crate) host: String,
    pub(crate) port: u16,
    command: Vec<String>,
    pub(crate) history_file: Option<PathBuf>,
    pub(crate) no_history: bool,
    pub(crate) no_splash: bool,
    pub(crate) no_color: bool,
}

pub(crate) fn parse_options() -> Result<Options, String> {
    let mut options = Options {
        host: "127.0.0.1".into(),
        port: 1311,
        ..Options::default()
    };
    let mut args = env::args().skip(1);
    while let Some(argument) = args.next() {
        if argument == "--" {
            options.command.extend(args);
            break;
        }
        match argument.as_str() {
            "--help" | "-h" => {
                options.command.push(argument);
                options.command.extend(args);
                break;
            }
            "--version" | "-V" => {
                options.command.push(argument);
                options.command.extend(args);
                break;
            }
            "--root" => options.root = Some(PathBuf::from(required(&mut args, "--root")?)),
            "--project" => options.project = Some(PathBuf::from(required(&mut args, "--project")?)),
            "--native-sockets" | "--allow-net" => options.native_sockets = true,
            "--allow-file" => options.allow_file = true,
            "--allow-process" => options.allow_process = true,
            "--allow-postgres" => options.allow_postgres = true,
            "--log-requests" => options.log_requests = true,
            "--offline" => options.offline = true,
            "--no-history" => options.no_history = true,
            "--no-splash" => options.no_splash = true,
            "--no-color" => options.no_color = true,
            "--history" => {
                options.history_file = Some(PathBuf::from(required(&mut args, "--history")?))
            }
            "--host" => options.host = required(&mut args, "--host")?,
            "--port" => {
                options.port = required(&mut args, "--port")?
                    .parse()
                    .map_err(|_| "--port must be between 0 and 65535".to_owned())?
            }
            value if value.starts_with("--history=") => {
                options.history_file = Some(PathBuf::from(&value[10..]))
            }
            value if value.starts_with("--root=") => {
                options.root = Some(PathBuf::from(option_value(value, "--root")?))
            }
            value if value.starts_with("--project=") => {
                options.project = Some(PathBuf::from(option_value(value, "--project")?))
            }
            value if value.starts_with("--host=") => {
                options.host = option_value(value, "--host")?.to_owned()
            }
            value if value.starts_with("--port=") => {
                options.port = option_value(value, "--port")?
                    .parse()
                    .map_err(|_| "--port must be between 0 and 65535".to_owned())?
            }
            value if value.starts_with('-') => return Err(format!("unknown option: {value}")),
            value => {
                options.command.push(value.into());
                options.command.extend(args);
                break;
            }
        }
    }
    Ok(options)
}

fn required(args: &mut impl Iterator<Item = String>, option: &str) -> Result<String, String> {
    args.next()
        .ok_or_else(|| format!("{option} requires a value"))
}

fn option_value<'a>(argument: &'a str, option: &str) -> Result<&'a str, String> {
    let value = argument
        .strip_prefix(option)
        .and_then(|value| value.strip_prefix('='))
        .unwrap_or_default();
    if value.is_empty() {
        Err(format!("{option} requires a value"))
    } else {
        Ok(value)
    }
}

pub(crate) fn run(mut options: Options) -> Result<(), String> {
    options.lite_project = distributed_lite_project();
    if options.allow_postgres {
        if let Some(path) = options.project.as_deref() {
            let project = project_model::discover(path)?;
            if !project
                .capabilities
                .iter()
                .any(|value| value == "db/postgres")
            {
                return Err("project must declare :db/postgres before --allow-postgres".into());
            }
        }
    }
    // Project aliases are argv-only macros, expanded before the normative
    // route table.  A directory without project.edn simply has no aliases.
    let expanded = match project_model::discover(
        options
            .project
            .as_deref()
            .unwrap_or_else(|| std::path::Path::new(".")),
    ) {
        Ok(project) => project_model::expand_aliases(&project, &options.command)?,
        Err(_) => options.command.clone(),
    };
    hara::run(&options, &expanded)
}

/// Runs the dependency-light native CLI surface without loading `tool.cli`.
///
/// This path is intentionally limited to evaluator and REPL operations so a
/// source checkout remains usable while higher-level HAL tooling is being
/// repaired. Project sources are still registered by the ordinary native
/// evaluator when `--project` is supplied.
pub(crate) fn run_lite(mut options: Options) -> Result<(), String> {
    options.lite_project = bundled_lite_project();
    let command = options.command.clone();
    match command.first().map(String::as_str) {
        Some("--help" | "-h" | "help") => {
            usage_lite();
            Ok(())
        }
        Some("--version" | "-V") => {
            println!("hara lite {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("eval") => project::direct_eval(&options, &command[1..].join(" ")),
        Some("run" | "--file") => project::run_file(
            &options,
            command
                .get(1)
                .ok_or_else(|| "run requires a file path".to_owned())?,
        ),
        Some("stdin") => {
            let mut source = String::new();
            io::stdin()
                .read_to_string(&mut source)
                .map_err(|error| format!("stdin: {error}"))?;
            project::direct_eval(&options, &source)
        }
        Some("headless" | "server") => project::run_headless(&options),
        Some("remote") => project::run_remote(
            command
                .get(1)
                .ok_or_else(|| "remote requires HOST:PORT".to_owned())?,
        ),
        Some("repl") | None => crate::repl::run_repl(&options, true),
        Some(command) => Err(format!("unknown lite command: {command}")),
    }
}

fn bundled_lite_project() -> Option<PathBuf> {
    distributed_lite_project().or_else(|| {
        let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../");
        repository
            .join("project.edn")
            .is_file()
            .then_some(repository)
    })
}

fn distributed_lite_project() -> Option<PathBuf> {
    if let Some(path) = env::var_os("HARA_LITE_PROJECT") {
        return Some(PathBuf::from(path));
    }
    let executable = env::current_exe().ok()?;
    let prefix = executable.parent()?.parent()?;
    let project = prefix.join("share/hara-lite");
    project.join("project.edn").is_file().then_some(project)
}

pub(crate) fn usage_lite() {
    println!("Hara Lite · native Rust runtime");
    println!();
    println!("Usage:");
    println!("  hara-lite [OPTIONS] repl");
    println!("  hara-lite [OPTIONS] eval EXPRESSION");
    println!("  hara-lite [OPTIONS] run FILE");
    println!("  hara-lite [OPTIONS] stdin");
    println!("  hara-lite [OPTIONS] headless");
    println!("  hara-lite remote HOST:PORT");
    println!();
    println!("Options:");
    println!("  --project PATH, --root PATH");
    println!("  --allow-file, --allow-net, --allow-process, --allow-postgres");
    println!("  --no-history, --no-splash, --no-color");
}

pub(crate) fn error_exit_code(error: &str) -> i32 {
    if error.starts_with("unknown ")
        || error.starts_with("usage:")
        || error.starts_with("unavailable:")
        || error.starts_with("--offline cannot")
        || error.contains(" requires ")
        || error.contains("cannot read")
        || error.contains("Cannot read")
        || error.contains("not found")
    {
        cli_app::CliOutcome::UsageError.exit_code()
    } else {
        cli_app::CliOutcome::Failed.exit_code()
    }
}

pub(crate) fn usage() {
    let program = "hara";
    println!("Hara CLI · Rust runtime");
    println!();
    println!("Usage:");
    println!("  {program} [OPTIONS] repl");
    println!("  {program} eval EXPRESSION | run FILE | stdin");
    println!("  {program} server | remote HOST:PORT");
    println!("  {program} project <new|check|run|test|add|remove|sync|update> ...");
    println!("  {program} manage OPERATION [NAMESPACE ...] [OPTIONS] [--write]");
    println!("  {program} seedgen <root|list|incomplete|benchadd> [LANGUAGE]");
    println!("  {program} package <COMMAND> ...");
    println!("  {program} id <login|enroll|status|key|namespace> ...");
    println!(
        "  {program} asset <check|build|inspect|publish|status|search|info|pull|sync|yank> ..."
    );
    println!("  {program} tap <bootstrap|init|add|remove|list|verify|mirror> ...");
    println!("  {program} spec <COMMAND> ...");
    println!("  {program} snapshot <build|verify|inspect|diff> ...");
    println!("  {program} extension <check|build|install|test> ...");
    println!();
    println!("Compatibility aliases:");
    println!("  new check test add remove sync update headless standalone");
    println!();
    println!("Global options:");
    println!("  --project PATH, --root PATH, --offline");
    println!("  --allow-file, --allow-net, --allow-process, --allow-postgres");
    println!("  --host HOST, --port PORT, --history PATH");
    println!("  --no-history, --no-splash, --no-color, --log-requests");
}

pub(crate) fn exit_error(message: &str, status: i32) -> ! {
    eprintln!("hara: {message}");
    std::process::exit(status)
}

#[cfg(test)]
mod spec_tests {
    use super::build::{
        canonical_build_form, canonical_build_from_edn, read_build_source, write_build_surface,
    };
    use super::build_check::{
        build_obligation_report, build_report_status, check_build, check_build_graph,
    };
    use super::error_exit_code;
    use super::form::{keyword, map_form, map_get};
    use super::metaspec::{
        lint_metaspec, metaspec_report, metaspec_template, read_spec_document,
        validate_against_metaspec, verify_metaspec, METASPEC_REQUIRED_KEYS,
    };
    use super::spec::check_contribution;
    use hara_wasm::cli_app;
    use hara_wasm::kernel::{parse, Form};
    use std::fs;
    use std::path::Path;

    #[test]
    fn offline_daemon_rejection_is_a_usage_error() {
        assert_eq!(
            error_exit_code("--offline cannot be used with headless"),
            cli_app::CliOutcome::UsageError.exit_code()
        );
    }

    #[test]
    fn generated_metaspec_template_lints_cleanly() {
        assert!(lint_metaspec(&metaspec_template()).is_empty());
    }

    #[test]
    fn missing_keys_have_agent_repair_actions() {
        let document = parse("{}").unwrap();
        let findings = lint_metaspec(&document);
        assert_eq!(findings.len(), METASPEC_REQUIRED_KEYS.len());
        assert_eq!(findings[0].rule, "tool.metaspec.rule/required-key");
        assert_eq!(
            findings[0].repair,
            map_form(vec![
                ("action/type", keyword("add-key")),
                ("action/path", Form::Vector(vec![])),
                ("action/key", keyword("document/id")),
            ])
        );
    }

    #[test]
    fn duplicate_ids_and_map_keys_are_not_silently_overwritten() {
        assert!(
            read_spec_document("{:document/id :demo/spec :document/id :demo/other}")
                .unwrap_err()
                .contains("Duplicate key")
        );
        let document = read_spec_document(
            "{:document/id :demo/spec
              :meta/schemas [{:schema/id :demo/value}
                             {:schema/id :demo/value}]}",
        )
        .unwrap();
        let rules = lint_metaspec(&document)
            .into_iter()
            .map(|finding| finding.rule)
            .collect::<Vec<_>>();
        assert!(rules.contains(&"tool.metaspec.rule/duplicate-id"));
    }

    #[test]
    fn unresolved_schema_references_fail_verification() {
        let mut document = metaspec_template();
        let Form::Map(entries) = &mut document else {
            unreachable!()
        };
        entries.push((
            keyword("example/schema-use"),
            map_form(vec![("schema/ref", keyword("missing/schema"))]),
        ));
        let findings = verify_metaspec(&document, Path::new("metaspec.edn"));
        assert!(findings
            .iter()
            .any(|finding| finding.rule == "tool.metaspec.rule/schema-reference"));
        let report = metaspec_report(&document, &findings);
        assert_eq!(map_get(&report, "report/status"), Some(&keyword("fail")));
    }

    #[test]
    fn greenways_buildspec_validates_against_artifact_metaspec() {
        let Some(specs_root) = hara_wasm::spec_registry::root() else {
            eprintln!("skipping: hara-specs-registry is unavailable");
            return;
        };
        let metaspec_path = specs_root.join("00-unsorted/artifact/metaspec/artifact-metaspec.edn");
        if !metaspec_path.is_file() {
            eprintln!("skipping: hara-specs-registry sibling repo not present");
            return;
        }
        let document_path = specs_root
            .join("00-unsorted/contrib/greenways/build/spec/draft/greenways-buildspec.edn");
        let document = read_spec_document(&fs::read_to_string(&document_path).unwrap()).unwrap();
        let metaspec = read_spec_document(&fs::read_to_string(metaspec_path).unwrap()).unwrap();
        assert!(validate_against_metaspec(&document, &metaspec, &document_path).is_empty());
    }

    #[test]
    fn build_surface_normalizes_to_exact_canonical_edn() {
        let Some(specs_root) = hara_wasm::spec_registry::root() else {
            eprintln!("skipping: hara-specs-registry is unavailable");
            return;
        };
        let contributions = specs_root.join("00-unsorted/contrib");
        let source_path = contributions.join("greenways/build/examples/minimal-build.hal");
        let edn_path = contributions.join("greenways/build/examples/minimal-build.edn");
        let source = fs::read_to_string(&source_path).unwrap();
        let canonical = read_spec_document(&fs::read_to_string(edn_path).unwrap()).unwrap();
        let (build, findings) = read_build_source(&source, source_path.to_str().unwrap()).unwrap();
        assert!(findings.is_empty());
        assert_eq!(canonical_build_form(&build), canonical);
    }

    #[test]
    fn build_edn_surface_round_trip_is_semantically_exact() {
        let canonical = read_spec_document(
            "{:greenways/type :build :greenways/version \"0.1.0\"
              :build/id :demo
              :build/artifact {:artifact/kind :demo/output
                               :artifact/output \"dist/demo.hal\"}
              :build/specs []
              :build/stages
              [{:stage/id :source :stage/requires []
                :stage/produces :demo/source :stage/checkers []}
               {:stage/id :output :stage/requires [:source]
                :stage/produces :demo/output :stage/checkers []}]}",
        )
        .unwrap();
        let (build, _) = canonical_build_from_edn(&canonical).unwrap();
        let surface = write_build_surface(&build);
        let (round_trip, findings) = read_build_source(&surface, "round-trip.hal").unwrap();
        assert!(findings.is_empty());
        assert_eq!(canonical_build_form(&round_trip), canonical);
    }

    #[test]
    fn build_cycle_and_blocked_checker_reports_are_structured() {
        let Some(specs_root) = hara_wasm::spec_registry::root() else {
            eprintln!("skipping: hara-specs-registry is unavailable");
            return;
        };
        let contributions = specs_root.join("00-unsorted/contrib");
        let cycle_path = contributions.join("greenways/build/examples/invalid-cycle.hal");
        let (cycle, parse_findings) = read_build_source(
            &fs::read_to_string(&cycle_path).unwrap(),
            cycle_path.to_str().unwrap(),
        )
        .unwrap();
        assert!(parse_findings.is_empty());
        let graph_findings = check_build_graph(&cycle);
        assert!(graph_findings.iter().any(|finding| {
            finding.kind == "greenways/dependency-cycle"
                && finding.message.contains("parse → emit → analyze → parse")
        }));

        let checker_path = contributions.join("greenways/build/examples/invalid-checker.hal");
        let (checker_build, _) = read_build_source(
            &fs::read_to_string(&checker_path).unwrap(),
            checker_path.to_str().unwrap(),
        )
        .unwrap();
        let findings = check_build(&checker_build);
        assert!(findings
            .iter()
            .any(|finding| finding.kind == "greenways/checker-commit"));
        let report = build_obligation_report(&checker_build, &findings);
        assert_eq!(build_report_status(&report), "blocked");
    }

    #[test]
    fn greenways_contribution_envelopes_verify_offline() {
        let Some(specs_root) = hara_wasm::spec_registry::root() else {
            eprintln!("skipping: hara-specs-registry is unavailable");
            return;
        };
        if !specs_root.join("00-unsorted/artifact/metaspec").is_dir() {
            eprintln!("skipping: hara-specs-registry sibling repo not present");
            return;
        }
        for path in [
            "greenways/build",
            "greenways/supersonic",
            "greenways/usdskel",
        ] {
            let root = specs_root.join("00-unsorted/contrib").join(path);
            let envelope =
                read_spec_document(&fs::read_to_string(root.join("CONTRIBUTION.edn")).unwrap())
                    .unwrap();
            assert!(
                check_contribution(&envelope, &root, &specs_root).is_empty(),
                "{path} did not verify"
            );
        }
    }
}
