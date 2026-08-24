use super::build::build_spec_command;
use super::build_check::{valid_full_git_sha, valid_github_repository};
use super::exit_error;
use super::form::{keyword, keyword_name, map_form, map_get, string, string_value};
use super::metaspec::{
    lint_metaspec, metaspec_report, metaspec_template, print_metaspec_text, read_spec_document,
    spec_format, validate_against_metaspec, verify_metaspec, SpecFormat,
};
use hara_wasm::kernel::Form;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SpecFinding {
    pub(crate) rule: &'static str,
    pub(crate) requirement: &'static str,
    pub(crate) path: Vec<Form>,
    pub(crate) message: String,
    pub(crate) repair: Form,
}

pub(crate) fn finding(
    rule: &'static str,
    requirement: &'static str,
    path: Vec<Form>,
    message: impl Into<String>,
    repair: Form,
) -> SpecFinding {
    SpecFinding {
        rule,
        requirement,
        path,
        message: message.into(),
        repair,
    }
}

pub(crate) fn spec_command(args: &[String]) -> Result<(), String> {
    let operation = args
        .first()
        .ok_or_else(|| {
            "spec requires lint, verify, validate, template, check-contribution, check, to-edn, from-edn, normalize, graph, or obligations"
                .to_owned()
        })?;
    if operation == "template" {
        if args.len() != 1 {
            exit_error("spec template accepts no file", 2);
        }
        println!("{}", metaspec_template());
        return Ok(());
    }
    if operation == "validate" {
        return spec_validate_command(&args[1..]);
    }
    if operation == "check-contribution" {
        return check_contribution_command(&args[1..]);
    }
    if matches!(
        operation.as_str(),
        "check" | "to-edn" | "from-edn" | "normalize" | "graph" | "obligations"
    ) {
        return build_spec_command(operation, &args[1..]);
    }
    if !matches!(operation.as_str(), "lint" | "verify") {
        exit_error(
            &format!(
                "spec {operation} is not implemented yet; use lint, verify, validate, template, or check-contribution"
            ),
            2,
        );
    }
    let path = args
        .get(1)
        .ok_or_else(|| format!("spec {operation} requires FILE"))?;
    let format = spec_format(&args[2..]).unwrap_or_else(|error| exit_error(&error, 2));
    let source = fs::read_to_string(path)
        .unwrap_or_else(|error| exit_error(&format!("cannot read {path}: {error}"), 2));
    let document = read_spec_document(&source)
        .unwrap_or_else(|error| exit_error(&format!("{path}: {error}"), 2));
    let mut findings = lint_metaspec(&document);
    if operation == "verify" {
        findings.extend(verify_metaspec(&document, Path::new(path)));
    }
    let report = metaspec_report(&document, &findings);
    match format {
        SpecFormat::Edn => println!("{report}"),
        SpecFormat::Text => print_metaspec_text(&document, &findings),
    }
    if findings.is_empty() {
        Ok(())
    } else {
        std::process::exit(1)
    }
}

fn check_contribution_command(args: &[String]) -> Result<(), String> {
    let contribution_root = args
        .first()
        .unwrap_or_else(|| exit_error("spec check-contribution requires DIRECTORY", 2));
    let format = spec_format(&args[1..]).unwrap_or_else(|error| exit_error(&error, 2));
    let contribution_root = Path::new(contribution_root);
    if !contribution_root.is_dir() {
        exit_error(
            &format!(
                "contribution path is not a directory: {}",
                contribution_root.display()
            ),
            2,
        );
    }
    let envelope_path = contribution_root.join("CONTRIBUTION.edn");
    let source = fs::read_to_string(&envelope_path).unwrap_or_else(|error| {
        exit_error(
            &format!("cannot read {}: {error}", envelope_path.display()),
            2,
        )
    });
    let envelope = read_spec_document(&source)
        .unwrap_or_else(|error| exit_error(&format!("{}: {error}", envelope_path.display()), 2));
    let specs_root = hara_wasm::spec_registry::root()
        .unwrap_or_else(|| exit_error("cannot locate hara-specs-registry", 2));
    let findings = check_contribution(&envelope, contribution_root, &specs_root);
    let report = contribution_report(&envelope, &findings);
    match format {
        SpecFormat::Edn => println!("{report}"),
        SpecFormat::Text => print_contribution_text(&envelope, &findings),
    }
    if findings.is_empty() {
        Ok(())
    } else {
        std::process::exit(1)
    }
}

