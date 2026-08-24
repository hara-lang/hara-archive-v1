use super::{parse_forms, read_forms};
use crate::kernel::Form;
use num_bigint::BigInt;
use std::fs;
#[test]
fn tracks_spans_comments_commas_and_reader_macros() {
    let forms = read_forms("; hi\n[1, 2] #'x #_gone :ok").unwrap();
    assert_eq!(forms.len(), 3);
    assert_eq!(forms[0].span.start.line, 2);
    assert!(matches!(&forms[1].form,Form::List(v)if matches!(&v[0],Form::Symbol(s)if s=="var")));
    assert_eq!(forms[2].form, Form::Keyword("ok".into()));
}
#[test]
fn preserves_recursive_source_locations_without_changing_forms() {
    let mut forms = read_forms("  \n[1 {:a\n (f x)}]").unwrap();
    let root = forms.remove(0);
    assert_eq!(root.form.to_string(), "[1 {:a (f x)}]");
    assert_eq!((root.span.start.line, root.span.start.column), (2, 1));
    assert_eq!(root.children.len(), 2);

    let map = &root.children[1];
    assert_eq!(map.form.to_string(), "{:a (f x)}");
    assert_eq!(map.children.len(), 2);
    let call = &map.children[1];
    assert_eq!(call.form.to_string(), "(f x)");
    assert_eq!((call.span.start.line, call.span.start.column), (3, 2));
    assert_eq!(call.children.len(), 2);
    assert_eq!(root.descendants().count(), 6);
}

#[test]
fn preserves_locations_through_dispatch_and_metadata() {
    let root = read_forms("#tag ^:private [#'x]").unwrap().remove(0);
    assert_eq!(root.form.to_string(), "#tag[(var x)]");
    assert_eq!(root.children.len(), 1);
    let metadata = &root.children[0];
    assert_eq!(metadata.children.len(), 2);
    let vector = &metadata.children[1];
    assert_eq!(vector.children.len(), 1);
    assert_eq!(vector.children[0].form.to_string(), "(var x)");
    assert_eq!(root.descendants().count(), 5);
}

#[test]
fn reports_delimited_errors_with_position() {
    let error = parse_forms("[1\n2").unwrap_err();
    assert!(error.contains("line 2"));
    assert!(error.contains("EOF while reading vector"));
}

#[test]
fn matches_canonical_numbers_characters_and_duplicate_errors() {
    assert_eq!(
        parse_forms("123 123.45 0xFF 2r1010 9223372036854775808 1.2300e2").unwrap(),
        vec![
            Form::Number(123),
            Form::Float(123.45),
            Form::Number(255),
            Form::Number(10),
            Form::BigInteger(BigInt::parse_bytes(b"9223372036854775808", 10).unwrap()),
            Form::Float(123.0),
        ]
    );
    for unsupported in [
        "123N", "0N", "+0N", "-0N", "1.2.3M", "123.45M", "0M", "12xN",
    ] {
        let error = parse_forms(unsupported).unwrap_err();
        assert!(
            error.contains("Legacy numeric suffixes N and M are not supported"),
            "{unsupported}: {error}"
        );
    }
    assert_eq!(
        parse_forms("\\newline \\u03bb \\o377 \\( \\) \\[ \\] \\{ \\}").unwrap(),
        vec![
            Form::Character('\n'),
            Form::Character('λ'),
            Form::Character('ÿ'),
            Form::Character('('),
            Form::Character(')'),
            Form::Character('['),
            Form::Character(']'),
            Form::Character('{'),
            Form::Character('}')
        ]
    );
    assert!(parse_forms("{:a 1 :a 2}")
        .unwrap_err()
        .contains("Duplicate key"));
    assert!(parse_forms("#{1 1}")
        .unwrap_err()
        .contains("Duplicate item"));
    assert!(parse_forms("123a").unwrap_err().contains("Invalid number"));
}

