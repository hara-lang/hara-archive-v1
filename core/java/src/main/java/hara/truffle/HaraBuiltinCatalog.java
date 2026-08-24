package hara.truffle;

import hara.lang.declaration.HaraAvailability;
import hara.lang.declaration.HaraNativeBinding;
import java.util.Map;
import java.util.Set;

/**
 * Immutable inventories used while bootstrapping the Truffle runtime.
 *
 * <p>The catalog is deliberately separate from {@link HaraContext}: these values describe the
 * language/native surface, but do not own context state or runtime behavior.
 */
@HaraNativeBinding(namespace = "std.native", name = "Maths")
@HaraNativeBinding(namespace = "std.native", name = "Num")
@HaraNativeBinding(namespace = "std.native", name = "Bits")
@HaraNativeBinding(
    namespace = "std.native", name = "Kernel", availability = HaraAvailability.CAPABILITY_GATED,
    capability = "native-runtime")
@HaraNativeBinding(
    namespace = "std.native", name = "Sandbox", availability = HaraAvailability.CAPABILITY_GATED,
    capability = "native-runtime")
@HaraNativeBinding(
    namespace = "std.native", name = "Package", availability = HaraAvailability.CAPABILITY_GATED,
    capability = "native-runtime")
@HaraNativeBinding(namespace = "std.native", name = "String")
@HaraNativeBinding(namespace = "std.native", name = "Bytes")
@HaraNativeBinding(namespace = "std.native", name = "Crypto")
@HaraNativeBinding(namespace = "std.native", name = "OS")
@HaraNativeBinding(
    namespace = "std.native", name = "Process", availability = HaraAvailability.CAPABILITY_GATED,
    capability = "native-runtime")
@HaraNativeBinding(
    namespace = "std.native", name = "File", availability = HaraAvailability.CAPABILITY_GATED,
    capability = "native-runtime")
@HaraNativeBinding(
    namespace = "std.native", name = "Socket", availability = HaraAvailability.CAPABILITY_GATED,
    capability = "native-runtime")
@HaraNativeBinding(namespace = "std.native", name = "Promise")
@HaraNativeBinding(namespace = "std.native", name = "Coroutine")
@HaraNativeBinding(namespace = "std.native", name = "Stream")
@HaraNativeBinding(namespace = "std.native", name = "Arr")
@HaraNativeBinding(namespace = "std.native", name = "Obj")
@HaraNativeBinding(namespace = "std.native", name = "Runtime")
@HaraNativeBinding(namespace = "std.native", name = "Printer")
@HaraNativeBinding(namespace = "std.native", name = "Document")
@HaraNativeBinding(namespace = "std.native", name = "Edn")
@HaraNativeBinding(namespace = "std.native", name = "Json")
@HaraNativeBinding(
    namespace = "std.native", name = "Host", availability = HaraAvailability.CAPABILITY_GATED,
    capability = "native-runtime")
@HaraNativeBinding(namespace = "std.native", name = "Test")
@HaraNativeBinding(namespace = "std.native", name = "RegExp")
@HaraNativeBinding(namespace = "std.native", name = "UUID")
@HaraNativeBinding(namespace = "std.native", name = "Result")
@HaraNativeBinding(namespace = "std.native", name = "Schema")
@HaraNativeBinding(namespace = "std.native", name = "Error")
@HaraNativeBinding(namespace = "std.native", name = "Base")
@HaraNativeBinding(namespace = "std.native", name = "Algo")
@HaraNativeBinding(namespace = "std.native", name = "Iter")
final class HaraBuiltinCatalog {
  /** Closed accounting inventory for forms; this is not a std.native type. */
  static final Map<String, java.util.List<String>> LANGUAGE_BUILTINS =
      Map.of(
          "evaluation",
          java.util.List.of(
              "quote", "syntax-quote", "do", "if", "let", "letfn", "binding", "loop",
              "recur", "throw", "try", "fn"),
          "definitions",
          java.util.List.of(
              "def", "declare", "var", "set!", "defmacro", "defstruct", "defmutable",
              "defprotocol", "extend-type", "defmulti", "defmethod"),
          "namespaces", java.util.List.of("ns", "ns+", "require", "alias"),
          "interop", java.util.List.of("new", "field", "."));

