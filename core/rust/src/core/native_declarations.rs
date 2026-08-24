use hara_protocol_macros::hara_native_registry;

#[hara_native_registry]
pub(crate) mod declarations {
    #[hara_native(
        namespace = "std.native",
        name = "Maths",
        methods = [
            "abs", "acos", "acosh", "asin", "asinh", "atan", "atan2", "atanh", "ceil",
            "cos", "cosh", "exp", "floor", "pow", "sin", "sinh", "sqrt", "tan", "tanh"
        ]
    )]
    struct Maths;

    #[hara_native(namespace = "std.native", name = "Num", methods = ["long", "double", "parse-long", "parse-double"])]
    struct Num;

    #[hara_native(namespace = "std.native", name = "Bits", methods = ["and", "or", "xor", "not", "shift-left", "shift-right"])]
    struct Bits;

    #[hara_native(
        namespace = "std.native",
        name = "Kernel",
        availability = "capability-gated",
        capability = "native-runtime",
        methods = [
            "session-create", "session-close", "session-list", "session-info", "session-eval",
            "session-namespace", "session-complete", "resource-register", "resource-remove",
            "resource-list", "filesystem-create", "filesystem-attach", "filesystem-detach",
            "filesystem-info", "filesystem-close", "capabilities", "package-build", "package-inspect",
            "package-install", "package-publish", "package-registry-verify", "tap-config-root", "tap-add",
            "tap-bootstrap", "tap-remove", "tap-list", "tap-mirror-add", "tap-initialize", "tap-verify",
            "snapshot-build", "snapshot-verify", "snapshot-inspect", "snapshot-diff"
        ]
    )]
    struct Kernel;

    #[hara_native(
        namespace = "std.native",
        name = "Sandbox",
        availability = "capability-gated",
        capability = "native-runtime",
        methods = ["open", "eval", "call", "cancel", "status", "close"]
    )]
    struct Sandbox;

    #[hara_native(
        namespace = "std.native",
        name = "Package",
        availability = "capability-gated",
        capability = "native-runtime",
        methods = ["catalog", "find", "ensure", "load", "unload", "state"]
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
        ]
    )]
    struct String;

    #[hara_native(namespace = "std.native", name = "Bytes", methods = ["new", "instance?", "count", "get", "set", "copy", "slice", "u8", "s8"])]
    struct Bytes;

    #[hara_native(
        namespace = "std.native",
        name = "Crypto",
        methods = [
            "sha256", "sha512", "hmac-sha256", "hmac-sha512", "random-bytes", "secure-equal?",
            "ed25519-keypair", "ed25519-public", "ed25519-sign", "ed25519-verify", "x25519-keypair",
            "x25519-public", "x25519-shared", "p256-keypair", "p256-public", "p256-sign", "p256-verify",
            "p256-shared"
        ]
    )]
    struct Crypto;

    #[hara_native(namespace = "std.native", name = "OS", methods = ["platform", "arch", "cwd", "env", "getenv", "time-ms", "time-ns"])]
    struct OS;

    #[hara_native(
        namespace = "std.native",
        name = "Process",
        availability = "capability-gated",
        capability = "native-runtime",
        methods = ["spawn", "instance?", "alive?", "write", "close-input", "stdout", "stderr", "stdout-stream", "stderr-stream", "wait", "kill"]
    )]
    struct Process;

    #[hara_native(
        namespace = "std.native",
        name = "File",
        availability = "capability-gated",
        capability = "native-runtime",
        methods = ["parent", "join", "resolve", "read", "write", "exists?", "stat", "entries", "list", "walk", "mkdir", "delete", "copy", "move", "temp-file", "temp-directory"]
    )]
    struct File;

    #[hara_native(
        namespace = "std.native",
        name = "Socket",
        availability = "capability-gated",
        capability = "native-runtime",
        methods = ["connect", "listen", "endpoint", "events", "next", "send", "close", "receive-stream"]
    )]
    struct Socket;

    #[hara_native(namespace = "std.native", name = "Promise", methods = ["run", "new", "from", "all", "delay", "instance?"])]
    struct Promise;

    #[hara_native(namespace = "std.native", name = "Coroutine", methods = ["create", "yield", "await", "instance?"])]
    struct Coroutine;

    #[hara_native(namespace = "std.native", name = "Stream", methods = ["create", "generate", "next", "instance?"])]
    struct Stream;

    #[hara_native(namespace = "std.native", name = "Arr", methods = ["new", "instance?", "get", "set", "push-first", "push-last", "pop-first", "pop-last", "insert", "remove", "clone", "slice", "map", "filter", "fold-left", "fold-right"])]
    struct Arr;

    #[hara_native(namespace = "std.native", name = "Obj", methods = ["new", "instance?", "get", "set", "has?", "delete", "clone", "assign", "keys", "vals", "pairs"])]
    struct Obj;

    #[hara_native(
        namespace = "std.native",
        name = "Runtime",
        methods = [
            "load-string", "macroexpand-1", "gensym", "ns-publics", "the-ns", "ns-name", "var-sym",
            "current", "snapshot", "vars", "namespaces", "namespace", "module", "resolve", "alias-state",
            "intern-var", "eval-in", "eval"
        ]
    )]
    struct Runtime;

    #[hara_native(namespace = "std.native", name = "Printer", methods = ["p", "println", "capture"])]
    struct Printer;

    #[hara_native(namespace = "std.native", name = "Document", methods = ["element", "text", "fragment", "annotate", "pass", "escaped", "group", "line", "break", "nest", "align", "normalize", "valid?", "render"])]
    struct Document;

    #[hara_native(namespace = "std.native", name = "Edn", methods = ["read", "read-forms", "write", "pretty"])]
    struct Edn;

    #[hara_native(namespace = "std.native", name = "Json", methods = ["read", "write", "pretty"])]
    struct Json;

    #[hara_native(
        namespace = "std.native",
        name = "Host",
        availability = "capability-gated",
        capability = "native-runtime",
        methods = ["call", "describe", "capabilities", "capability?"]
    )]
    struct Host;

    #[hara_native(
        namespace = "std.native",
        name = "Test",
        methods = [
            "catalog", "config", "context", "events", "compare", "run", "result", "passed?", "actual",
            "expected", "failures", "failure-seq", "failure-count", "failure", "failure?"
        ]
    )]
    struct Test;

    #[hara_native(namespace = "std.native", name = "RegExp", methods = ["instance?", "compile", "pattern", "find?", "find", "matches", "replace", "split"])]
    struct RegExp;

    #[hara_native(namespace = "std.native", name = "UUID", methods = ["instance?"])]
    struct UUID;

    #[hara_native(namespace = "std.native", name = "Result", methods = ["create", "synchronize", "instance?", "success?", "error?", "status", "data", "error-value", "context", "with-context"])]
    struct Result;

    #[hara_native(namespace = "std.native", name = "Schema", methods = ["compile", "of", "instance?", "kind", "form", "ast", "origin"])]
    struct Schema;

    #[hara_native(namespace = "std.native", name = "Error", methods = ["new", "message", "class"])]
    struct Error;

    #[hara_native(
        namespace = "std.native",
        name = "Base",
        methods = [
            "list", "vector", "vec", "set", "tuple", "hash-map", "hash-set", "atom", "pointer", "symbol",
            "keyword", "reduced", "unreduced", "apply", "not", "compare",
            "number?", "long?", "satisfies?", "special-symbol?", "type", "instance?"
        ]
    )]
    struct Base;

    #[hara_native(namespace = "std.native", name = "Algo", methods = ["deque", "ordered-map", "ordered-set", "priority-map", "queue", "sorted-map", "sorted-set", "trie", "deque?", "ordered-map?", "ordered-set?", "priority-map?", "queue?", "sorted-map?", "sorted-set?", "trie?"])]
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
        ]
    )]
    struct Iter;

    #[hara_native(
        namespace = "std.native",
        name = "Work",
        availability = "capability-gated",
        capability = "native-runtime",
        methods = ["default-host", "current-run", "cancelled?", "check-cancelled", "deadline-nanos", "emit", "submit-child", "on-close"]
    )]
    struct Work;
}
