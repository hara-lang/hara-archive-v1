use hara_wasm::kernel::{parse, Form};

fn form_keyword<'a>(form: &'a Form, name: &str) -> Option<&'a Form> {
    let Form::Map(entries) = form else {
        return None;
    };
    entries
        .iter()
        .find(|(key, _)| matches!(key, Form::Keyword(value) if value == name))
        .map(|(_, value)| value)
}

fn form_count(form: &Form, name: &str, fallback: usize) -> Result<usize, String> {
    match form_keyword(form, name) {
        Some(Form::Number(value)) => usize::try_from(*value)
            .map_err(|_| format!("test summary :{name} must be non-negative")),
        Some(Form::BigInteger(value)) => value
            .try_into()
            .map_err(|_| format!("test summary :{name} is outside usize range")),
        Some(_) => Err(format!("test summary :{name} must be an integer")),
        None => Ok(fallback),
    }
}

fn direct_test_results(results: &[Form]) -> Result<(usize, usize), String> {
    let mut passed = 0;
    let mut failed = 0;
    for result in results {
        let Form::Tagged(tag, fields) = result else {
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
    Ok((passed, failed))
}

fn structured_test_results(summary: &Form) -> Result<(usize, usize), String> {
    let status = form_keyword(summary, "status")
        .ok_or_else(|| "code.test summary is missing :status".to_owned())?;
    let Form::Keyword(status) = status else {
        return Err("code.test summary :status must be a keyword".into());
    };
    let counts = form_keyword(summary, "counts")
        .ok_or_else(|| "code.test summary is missing :counts".to_owned())?;
    if !matches!(counts, Form::Map(_)) {
        return Err("code.test summary :counts must be a map".into());
    }
    let passed_facts = form_count(counts, "passed", 0)?;
    let failed_facts = form_count(counts, "failed", 0)?;
    let errors = form_count(counts, "error", 0)?;
    let timeouts = form_count(counts, "timeout", 0)?;
    let passed = form_count(summary, "passed", passed_facts)?;
    let mut failed = form_count(summary, "failed", failed_facts + errors + timeouts)?;
    if status != "passed" && failed == 0 {
        // Preserve a failing structured outcome even when a runner reports no
        // failed assertion count (for example a cancelled execution).
        failed = 1;
    }
    Ok((passed, failed))
}

fn parsed_test_results(form: &Form) -> Result<(usize, usize), String> {
    match form {
        Form::Vector(results) | Form::List(results) => direct_test_results(results),
        Form::Map(_) => structured_test_results(form),
        _ => Err("test file must return a direct result vector/list or a code.test summary".into()),
    }
}

pub(super) fn test_results(value: &str) -> Result<(usize, usize), String> {
    let parsed = parse(value)?;
    match parsed {
        // Encoded strings remain representation compatibility only. New test
        // files return vectors/lists or code.test summaries directly.
        Form::String(source) => parsed_test_results(&parse(&source)?),
        form => parsed_test_results(&form),
    }
}

#[cfg(test)]
mod tests {
    use super::test_results;

    #[test]
    fn accepts_direct_vectors_and_lists() {
        let result = |value: &str| format!("#{}", value);
        assert_eq!(
            test_results(&format!(
                "[{} {}]",
                result("hara/Result[:success true nil {}]"),
                result("hara/Result[:success false nil {}]")
            ))
            .unwrap(),
            (1, 1)
        );
        assert_eq!(
            test_results(&format!(
                "({})",
                result("hara/Result[:success true nil {}]")
            ))
            .unwrap(),
            (1, 0)
        );
    }

    #[test]
    fn retains_encoded_vector_compatibility() {
        let encoded = format!("[{}]", format!("#{}", "hara/Result[:success true nil {}]"));
        assert_eq!(test_results(&format!("{:?}", encoded)).unwrap(), (1, 0));
    }

    #[test]
    fn accepts_structured_code_test_summaries() {
        assert_eq!(
            test_results(
                "{:status :passed :counts {:passed 2 :failed 0 :error 0 :timeout 0} :passed 3 :failed 0}"
            )
            .unwrap(),
            (3, 0)
        );
        assert_eq!(
            test_results("{:status :failed :counts {:passed 1 :failed 0 :error 1 :timeout 0}}")
                .unwrap(),
            (1, 1)
        );
    }
}
