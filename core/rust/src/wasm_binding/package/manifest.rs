use std::collections::BTreeMap;

use crate::kernel::Form;
use crate::wasm_binding::CancellationPolicy;

use super::{
    digest, document, keyword_form, string_form, BindingTarget, WasmInterface, ADAPTER_FILE,
    GENERATED_VERSION,
};

pub(super) fn package_document(
    interface: &WasmInterface,
    target: BindingTarget,
    identity: &str,
    has_adapter: bool,
    files: &BTreeMap<String, Vec<u8>>,
) -> Result<String, String> {
    let file_entries = files
        .iter()
        .map(|(path, bytes)| {
            (
                string_form(path),
                Form::Map(vec![
                    (keyword_form("sha256"), string_form(&digest(bytes))),
                    (keyword_form("size"), Form::Number(bytes.len() as i64)),
                ]),
            )
        })
        .collect();
    let exports = interface
        .exports
        .iter()
        .map(|export| keyword_form(&export.name))
        .collect();
    let (artifact_path, artifact_type, entry_point): (&str, &str, String) = if target
        == BindingTarget::HtaV1
    {
        (
            if has_adapter {
                ADAPTER_FILE
            } else {
                &interface.module
            },
            "hta",
            "hta_start".into(),
        )
    } else {
        let entry_point = interface
            .exports
            .first()
            .map(|export| export.wasm_export.clone())
            .ok_or_else(|| "wasm-binding/package-invalid: package requires an export".to_owned())?;
        (&interface.module, "wasm", entry_point)
    };
    let module_bytes = files.get(artifact_path).ok_or_else(|| {
        format!(
            "wasm-binding/package-invalid: missing artifact {}",
            artifact_path
        )
    })?;
    let lifecycle = Form::Map(vec![
        (
            keyword_form("lifecycle/load"),
            keyword_form("idempotent"),
        ),
        (
            keyword_form("lifecycle/close"),
            keyword_form("idempotent"),
        ),
        (
            keyword_form("lifecycle/session-isolation"),
            Form::Bool(true),
        ),
        (
            keyword_form("lifecycle/async"),
            Form::Bool(target == BindingTarget::HtaV1),
        ),
        (
            keyword_form("lifecycle/cancellation"),
            Form::Bool(
                interface.exports.iter().any(|export| {
                    export.cancellation.is_some_and(|policy| {
                        !matches!(policy, CancellationPolicy::Ignore)
                    })
                }),
            ),
        ),
    ]);
    let host_calls = interface
        .host_calls
        .iter()
        .flat_map(|(service, contract)| {
            contract
                .methods
                .iter()
                .map(move |method| format!("{service}/{method}"))
        })
        .map(|call| string_form(&call))
        .collect();
    let artifact = Form::Map(vec![
        (
            keyword_form("variant/artifact"),
            Form::Map(vec![
                (keyword_form("artifact/type"), keyword_form(artifact_type)),
                (keyword_form("artifact/path"), string_form(artifact_path)),
                (
                    keyword_form("artifact/sha256"),
                    string_form(&digest(module_bytes)),
                ),
                (
                    keyword_form("artifact/target"),
                    string_form("wasm32-wasi-preview1"),
                ),
                (
                    keyword_form("artifact/abi"),
                    string_form(target.as_keyword()),
                ),
                (
                    keyword_form("artifact/entry-point"),
                    string_form(&entry_point),
                ),
            ]),
        ),
        (
            keyword_form("variant/required-capabilities"),
            Form::Set(
                interface
                    .capabilities
                    .iter()
                    .map(|capability| keyword_form(capability))
                    .collect(),
            ),
        ),
        (keyword_form("variant/host-calls"), Form::Set(host_calls)),
        (keyword_form("variant/exports"), Form::Set(exports)),
        (keyword_form("variant/lifecycle"), lifecycle),
    ]);
    let project_bytes = files
        .get("project.edn")
        .map(Vec::as_slice)
        .unwrap_or_default();
    let provenance = digest(project_bytes);
    Ok(document(Form::Map(vec![
        (keyword_form("harp/format"), string_form("0.0.0-alpha")),
        (
            keyword_form("package"),
            Form::Map(vec![
                (keyword_form("identity"), string_form(identity)),
                (keyword_form("version"), string_form(GENERATED_VERSION)),
                (
                    keyword_form("provenance"),
                    Form::Map(vec![
                        (
                            keyword_form("repository"),
                            string_form("generated/hara-wasm-bindgen"),
                        ),
                        (
                            keyword_form("commit"),
                            string_form(provenance.trim_start_matches("sha256:")),
                        ),
                    ]),
                ),
            ]),
        ),
        (keyword_form("files"), Form::Map(file_entries)),
        (
            keyword_form("wasm-imports"),
            Form::Map(vec![(keyword_form(&interface.namespace), artifact)]),
        ),
    ])))
}