  static final Set<String> SPECIAL_SYMBOLS =
      Set.of(
          "quote",
          "comment",
          "do",
          "if",
          "when",
          "when-not",
          "cond",
          "and",
          "or",
          "let",
          "letfn",
          "binding",
          "loop",
          "recur",
          "throw",
          "try",
          "fn",
          "defn",
          "defn-",
          "declare",
          "defmulti",
          "defmethod",
          "def",
          "var",
          "deref",
          "set!",
          "defstruct",
          "defmutable",
          "defprotocol",
          "extend-type",
          "defmacro",
          "new",
          "ns",
          "ns+");

  static final Map<String, String> GENERATED_LIBRARIES =
      Map.ofEntries(
          Map.entry("string", "std.foundation.string"),
          Map.entry("coroutine", "std.foundation.coroutine"),
          Map.entry("promise", "std.foundation.promise"),
          Map.entry("bytes", "std.foundation.bytes"),
          Map.entry("pretty", "std.foundation.pretty"));

  static final Map<String, String> DEFAULT_LIBRARY_ALIASES =
      Map.ofEntries(
          Map.entry("string", "str"),
          Map.entry("coroutine", "co"),
          Map.entry("promise", "promise"),
          Map.entry("bytes", "bytes"),
          Map.entry("pretty", "pretty"));

  static final Set<String> MARKER_METHOD_NAMES =
      Set.of(
          "get",
          "set",
          "push-first",
          "push-last",
          "pop-first",
          "pop-last",
          "insert",
          "remove",
          "clone",
          "slice",
          "map",
          "filter",
          "fold-left",
          "fold-right",
          "has?",
          "delete",
          "assign",
          "keys",
          "vals",
          "pairs");

