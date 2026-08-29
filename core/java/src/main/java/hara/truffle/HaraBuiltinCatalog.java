package hara.truffle;

import hara.lang.declaration.HaraAvailability;
import hara.lang.declaration.HaraNativeBinding;
import java.util.HashSet;
import java.util.Map;
import java.util.Set;

/**
 * Immutable inventories used while bootstrapping the Truffle runtime.
 *
 * <p>The catalog is deliberately separate from {@link HaraContext}: these values describe the
 * language/native surface, but do not own context state or runtime behavior.
 */
@HaraNativeBinding(namespace = "std.native", name = "Maths", methods = {"abs", "acos", "acosh", "asin", "asinh", "atan", "atan2", "atanh", "ceil", "cos", "cosh", "exp", "floor", "pow", "sin", "sinh", "sqrt", "tan", "tanh"})
@HaraNativeBinding(namespace = "std.native", name = "Num", methods = {"long", "double", "parse-long", "parse-double"})
@HaraNativeBinding(namespace = "std.native", name = "Bits", methods = {"and", "or", "xor", "not", "shift-left", "shift-right"})
@HaraNativeBinding(
    namespace = "std.native", name = "Kernel", availability = HaraAvailability.CAPABILITY_GATED,
    capability = "kernel", methods = {"session-create", "session-close", "session-list", "session-info", "session-eval", "session-namespace", "session-complete", "resource-register", "resource-remove", "resource-list", "filesystem-create", "filesystem-attach", "filesystem-detach", "filesystem-info", "filesystem-close", "capabilities", "package-build", "package-inspect", "package-install", "package-publish", "package-registry-verify", "tap-config-root", "tap-add", "tap-bootstrap", "tap-remove", "tap-list", "tap-mirror-add", "tap-initialize", "tap-verify", "snapshot-build", "snapshot-verify", "snapshot-inspect", "snapshot-diff"})
@HaraNativeBinding(
    namespace = "std.native", name = "Sandbox", availability = HaraAvailability.CAPABILITY_GATED,
    capability = "sandbox", methods = {"open", "eval", "call", "cancel", "status", "close"})
@HaraNativeBinding(
    namespace = "std.native", name = "Package", availability = HaraAvailability.CAPABILITY_GATED,
    capability = "kernel", methods = {"catalog", "find", "ensure", "load", "unload", "state"})
@HaraNativeBinding(namespace = "std.native", name = "String", methods = {"length", "blank?", "includes?", "starts-with?", "ends-with?", "char-at", "slice", "index-of", "last-index-of", "join", "split", "split-lines", "repeat", "replace", "replace-first", "trim", "trim-left", "trim-right", "upper", "lower", "capitalize", "decapitalize", "pad-left", "pad-right", "reverse", "encode-utf8", "decode-utf8", "to-fixed"})
@HaraNativeBinding(namespace = "std.native", name = "Bytes", methods = {"new", "count", "get", "set", "copy", "slice", "u8", "s8"})
@HaraNativeBinding(namespace = "std.native", name = "Crypto", methods = {"sha256", "sha512", "hmac-sha256", "hmac-sha512", "random-bytes", "secure-equal?", "ed25519-keypair", "ed25519-public", "ed25519-sign", "ed25519-verify", "x25519-keypair", "x25519-public", "x25519-shared", "p256-keypair", "p256-public", "p256-sign", "p256-verify", "p256-shared"})
@HaraNativeBinding(namespace = "std.native", name = "OS", methods = {"platform", "arch", "cwd", "env", "getenv", "time-ms", "time-ns"})
@HaraNativeBinding(
    namespace = "std.native", name = "Process", availability = HaraAvailability.CAPABILITY_GATED,
    capability = "native-runtime", methods = {"spawn", "alive?", "write", "close-input", "stdout", "stderr", "stdout-stream", "stderr-stream", "wait", "kill"})
@HaraNativeBinding(
    namespace = "std.native", name = "File", availability = HaraAvailability.CAPABILITY_GATED,
    capability = "file", methods = {"parent", "join", "resolve", "read", "write", "exists?", "stat", "entries", "list", "walk", "mkdir", "delete", "copy", "move", "temp-file", "temp-directory"})
@HaraNativeBinding(
    namespace = "std.native", name = "Socket", availability = HaraAvailability.CAPABILITY_GATED,
    capability = "network", methods = {"connect", "listen", "endpoint", "events", "next", "send", "close", "receive-stream"})