pub(crate) fn check_contribution(
    envelope: &Form,
    contribution_root: &Path,
    specs_root: &Path,
) -> Vec<SpecFinding> {
    let mut findings = Vec::new();
    for key in [
        "contribution/id",
        "contribution/owner",
        "contribution/version",
        "contribution/status",
        "contribution/title",
        "contribution/summary",
        "contribution/source",
        "contribution/specs",
    ] {
        if map_get(envelope, key).is_none() {
            findings.push(finding(
                "hara.contribution.rule/required-key",
                "hara.contribution/required-fields",
                vec![],
                format!("Missing required contribution key :{key}"),
                map_form(vec![
                    ("action/type", keyword("add-key")),
                    ("action/key", keyword(key)),
                ]),
            ));
        }
    }
    let contribution_id = map_get(envelope, "contribution/id").and_then(keyword_name);
    let owner = map_get(envelope, "contribution/owner").and_then(keyword_name);
    if let (Some(id), Some(owner)) = (&contribution_id, &owner) {
        if !id.starts_with(&format!("{owner}/")) {
            findings.push(finding(
                "hara.contribution.rule/owner-qualified-id",
                "hara.contribution/owner-qualified-identifiers",
                vec![keyword("contribution/id")],
                format!("Contribution ID :{id} is not owned by :{owner}"),
                map_form(vec![("action/type", keyword("use-owner-qualified-id"))]),
            ));
        }
    }
    let status = map_get(envelope, "contribution/status").and_then(keyword_name);
    if !matches!(
        status.as_deref(),
        Some("draft" | "candidate" | "stable" | "deprecated" | "scaffold")
    ) {
        findings.push(finding(
            "hara.contribution.rule/status",
            "hara.contribution/known-status",
            vec![keyword("contribution/status")],
            "Contribution status must be :draft, :candidate, :stable, :deprecated, or :scaffold",
            map_form(vec![("action/type", keyword("select-status"))]),
        ));
    }
    check_contribution_source(envelope, &mut findings);
    let specs = match map_get(envelope, "contribution/specs") {
        Some(Form::Vector(specs)) => specs,
        Some(_) => {
            findings.push(finding(
                "hara.contribution.rule/spec-list",
                "hara.contribution/spec-list",
                vec![keyword("contribution/specs")],
                ":contribution/specs must be a vector",
                map_form(vec![("action/type", keyword("replace-with-vector"))]),
            ));
            return findings;
        }
        None => return findings,
    };
    if status.as_deref() != Some("scaffold") && specs.is_empty() {
        findings.push(finding(
            "hara.contribution.rule/spec-required",
            "hara.contribution/normative-spec",
            vec![keyword("contribution/specs")],
            "A non-scaffold contribution must contain at least one specification",
            map_form(vec![("action/type", keyword("add-specification"))]),
        ));
    }
    for (index, spec) in specs.iter().enumerate() {
        check_contribution_spec(spec, index, contribution_root, specs_root, &mut findings);
    }
    findings
}

fn check_contribution_source(envelope: &Form, findings: &mut Vec<SpecFinding>) {
    let Some(source) = map_get(envelope, "contribution/source") else {
        return;
    };
    let source_path = vec![keyword("contribution/source")];
    if map_get(source, "source/provider") != Some(&keyword("github")) {
        findings.push(finding(
            "hara.contribution.rule/source-provider",
            "hara.contribution/github-source",
            source_path.clone(),
            "Contribution source provider must be :github",
            map_form(vec![
                ("action/type", keyword("set-value")),
                ("action/value", keyword("github")),
            ]),
        ));
    }
    let repository = map_get(source, "source/repository").and_then(string_value);
    if !repository.as_deref().is_some_and(valid_github_repository) {
        findings.push(finding(
            "hara.contribution.rule/source-repository",
            "hara.contribution/github-source",
            source_path.clone(),
            "Contribution source repository must be owner/name",
            map_form(vec![("action/type", keyword("set-repository"))]),
        ));
    }
    let commit = map_get(source, "source/commit").and_then(string_value);
    if !commit.as_deref().is_some_and(valid_full_git_sha) {
        findings.push(finding(
            "hara.contribution.rule/source-commit",
            "hara.contribution/immutable-source",
            source_path.clone(),
            "Contribution source commit must be a full 40-character Git SHA",
            map_form(vec![("action/type", keyword("resolve-commit-sha"))]),
        ));
    }
    let path = map_get(source, "source/path").and_then(string_value);
    if !path.as_deref().is_some_and(safe_relative_path) {
        findings.push(finding(
            "hara.contribution.rule/source-path",
            "hara.contribution/repository-relative-paths",
            source_path,
            "Contribution source path must be repository-relative",
            map_form(vec![("action/type", keyword("set-relative-path"))]),
        ));
    }
}

