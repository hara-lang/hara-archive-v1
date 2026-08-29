use hara_protocol_macros::hara_native_registry;

#[hara_native_registry]
pub(crate) mod declarations {
    #[hara_native(
        namespace = "std.native",
        name = "Maths",
        methods = [
            "abs", "acos", "acosh", "asin", "asinh", "atan", "atan2", "atanh", "ceil",
            "cos", "cosh", "exp", "floor", "pow", "sin", "sinh", "sqrt", "tan", "tanh"
        ],
        provider = native_maths_provider
    )]
    struct Maths;

    #[hara_native(namespace = "std.native", name = "Num", methods = ["long", "double", "parse-long", "parse-double"], provider = native_num_provider)]
    struct Num;

    #[hara_native(namespace = "std.native", name = "Bits", methods = ["and", "or", "xor", "not", "shift-left", "shift-right"], provider = native_bits_provider)]
    struct Bits;

    #[hara_native(
        namespace = "std.native",
        name = "Kernel",
        availability = "capability-gated",
        capability = "kernel",
        methods = [
            "session-create", "session-close", "session-list", "session-info", "session-eval",
            "session-namespace", "session-complete", "resource-register", "resource-remove",
            "resource-list", "filesystem-create", "filesystem-attach", "filesystem-detach",
            "filesystem-info", "filesystem-close", "capabilities", "package-build", "package-inspect",
            "package-install", "package-publish", "package-registry-verify", "tap-config-root", "tap-add",
            "tap-bootstrap", "tap-remove", "tap-list", "tap-mirror-add", "tap-initialize", "tap-verify",
            "snapshot-build", "snapshot-verify", "snapshot-inspect", "snapshot-diff"
        ],
        provider = native_kernel_provider
    )]
    struct Kernel;

    #[hara_native(
        namespace = "std.native",
        name = "Sandbox",
        availability = "capability-gated",
        capability = "sandbox",
        methods = ["open", "eval", "call", "cancel", "status", "close"],
        provider = native_sandbox_provider
    )]
    struct Sandbox;

    #[hara_native(
        namespace = "std.native",
        name = "Package",
        availability = "capability-gated",
        capability = "kernel",
        methods = ["catalog", "find", "ensure", "load", "unload", "state"],
        provider = native_package_provider
    )]
    struct Package;

    #[hara_native(
        namespace = "std.native",
        name = "String",
        methods = [
            "length", "blank?", "includes?", "starts-with?", "ends-with?", "char-at", "slice", "index-of",
            "last-index-of", "join", "split", "split-lines", "repeat", "replace", "replace-first", "trim",
            "trim-left", "trim-right", "upper", "lower", "capitalize", "decapitalize", "pad-left",
            "pad-right", "reverse", "encode-utf8", "decode-utf8", "to-fixed"
        ],
        provider = native_string_provider
    )]
    struct String;

    #[hara_native(namespace = "std.native", name = "Bytes", methods = ["new", "count", "get", "set", "copy", "slice", "u8", "s8"], provider = native_bytes_provider)]
    struct Bytes;

    #[hara_native(
        namespace = "std.native",
        name = "Crypto",
        methods = [
            "sha256", "sha512", "hmac-sha256", "hmac-sha512", "random-bytes", "secure-equal?",
            "ed25519-keypair", "ed25519-public", "ed25519-sign", "ed25519-verify", "x25519-keypair",
            "x25519-public", "x25519-shared", "p256-keypair", "p256-public", "p256-sign", "p256-verify",
            "p256-shared"
        ],
        provider = native_crypto_provider
    )]
    struct Crypto;

    #[hara_native(namespace = "std.native", name = "OS", methods = ["platform", "arch", "cwd", "env", "getenv", "time-ms", "time-ns"], provider = native_os_provider)]
    struct OS;

    #[hara_native(
        namespace = "std.native",
        name = "Process",
        availability = "capability-gated",
        capability = "native-runtime",
        methods = ["spawn", "alive?", "write", "close-input", "stdout", "stderr", "stdout-stream", "stderr-stream", "wait", "kill"],
        provider = native_os_provider
    )]
    struct Process;

    #[hara_native(
        namespace = "std.native",
        name = "File",
        availability = "capability-gated",
        capability = "file",
        methods = ["parent", "join", "resolve", "read", "write", "exists?", "stat", "entries", "list", "walk", "mkdir", "delete", "copy", "move", "temp-file", "temp-directory"],
        provider = native_file_provider
    )]
    struct File;

    #[hara_native(
        namespace = "std.native",
        name = "Socket",
        availability = "capability-gated",
        capability = "network",
        methods = ["connect", "listen", "endpoint", "events", "next", "send", "close", "receive-stream"],
        provider = native_socket_provider
    )]
    struct Socket;

    #[hara_native(namespace = "std.native", name = "Promise", methods = ["run", "new", "from", "all", "delay"], provider = native_promise_provider)]
    struct Promise;

    #[hara_native(namespace = "std.native", name = "Coroutine", methods = ["create", "yield", "await"], provider = native_coroutine_provider)]
    struct Coroutine;

    #[hara_native(namespace = "std.native", name = "Stream", methods = ["create", "generate", "next"], provider = native_stream_provider)]
    struct Stream;

    #[hara_native(namespace = "std.native", name = "Arr", methods = ["new", "get", "set", "push-first", "push-last", "pop-first", "pop-last", "insert", "remove", "clone", "slice", "map", "filter", "fold-left", "fold-right"], provider = native_mutable_provider)]
    struct Arr;

    #[hara_native(namespace = "std.native", name = "Obj", methods = ["new", "get", "set", "has?", "delete", "clone", "assign", "keys", "vals", "pairs"], provider = native_mutable_provider)]
    struct Obj;

    #[hara_native(
        namespace = "std.native",
        name = "Runtime",
        methods = [
            "load-string", "macroexpand-1", "gensym", "ns-publics", "ns-aliases", "ns-find", "ns-create", "ns-name", "var-sym",
            "current", "snapshot", "vars", "namespaces", "namespace", "module", "alias-state",
            "intern-var", "eval-in", "eval"
        ],
        provider = native_runtime_provider
    )]
    struct Runtime;

    #[hara_native(namespace = "std.native", name = "Printer", methods = ["p", "println", "capture"], provider = native_printer_provider)]
    struct Printer;

    #[hara_native(namespace = "std.native", name = "Document", methods = ["element", "text", "fragment", "annotate", "pass", "escaped", "group", "line", "break", "nest", "align", "normalize", "valid?", "render"], provider = native_document_provider)]
    struct Document;

    #[hara_native(namespace = "std.native", name = "Edn", methods = ["read", "read-forms", "write", "pretty"], provider = native_edn_provider)]
    struct Edn;

    #[hara_native(namespace = "std.native", name = "Json", methods = ["read", "write", "pretty"], provider = native_json_provider)]
    struct Json;

    #[hara_native(
        namespace = "std.native",
        name = "Host",
        availability = "capability-gated",
        capability = "host-call",
        methods = ["call", "describe", "capabilities", "capability?"],
        provider = native_host_provider
    )]
    struct Host;

    #[hara_native(
        namespace = "std.native",
        name = "Test",
        methods = [
            "catalog", "config", "context", "events", "compare", "run", "result", "passed?", "actual",
            "expected", "failures", "failure-seq", "failure-count", "failure", "failure?"
        ],
        provider = native_test_provider
    )]
    struct Test;

    #[hara_native(namespace = "std.native", name = "RegExp", methods = ["compile", "pattern", "find?", "find", "matches", "replace", "split"], provider = native_regexp_provider)]
    struct RegExp;

    #[hara_native(namespace = "std.native", name = "Result", methods = ["create", "synchronize", "success?", "error?", "status", "data", "error-value", "context", "with-context"], provider = native_result_provider)]
    struct Result;

    #[hara_native(namespace = "std.native", name = "Schema", methods = ["compile", "of", "kind", "form", "ast", "origin"], provider = native_schema_provider)]
    struct Schema;

    #[hara_native(namespace = "std.native", name = "Exception", methods = ["new", "message", "class"], provider = native_exception_provider)]
    struct Exception;

    #[hara_native(
        namespace = "std.native",
        name = "Base",
        methods = [
            "list", "vector", "vec", "set", "hash-map", "hash-set", "atom", "bytes", "pointer", "symbol",
            "keyword", "uuid", "reduced", "unreduced", "hash", "apply", "resolve",
            "number?", "long?", "satisfies?", "special-symbol?", "type", "instance?"
        ],
        whole_wasm_methods = [("number?", 1)],
        provider = native_base_provider
    )]
    struct Base;

    #[hara_native(namespace = "std.native", name = "Algo", methods = ["deque", "ordered-map", "ordered-set", "priority-map", "queue", "sorted-map", "sorted-set", "trie", "deque?", "ordered-map?", "ordered-set?", "priority-map?", "queue?", "sorted-map?", "sorted-set?", "trie?"], provider = native_algo_provider)]
    struct Algo;

    #[hara_native(
        namespace = "std.native",
        name = "Iter",
        methods = [
            "seq", "iter", "iter-finite?", "iter-materialize", "iter-next?", "iter-next", "iter-close",
            "iter-concat", "iter-map", "iter-filter", "iter-take-while", "iter-drop-while", "iter-mapcat",
            "iter-keep", "iter-interpose", "iter-interleave", "iter-every?", "iter-any?", "iter-take",
            "iter-drop", "iter-zip", "iter-cycle", "iter-partition-pair", "iter-partition-all", "iter-partition",
            "iter-range", "iter-constantly", "iter-repeatedly", "iter-iterate"
        ],
        provider = native_iter_provider
    )]
    struct Iter;

    #[hara_native(
        namespace = "std.native",
        name = "Work",
        availability = "capability-gated",
        capability = "native-runtime",
        methods = ["default-host", "current-run", "cancelled?", "check-cancelled", "deadline-nanos", "emit", "submit-child", "on-close"],
        provider = native_work_provider
    )]
    struct Work;
}