@HaraNativeBinding(namespace = "std.native", name = "Promise", methods = {"run", "new", "from", "all", "delay"})
@HaraNativeBinding(namespace = "std.native", name = "Coroutine", methods = {"create", "yield", "await"})
@HaraNativeBinding(namespace = "std.native", name = "Stream", methods = {"create", "generate", "next"})
@HaraNativeBinding(namespace = "std.native", name = "Arr", methods = {"new", "get", "set", "push-first", "push-last", "pop-first", "pop-last", "insert", "remove", "clone", "slice", "map", "filter", "fold-left", "fold-right"})
@HaraNativeBinding(namespace = "std.native", name = "Obj", methods = {"new", "get", "set", "has?", "delete", "clone", "assign", "keys", "vals", "pairs"})
@HaraNativeBinding(namespace = "std.native", name = "Runtime", methods = {"load-string", "macroexpand-1", "gensym", "ns-publics", "ns-aliases", "ns-find", "ns-create", "ns-name", "var-sym", "current", "snapshot", "vars", "namespaces", "namespace", "module", "alias-state", "intern-var", "eval-in", "eval"})
@HaraNativeBinding(namespace = "std.native", name = "Printer", methods = {"p", "println", "capture"})
@HaraNativeBinding(namespace = "std.native", name = "Document", methods = {"element", "text", "fragment", "annotate", "pass", "escaped", "group", "line", "break", "nest", "align", "normalize", "valid?", "render"})
@HaraNativeBinding(namespace = "std.native", name = "Edn", methods = {"read", "read-forms", "write", "pretty"})
@HaraNativeBinding(namespace = "std.native", name = "Json", methods = {"read", "write", "pretty"})
@HaraNativeBinding(
    namespace = "std.native", name = "Host", availability = HaraAvailability.CAPABILITY_GATED,
    capability = "host-call", methods = {"call", "describe", "capabilities", "capability?"})
@HaraNativeBinding(namespace = "std.native", name = "Test", methods = {"catalog", "config", "context", "events", "compare", "run", "result", "passed?", "actual", "expected", "failures", "failure-seq", "failure-count", "failure", "failure?"})
@HaraNativeBinding(namespace = "std.native", name = "RegExp", methods = {"compile", "pattern", "find?", "find", "matches", "replace", "split"})
@HaraNativeBinding(namespace = "std.native", name = "Result", methods = {"create", "synchronize", "success?", "error?", "status", "data", "error-value", "context", "with-context"})
@HaraNativeBinding(namespace = "std.native", name = "Schema", methods = {"compile", "of", "kind", "form", "ast", "origin"})
@HaraNativeBinding(namespace = "std.native", name = "Exception", methods = {"new", "message", "class"})
@HaraNativeBinding(namespace = "std.native", name = "Base", methods = {"list", "vector", "vec", "set", "hash-map", "hash-set", "bytes", "atom", "pointer", "symbol", "keyword", "uuid", "reduced", "unreduced", "hash", "apply", "resolve", "number?", "long?", "satisfies?", "special-symbol?", "type", "instance?"})
@HaraNativeBinding(namespace = "std.native", name = "Algo", methods = {"deque", "ordered-map", "ordered-set", "priority-map", "queue", "sorted-map", "sorted-set", "trie", "deque?", "ordered-map?", "ordered-set?", "priority-map?", "queue?", "sorted-map?", "sorted-set?", "trie?"})
@HaraNativeBinding(namespace = "std.native", name = "Iter", methods = {"seq", "iter", "iter-finite?", "iter-materialize", "iter-next?", "iter-next", "iter-close", "iter-concat", "iter-map", "iter-filter", "iter-take-while", "iter-drop-while", "iter-mapcat", "iter-keep", "iter-interpose", "iter-interleave", "iter-every?", "iter-any?", "iter-take", "iter-drop", "iter-zip", "iter-cycle", "iter-partition-pair", "iter-partition-all", "iter-partition", "iter-range", "iter-constantly", "iter-repeatedly", "iter-iterate"})
@HaraNativeBinding(
    namespace = "std.native", name = "Work", availability = HaraAvailability.CAPABILITY_GATED,
    capability = "native-runtime", methods = {"default-host", "current-run", "cancelled?", "check-cancelled", "deadline-nanos", "emit", "submit-child", "on-close"})
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

  static final Set<String> MARKER_METHOD_NAMES = markerMethodNames();

  private static Set<String> markerMethodNames() {
    Set<String> methods = new HashSet<>();
    for (String type : Set.of("Arr", "Obj")) {
      HaraNativeDeclarations.methods(type).stream()
          .filter(method -> !"new".equals(method))
          .forEach(methods::add);
    }
    return Set.copyOf(methods);
  }

  private HaraBuiltinCatalog() {}
}