fn check_contribution_spec(
    spec: &Form,
    index: usize,
    contribution_root: &Path,
    specs_root: &Path,
    findings: &mut Vec<SpecFinding>,
) {
    let path_prefix = vec![keyword("contribution/specs"), Form::Number(index as i64)];
    for key in [
        "spec/id",
        "spec/version",
        "spec/path",
        "spec/metaspec",
        "spec/sha256",
    ] {
        if map_get(spec, key).is_none() {
            findings.push(finding(
                "hara.contribution.rule/spec-field",
                "hara.contribution/spec-reference",
                path_prefix.clone(),
                format!("Specification reference is missing :{key}"),
                map_form(vec![
                    ("action/type", keyword("add-key")),
                    ("action/key", keyword(key)),
                ]),
            ));
        }
    }
    let Some(relative_path) = map_get(spec, "spec/path").and_then(string_value) else {
        return;
    };
    if !safe_relative_path(&relative_path) {
        findings.push(finding(
            "hara.contribution.rule/spec-path",
            "hara.contribution/repository-relative-paths",
            path_prefix,
            "Specification path must remain inside its contribution",
            map_form(vec![("action/type", keyword("set-relative-path"))]),
        ));
        return;
    }
    let document_path = contribution_root.join(&relative_path);
    let bytes = match fs::read(&document_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            findings.push(finding(
                "hara.contribution.rule/spec-readable",
                "hara.contribution/spec-readable",
                path_prefix,
                format!("Cannot read {}: {error}", document_path.display()),
                map_form(vec![("action/type", keyword("create-or-repair-file"))]),
            ));
            return;
        }
    };
    let expected_digest = map_get(spec, "spec/sha256").and_then(string_value);
    let actual_digest = format!("sha256:{:x}", Sha256::digest(&bytes));
    if expected_digest.as_deref() != Some(actual_digest.as_str()) {
        findings.push(finding(
            "hara.contribution.rule/spec-digest",
            "hara.contribution/content-addressed-spec",
            path_prefix.clone(),
            format!("Specification digest mismatch; actual digest is {actual_digest}"),
            map_form(vec![
                ("action/type", keyword("set-value")),
                ("action/key", keyword("spec/sha256")),
                ("action/value", string(actual_digest)),
            ]),
        ));
    }
    let document_source = match String::from_utf8(bytes) {
        Ok(source) => source,
        Err(_) => {
            findings.push(finding(
                "hara.contribution.rule/spec-edn",
                "hara.contribution/spec-readable",
                path_prefix,
                "Specification is not UTF-8 EDN",
                map_form(vec![("action/type", keyword("rewrite-as-edn"))]),
            ));
            return;
        }
    };
    let document = match read_spec_document(&document_source) {
        Ok(document) => document,
        Err(error) => {
            findings.push(finding(
                "hara.contribution.rule/spec-edn",
                "hara.contribution/spec-readable",
                path_prefix,
                format!("Specification cannot be read: {error}"),
                map_form(vec![("action/type", keyword("repair-edn"))]),
            ));
            return;
        }
    };
    if map_get(spec, "spec/id") != map_get(&document, "document/id") {
        findings.push(finding(
            "hara.contribution.rule/spec-id",
            "hara.contribution/spec-reference",
            path_prefix.clone(),
            "Envelope :spec/id does not match specification :document/id",
            map_form(vec![("action/type", keyword("align-document-id"))]),
        ));
    }
    if map_get(spec, "spec/version") != map_get(&document, "document/version") {
        findings.push(finding(
            "hara.contribution.rule/spec-version",
            "hara.contribution/spec-reference",
            path_prefix.clone(),
            "Envelope :spec/version does not match specification :document/version",
            map_form(vec![("action/type", keyword("align-document-version"))]),
        ));
    }
    let Some(metaspec_path) = map_get(spec, "spec/metaspec").and_then(string_value) else {
        return;
    };
    if !safe_relative_path(&metaspec_path) {
        findings.push(finding(
            "hara.contribution.rule/metaspec-path",
            "hara.contribution/repository-relative-paths",
            path_prefix,
            "Meta-specification path must be repository-relative",
            map_form(vec![("action/type", keyword("set-relative-path"))]),
        ));
        return;
    }
    let metaspec_path = specs_root.join(
        metaspec_path
            .strip_prefix("specs/")
            .unwrap_or(&metaspec_path),
    );
    let metaspec_source = match fs::read_to_string(&metaspec_path) {
        Ok(source) => source,
        Err(error) => {
            findings.push(finding(
                "hara.contribution.rule/metaspec-readable",
                "hara.contribution/metaspec-conformance",
                path_prefix,
                format!("Cannot read {}: {error}", metaspec_path.display()),
                map_form(vec![("action/type", keyword("repair-metaspec-reference"))]),
            ));
            return;
        }
    };
    let metaspec = match read_spec_document(&metaspec_source) {
        Ok(metaspec) => metaspec,
        Err(error) => {
            findings.push(finding(
                "hara.contribution.rule/metaspec-readable",
                "hara.contribution/metaspec-conformance",
                path_prefix,
                format!("Meta-specification cannot be read: {error}"),
                map_form(vec![("action/type", keyword("repair-metaspec"))]),
            ));
            return;
        }
    };
    for meta_finding in validate_against_metaspec(&document, &metaspec, &document_path) {
        findings.push(meta_finding);
    }
}