  static final Map<String, java.util.List<String>> NATIVE_METHODS =
      Map.ofEntries(
          Map.entry("Maths", java.util.List.of("abs", "acos", "acosh", "asin", "asinh", "atan", "atan2", "atanh", "ceil", "cos", "cosh", "exp", "floor", "pow", "sin", "sinh", "sqrt", "tan", "tanh")),
          Map.entry("Num", java.util.List.of("long", "double", "parse-long", "parse-double")),
          Map.entry("Bits", java.util.List.of("and", "or", "xor", "not", "shift-left", "shift-right")),
          Map.entry("Kernel", java.util.List.of("session-create", "session-close", "session-list", "session-info", "session-eval", "session-namespace", "session-complete", "resource-register", "resource-remove", "resource-list", "filesystem-create", "filesystem-attach", "filesystem-detach", "filesystem-info", "filesystem-close", "capabilities", "package-build", "package-inspect", "package-install", "package-publish", "package-registry-verify", "tap-config-root", "tap-add", "tap-bootstrap", "tap-remove", "tap-list", "tap-mirror-add", "tap-initialize", "tap-verify", "snapshot-build", "snapshot-verify", "snapshot-inspect", "snapshot-diff")),
          Map.entry("Sandbox", java.util.List.of("open", "eval", "call", "cancel", "status", "close")),
          Map.entry("Package", java.util.List.of("catalog", "find", "ensure", "load", "unload", "state")),
          Map.entry("String", java.util.List.of("length", "blank?", "includes?", "starts-with?", "ends-with?", "char-at", "slice", "index-of", "last-index-of", "join", "split", "split-lines", "repeat", "replace", "replace-first", "trim", "trim-left", "trim-right", "upper", "lower", "capitalize", "decapitalize", "pad-left", "pad-right", "reverse", "encode-utf8", "decode-utf8", "to-fixed")),
          Map.entry("Bytes", java.util.List.of("new", "instance?", "count", "get", "set", "copy", "slice", "u8", "s8")),
          Map.entry(
              "Crypto",
              java.util.List.of(
                  "sha256", "sha512", "hmac-sha256", "hmac-sha512", "random-bytes",
                  "secure-equal?", "ed25519-keypair", "ed25519-public", "ed25519-sign",
                  "ed25519-verify", "x25519-keypair", "x25519-public", "x25519-shared",
                  "p256-keypair", "p256-public", "p256-sign", "p256-verify", "p256-shared")),
          Map.entry("OS", java.util.List.of("platform", "arch", "cwd", "env", "getenv", "time-ms", "time-ns")),
          Map.entry("Process", java.util.List.of("spawn", "instance?", "alive?", "write", "close-input", "stdout", "stderr", "stdout-stream", "stderr-stream", "wait", "kill")),
          Map.entry(
              "File",
              java.util.List.of(
                  "parent", "join", "resolve", "read", "write", "exists?", "stat",
                  "entries", "list", "walk", "mkdir", "delete", "copy", "move",
                  "temp-file", "temp-directory")),
          Map.entry("Socket", java.util.List.of("connect", "listen", "endpoint", "events", "next", "send", "close", "receive-stream")),
          Map.entry("Promise", java.util.List.of("run", "new", "from", "all", "delay", "instance?")),
          Map.entry("Coroutine", java.util.List.of("create", "yield", "await", "instance?")),
          Map.entry("Stream", java.util.List.of("create", "generate", "next", "instance?")),
          Map.entry("Arr", java.util.List.of("new", "instance?", "get", "set", "push-first", "push-last", "pop-first", "pop-last", "insert", "remove", "clone", "slice", "map", "filter", "fold-left", "fold-right")),
          Map.entry("Obj", java.util.List.of("new", "instance?", "get", "set", "has?", "delete", "clone", "assign", "keys", "vals", "pairs")),
          Map.entry(
              "Runtime",
              java.util.List.of(
                  "load-string", "macroexpand-1", "gensym", "var-sym", "current", "snapshot",
                  "vars", "namespaces", "namespace", "module", "resolve", "alias-state",
                  "intern-var", "eval-in", "eval")),
          Map.entry("Printer", java.util.List.of("p", "println", "capture")),
          Map.entry("Document", java.util.List.of("element", "text", "fragment", "annotate", "pass", "escaped", "group", "line", "break", "nest", "align", "normalize", "valid?", "render")),
          Map.entry("Edn", java.util.List.of("read", "read-forms", "write", "pretty")),
          Map.entry("Json", java.util.List.of("read", "write", "pretty")),
          Map.entry("Host", java.util.List.of("call", "describe", "capabilities", "capability?")),
          Map.entry(
              "Test",
              java.util.List.of(
                  "catalog", "config", "context", "events", "compare", "run", "result",
                  "passed?", "actual", "expected", "failures", "failure-seq", "failure-count",
                  "failure", "failure?")),
          Map.entry(
              "RegExp",
              java.util.List.of(
                  "instance?", "compile", "pattern", "find?", "find", "matches", "replace",
                  "split")),
          Map.entry("UUID", java.util.List.of("instance?")),
          Map.entry(
              "Result",
              java.util.List.of(
                  "create", "synchronize", "instance?", "success?", "error?", "status",
                  "data", "error-value", "context", "with-context")),
          Map.entry(
              "Schema",
              java.util.List.of("compile", "of", "instance?", "kind", "form", "ast", "origin")),
          Map.entry("Error", java.util.List.of("new", "message", "class")),
          Map.entry(
              "Base",
              java.util.List.of(
                  "list", "vector", "vec", "set", "tuple", "hash-map", "hash-set", "atom",
                  "pointer", "symbol", "keyword", "reduced", "unreduced", "apply", "not", "boolean", "compare",
                  "reduced?", "nil?", "boolean?", "string?", "char?", "number?", "integer?",
                  "long?", "double?", "keyword?", "symbol?", "pointer?",
                  "atom?", "function?", "bytes?", "array?", "object?", "list?", "cons?", "vector?",
                  "tuple?", "map?", "set?", "sequential?", "coll?", "satisfies?", "type", "instance?")),
          Map.entry(
              "Algo",
              java.util.List.of(
                  "deque", "ordered-map", "ordered-set", "priority-map", "queue",
                  "sorted-map", "sorted-set", "trie", "deque?", "ordered-map?",
                  "ordered-set?", "priority-map?", "queue?", "sorted-map?",
                  "sorted-set?", "trie?")),
          Map.entry(
              "Iter",
              java.util.List.of(
                  "iter", "iter?", "iter-finite?", "iter-materialize",
                  "iter-next?", "iter-next", "iter-close", "iter-concat",
                  "iter-map", "iter-filter", "iter-take-while", "iter-drop-while",
                  "iter-mapcat", "iter-keep", "iter-interpose", "iter-interleave",
                  "iter-every?", "iter-any?", "iter-take", "iter-drop", "iter-zip",
                  "iter-cycle", "iter-partition-pair", "iter-partition-all",
                  "iter-partition", "iter-range", "iter-constantly",
                  "iter-repeatedly", "iter-iterate")));

  private HaraBuiltinCatalog() {}
}