#[test]
fn preserves_regex_tagged_and_extended_string_literals() {
    assert_eq!(
        parse_forms("#\"abc\" #math[:tensor 42]").unwrap(),
        vec![
            Form::Regex("abc".into()),
            Form::Tagged(
                "math".into(),
                Box::new(Form::Vector(vec![
                    Form::Keyword("tensor".into()),
                    Form::Number(42)
                ]))
            )
        ]
    );
    assert_eq!(
        parse_forms("\"\\t\\r\\n\\b\\f\\\\\\\"\\7\"").unwrap(),
        vec![Form::String("\t\r\n\u{0008}\u{000c}\\\"\u{0007}".into())]
    );
}
#[test]
fn preserves_metadata_and_rejects_unknown_dispatch_forms() {
    assert_eq!(
        parse_forms("^:private [1]").unwrap(),
        vec![Form::Metadata(
            Box::new(Form::Map(vec![(
                Form::Keyword("private".into()),
                Form::Bool(true)
            )])),
            Box::new(Form::Vector(vec![Form::Number(1)]))
        )]
    );
    assert!(parse_forms("#[1 2]")
        .unwrap_err()
        .contains("No dispatch macro for: ["));
}

#[test]
fn expands_anonymous_function_reader_arguments() {
    assert_eq!(
        parse_forms("#(+ % %2 (count %&))").unwrap(),
        vec![Form::List(vec![
            Form::Symbol("fn".into()),
            Form::Vector(vec![
                Form::Symbol("__reader_fn_0_1".into()),
                Form::Symbol("__reader_fn_0_2".into()),
                Form::Symbol("&".into()),
                Form::Symbol("__reader_fn_0_rest".into()),
            ]),
            Form::List(vec![
                Form::Symbol("+".into()),
                Form::Symbol("__reader_fn_0_1".into()),
                Form::Symbol("__reader_fn_0_2".into()),
                Form::List(vec![
                    Form::Symbol("count".into()),
                    Form::Symbol("__reader_fn_0_rest".into()),
                ]),
            ]),
        ])]
    );
    assert!(parse_forms("#(+ %0 1)")
        .unwrap_err()
        .contains("arguments begin at %1"));
}
#[test]
fn matches_extended_canonical_reader_categories() {
    assert_eq!(
        parse_forms("9223372036854775808").unwrap(),
        vec![Form::BigInteger(
            BigInt::parse_bytes(b"9223372036854775808", 10,).unwrap()
        )]
    );
    assert!(parse_forms("1/2")
        .unwrap_err()
        .contains("Ratios are not supported"));
    assert_eq!(
        parse_forms(r##"#"\d+""##).unwrap(),
        vec![Form::Regex(r"\d+".into())]
    );
    for source in ["##Inf", "##-Inf", "##NaN", "1e309", "-1e309"] {
        assert!(
            parse_forms(source)
                .unwrap_err()
                .contains("non-finite number"),
            "{source}"
        );
    }
    assert!(parse_forms("#'1")
        .unwrap_err()
        .contains("Var quote expects a symbol"));
    assert!(parse_forms("^1 [2]")
        .unwrap_err()
        .contains("Metadata must be"));
    assert!(parse_forms("^:private 1")
        .unwrap_err()
        .contains("Metadata can only be applied"));
}
#[test]
fn allows_multi_slash_symbols_but_not_keywords_like_java() {
    assert_eq!(
        parse_forms("a/b/c").unwrap(),
        vec![Form::Symbol("a/b/c".into())]
    );
    assert!(parse_forms(":a/b/c").unwrap_err().contains("Keyword"));
}

#[test]
fn validates_keywords_and_merges_metadata_like_java() {
    for (invalid, expected) in [
        (":", "cannot be empty"),
        (":/", "single slash"),
        (":/name", "start with a slash"),
        (":name/", "end with a slash"),
        (":a/b/c", "only contain one slash"),
    ] {
        assert!(parse_forms(invalid).unwrap_err().contains(expected));
    }
    assert_eq!(
        parse_forms(":ns/name").unwrap(),
        vec![Form::Keyword("ns/name".into())]
    );
    assert_eq!(
        parse_forms("^:private ^{:tag fast} item").unwrap(),
        vec![Form::Metadata(
            Box::new(Form::Map(vec![
                (Form::Keyword("tag".into()), Form::Symbol("fast".into())),
                (Form::Keyword("private".into()), Form::Bool(true)),
            ])),
            Box::new(Form::Symbol("item".into())),
        )]
    );
    assert_eq!(
        parse_forms("^:ignored :keyword").unwrap(),
        vec![Form::Keyword("keyword".into())]
    );
}

#[test]
fn matches_java_malformed_reader_failures() {
    for (source, expected) in [
        (")", "Unmatched delimiter"),
        ("\"", "EOF while reading string"),
        ("\"\\u12X4\"", "Invalid digit"),
        ("\"\\q\"", "Unsupported escape character"),
        ("\\u12X4", "Invalid digit"),
        ("{:a 1 :b}", "even number of forms"),
        ("#", "EOF while reading hash dispatch"),
    ] {
        let error = parse_forms(source).unwrap_err();
        assert!(error.contains(expected), "{source}: {error}");
    }
}

#[test]
fn supports_dispatch_and_quote_forms() {
    let forms = parse_forms("#{1 2} 'x @y #tag {:a 1}").unwrap();
    assert_eq!(forms.len(), 4);
    assert!(matches!(forms[0], Form::Set(_)));
    assert!(matches!(&forms[3], Form::Tagged(tag, _) if tag == "tag"));
}
#[test]
fn rejects_dispatch_forms_absent_from_the_java_reference() {
    for source in [
        "#:hello{:a 1}",
        "#?(:clj hello)",
        "#?@(:clj [x])",
        "#=(f)",
        "[#|1]",
    ] {
        let error = parse_forms(source).unwrap_err();
        assert!(
            error.contains("No dispatch macro for:"),
            "{source}: {error}"
        );
    }
}

#[test]
fn matches_java_symbol_and_number_macro_termination() {
    assert_eq!(
        parse_forms("1#_2 3").unwrap(),
        vec![Form::Number(1), Form::Number(3)]
    );
    assert_eq!(
        parse_forms("1\u{27}foo").unwrap(),
        vec![
            Form::Number(1),
            Form::List(vec![
                Form::Symbol("quote".into()),
                Form::Symbol("foo".into())
            ])
        ]
    );
    assert_eq!(
        parse_forms("foo\u{27}bar").unwrap(),
        vec![Form::Symbol("foo\u{27}bar".into())]
    );
    assert_eq!(
        parse_forms("foo#bar 1").unwrap(),
        vec![Form::Symbol("foo#bar".into()), Form::Number(1)]
    );
    assert_eq!(
        parse_forms("foo@bar").unwrap(),
        vec![
            Form::Symbol("foo".into()),
            Form::List(vec![
                Form::Symbol("deref".into()),
                Form::Symbol("bar".into())
            ])
        ]
    );
    assert_eq!(
        parse_forms("foo^:tag [1]").unwrap(),
        vec![
            Form::Symbol("foo".into()),
            Form::Metadata(
                Box::new(Form::Map(vec![(
                    Form::Keyword("tag".into()),
                    Form::Bool(true)
                )])),
                Box::new(Form::Vector(vec![Form::Number(1)]))
            )
        ]
    );
}

#[test]
fn shared_reader_corpus_matches_canonical_forms_and_errors() {
    let relative = "01-lang/001-language/draft/conformance/reader.edn";
    let path = crate::spec_registry::resolve(relative).filter(|candidate| candidate.is_file());
    let Some(path) = path else {
        eprintln!(
            "skipping: hara-specs-registry/01-lang/001-language/draft/conformance/reader.edn unavailable (hara-specs-registry sibling repo not present)"
        );
        return;
    };
    let manifest_source = fs::read_to_string(path).unwrap();
    let manifest = parse_forms(&manifest_source).unwrap().remove(0);
    let Form::Map(manifest) = manifest else {
        panic!("reader parity manifest must be a map");
    };
    let cases = map_value(&manifest, "cases").expect("manifest must contain :cases");
    let Form::Vector(cases) = cases else {
        panic!("reader parity :cases must be a vector");
    };

    for case in cases {
        let Form::Map(case) = case else {
            panic!("reader parity case must be a map");
        };
        let id = map_value(case, "id").expect("case must contain :id");
        let id_str = id.to_string();

        // The parser now accepts arbitrary-size integers as BigInteger forms,
        // so the legacy overflow error cases are no longer errors. The legacy
        // N/M suffix cases now produce a dedicated error message that differs
        // from the corpus expectation.
        if matches!(
            id_str.as_str(),
            ":positive-integer-overflow"
                | ":negative-integer-overflow"
                | ":unsupported-bigint"
                | ":unsupported-zero-bigint"
                | ":unsupported-positive-zero-bigint"
                | ":unsupported-negative-zero-bigint"
                | ":unsupported-zero-bigdec"
                | ":unsupported-big-decimal"
                | ":invalid-bigint"
        ) {
            continue;
        }

        let source = string_value(case, "source");
        let readable = map_value(case, "rust-readable").or_else(|| map_value(case, "readable"));
        // Decimal literals now parse as Float and display with the (double ...)
        // constructor prefix. The shared reader corpus still records the old
        // Decimal display; override those cases until the corpus is updated.
        let rust_readable_override = match id_str.as_str() {
            ":numbers" => Some(Form::String(
                "[0 -1 9223372036854775807 (double 1.25) (double 1.25)]".into(),
            )),
            ":floating-point" => Some(Form::String(
                "[(double 0.0) (double 1.0) (double 100.0) (double -0.0125)]".into(),
            )),
            _ => None,
        };
        let readable = rust_readable_override.as_ref().or(readable);
        let expected_error = map_value(case, "rust-error").or_else(|| map_value(case, "error"));

        match (readable, expected_error) {
            (Some(Form::String(readable)), None) => {
                let forms = parse_forms(source)
                    .unwrap_or_else(|error| panic!("{id} unexpectedly failed: {error}"));
                let actual = forms
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" ");
                assert_eq!(&actual, readable, "{id}");
                // Float display uses the `(double ...)` constructor form, which
                // is a valid expression but not a round-trippable literal. Skip
                // the literal round-trip check for cases that now produce floats.
                if !matches!(id_str.as_str(), ":numbers" | ":floating-point") {
                    let round_trip = parse_forms(&actual)
                        .unwrap()
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(" ");
                    assert_eq!(round_trip, actual, "{id} canonical output must round-trip");
                }
            }
            (None, Some(Form::String(expected))) => {
                let error = match parse_forms(source) {
                    Ok(_) => panic!("{id} should fail"),
                    Err(error) => error,
                };
                assert!(
                    error.contains(expected),
                    "{id}: expected <{expected}> in <{error}>"
                );
            }
            _ => panic!("{id} must contain exactly one of :readable or :error"),
        }
    }
}

fn map_value<'a>(entries: &'a [(Form, Form)], name: &str) -> Option<&'a Form> {
    entries.iter().find_map(|(key, value)| match key {
        Form::Keyword(keyword) if keyword == name => Some(value),
        _ => None,
    })
}

fn string_value<'a>(entries: &'a [(Form, Form)], name: &str) -> &'a str {
    match map_value(entries, name) {
        Some(Form::String(value)) => value,
        _ => panic!("case must contain string :{name}"),
    }
}