fn safe_relative_path(value: &str) -> bool {
    let path = Path::new(value);
    !value.is_empty()
        && !path.is_absolute()
        && path.components().all(|component| {
            matches!(
                component,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
}

fn contribution_report(envelope: &Form, findings: &[SpecFinding]) -> Form {
    let status = if findings.is_empty() { "pass" } else { "fail" };
    let finding_forms = findings
        .iter()
        .map(|finding| {
            map_form(vec![
                ("finding/id", keyword(finding.rule)),
                ("requirement/id", keyword(finding.requirement)),
                ("finding/level", keyword("error")),
                ("finding/path", Form::Vector(finding.path.clone())),
                ("finding/message", string(&finding.message)),
                ("finding/repair", finding.repair.clone()),
            ])
        })
        .collect();
    map_form(vec![
        ("report/type", keyword("hara/contribution-check")),
        ("report/version", string("0.1.0")),
        (
            "contribution/id",
            map_get(envelope, "contribution/id")
                .cloned()
                .unwrap_or(Form::Nil),
        ),
        ("report/status", keyword(status)),
        (
            "summary",
            map_form(vec![
                (
                    "summary/pass",
                    Form::Number(if findings.is_empty() { 1 } else { 0 }),
                ),
                ("summary/fail", Form::Number(findings.len() as i64)),
                ("summary/unknown", Form::Number(0)),
                ("summary/blocked", Form::Number(0)),
            ]),
        ),
        ("findings", Form::Vector(finding_forms)),
        (
            "next-actions",
            Form::Vector(
                findings
                    .iter()
                    .map(|finding| finding.repair.clone())
                    .collect(),
            ),
        ),
    ])
}

fn print_contribution_text(envelope: &Form, findings: &[SpecFinding]) {
    let id = map_get(envelope, "contribution/id")
        .map(ToString::to_string)
        .unwrap_or_else(|| "<unknown>".into());
    if findings.is_empty() {
        println!("PASS {id}");
    } else {
        println!("FAIL {id} ({} findings)", findings.len());
        for finding in findings {
            println!("  {} — {}", finding.rule, finding.message);
        }
    }
}

fn spec_validate_command(args: &[String]) -> Result<(), String> {
    let path = args
        .first()
        .unwrap_or_else(|| exit_error("spec validate requires FILE --against METASPEC", 2));
    if args.get(1).map(String::as_str) != Some("--against") {
        exit_error("spec validate requires FILE --against METASPEC", 2);
    }
    let metaspec_path = args
        .get(2)
        .unwrap_or_else(|| exit_error("spec validate requires FILE --against METASPEC", 2));
    let format = spec_format(&args[3..]).unwrap_or_else(|error| exit_error(&error, 2));
    let source = fs::read_to_string(path)
        .unwrap_or_else(|error| exit_error(&format!("cannot read {path}: {error}"), 2));
    let metaspec_source = fs::read_to_string(metaspec_path)
        .unwrap_or_else(|error| exit_error(&format!("cannot read {metaspec_path}: {error}"), 2));
    let document = read_spec_document(&source)
        .unwrap_or_else(|error| exit_error(&format!("{path}: {error}"), 2));
    let metaspec = read_spec_document(&metaspec_source)
        .unwrap_or_else(|error| exit_error(&format!("{metaspec_path}: {error}"), 2));
    let meta_findings = lint_metaspec(&metaspec);
    if !meta_findings.is_empty() {
        exit_error("the --against meta-spec does not pass structural lint", 2);
    }
    let findings = validate_against_metaspec(&document, &metaspec, Path::new(path));
    let report = metaspec_report(&document, &findings);
    match format {
        SpecFormat::Edn => println!("{report}"),
        SpecFormat::Text => print_metaspec_text(&document, &findings),
    }
    if findings.is_empty() {
        Ok(())
    } else {
        std::process::exit(1)
    }
}

pub(crate) fn spec_finding_form(finding: &SpecFinding) -> Form {
    map_form(vec![
        ("finding/id", keyword(finding.rule)),
        ("rule/id", keyword(finding.rule)),
        ("requirement/id", keyword(finding.requirement)),
        ("finding/level", keyword("error")),
        ("finding/path", Form::Vector(finding.path.clone())),
        ("finding/message", string(&finding.message)),
        ("finding/repair", finding.repair.clone()),
    ])
}
