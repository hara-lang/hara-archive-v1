package hara.truffle;

import hara.kernel.base.Parser;
import hara.lang.data.Keyword;
import hara.lang.data.Symbol;
import hara.lang.protocol.ILinearType;
import hara.lang.protocol.IMapType;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Iterator;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;

/** Strict JVM view of the canonical {@code memory.v1} binding plan. */
final class HaraWasmMemoryBinding {
  static final String SCHEMA = "hara.wasm-memory-binding/0-alpha";

  private static final Set<String> ROOT_FIELDS =
      Set.of("schema", "namespace", "module", "target", "memory", "functions");
  private static final Set<String> MEMORY_FIELDS =
      Set.of("export", "allocate", "reallocate", "release");
  private static final Set<String> FUNCTION_FIELDS =
      Set.of(
          "hara/name",
          "wasm/export",
          "arguments",
          "returns",
          "wasm/arguments",
          "wasm/returns");
  private static final Set<String> ARGUMENT_FIELDS =
      Set.of("name", "hara/type", "wasm/types", "lower", "ownership");
  private static final Set<String> RESULT_FIELDS =
      Set.of("hara/type", "wasm/type", "lift", "ownership");

  private final String namespace;
  private final String module;
  private final Memory memory;
  private final Map<String, Function> functions;

  private HaraWasmMemoryBinding(
      String namespace, String module, Memory memory, Map<String, Function> functions) {
    this.namespace = namespace;
    this.module = module;
    this.memory = memory;
    this.functions = Collections.unmodifiableMap(new LinkedHashMap<>(functions));
  }

  static HaraWasmMemoryBinding parse(String source, String origin) {
    Object value;
    try {
      value = Parser.LispReader.readString(source, null);
    } catch (RuntimeException error) {
      throw invalid(origin, "cannot parse bindings.edn", error);
    }
    IMapType<?, ?> root = requireMap(value, origin, "binding plan");
    rejectUnknownKeys(root, ROOT_FIELDS, origin, "binding plan");
    requireString(root, "schema", origin, SCHEMA);
    String namespace = requireSymbol(root, "namespace", origin);
    String module = requireString(root, "module", origin);
    requireKeyword(root, "target", origin, "memory.v1");
    Memory memory = parseMemory(lookup(root, "memory"), origin);
    Map<String, Function> functions = parseFunctions(lookup(root, "functions"), origin);

    boolean usesMemory = false;
    boolean requiresAllocate = false;
    boolean requiresRelease = false;
    for (Function function : functions.values()) {
      for (Argument argument : function.arguments()) {
        usesMemory |= argument.pointerLength();
        requiresAllocate |= argument.pointerLength();
        requiresRelease |= argument.ownership() == Ownership.TRANSFERRED;
      }
      usesMemory |= function.result().packedI64();
      requiresRelease |= function.result().ownership() == Ownership.CALLER;
    }
    if (!usesMemory) throw invalid(origin, "memory.v1 must lower or lift at least one value");
    if (requiresAllocate && memory.allocate() == null) {
      throw invalid(origin, "pointer/length inputs require :memory :allocate");
    }
    if (requiresRelease && memory.release() == null) {
      throw invalid(origin, "transferred inputs and caller-owned results require :memory :release");
    }
    return new HaraWasmMemoryBinding(namespace, module, memory, functions);
  }

  void verifyManifest(HaraExtensionManifest manifest) {
    if (!"wasm".equals(manifest.provider()) || !"memory.v1".equals(manifest.abi())) {
      throw mismatch("manifest must declare a Wasm :memory.v1 provider");
    }
    if (!manifest.namespace().equals(namespace) || !module.equals(manifest.module())) {
      throw mismatch("manifest namespace or module differs from bindings.edn");
    }
    if (!manifest.capabilities().isEmpty()) {
      throw mismatch("memory.v1 cannot require host capabilities");
    }
    if (manifest.exports().size() != functions.size()) {
      throw mismatch("manifest exports do not match bindings.edn");
    }
    for (Function function : functions.values()) {
      HaraExtensionManifest.Export exported = manifest.exports().get(function.name());
      if (exported == null
          || !function.wasmExport().equals(exported.wasmExport())
          || exported.async()
          || !function.publicArgumentTypes().equals(exported.arguments())
          || !function.result().type().keyword().equals(exported.returns())) {
        throw mismatch("manifest export " + function.name() + " differs from bindings.edn");
      }
    }
    if (!manifest.assets().contains("bindings.edn")) {
      throw mismatch("memory.v1 packages must declare bindings.edn as an asset");
    }
  }

  String namespace() {
    return namespace;
  }

  String module() {
    return module;
  }

  Memory memory() {
    return memory;
  }

  Function function(String name) {
    return functions.get(name);
  }

  Map<String, Function> functions() {
    return functions;
  }

  private static Memory parseMemory(Object value, String origin) {
    IMapType<?, ?> memory = requireMap(value, origin, "memory");
    rejectUnknownKeys(memory, MEMORY_FIELDS, origin, "memory");
    String export = requireString(memory, "export", origin);
    String allocate = optionalString(memory, "allocate", origin);
    String reallocate = optionalString(memory, "reallocate", origin);
    String release = optionalString(memory, "release", origin);
    if (reallocate != null) {
      throw invalid(origin, ":memory :reallocate is reserved for a later ABI revision");
    }
    return new Memory(export, allocate, release);
  }

  private static Map<String, Function> parseFunctions(Object value, String origin) {
    ILinearType<?> vector = requireVector(value, origin, "functions");
    LinkedHashMap<String, Function> functions = new LinkedHashMap<>();
    for (Object item : vector) {
      IMapType<?, ?> function = requireMap(item, origin, "function");
      rejectUnknownKeys(function, FUNCTION_FIELDS, origin, "function");
      String name = requireSymbol(function, "hara/name", origin);
      String wasmExport = requireString(function, "wasm/export", origin);
      List<Argument> arguments = parseArguments(lookup(function, "arguments"), origin, name);
      Result result = parseResult(lookup(function, "returns"), origin, name);
      List<RawType> rawArguments =
          parseRawTypes(lookup(function, "wasm/arguments"), origin, name + " raw arguments");
      RawType rawResult =
          RawType.parse(requireKeyword(function, "wasm/returns", origin), origin, name);
      ArrayList<RawType> compiledArguments = new ArrayList<>();
      for (Argument argument : arguments) compiledArguments.addAll(argument.rawTypes());
      if (!compiledArguments.equals(rawArguments) || result.rawType() != rawResult) {
        throw invalid(origin, name + " raw signature does not match its lowering plan");
      }
      if (functions.put(name, new Function(name, wasmExport, arguments, result)) != null) {
        throw invalid(origin, "duplicate function " + name);
      }
    }
    if (functions.isEmpty()) throw invalid(origin, "functions cannot be empty");
    return functions;
  }

  private static List<Argument> parseArguments(Object value, String origin, String function) {
    ILinearType<?> vector = requireVector(value, origin, function + " arguments");
    ArrayList<Argument> arguments = new ArrayList<>();
    LinkedHashSet<String> names = new LinkedHashSet<>();
    for (Object item : vector) {
      IMapType<?, ?> argument = requireMap(item, origin, function + " argument");
      rejectUnknownKeys(argument, ARGUMENT_FIELDS, origin, function + " argument");
      String name = requireSymbol(argument, "name", origin);
      if (!names.add(name)) throw invalid(origin, function + " has duplicate argument " + name);
      Type type = Type.parse(requireKeyword(argument, "hara/type", origin), origin, function);
      List<RawType> rawTypes =
          parseRawTypes(lookup(argument, "wasm/types"), origin, function + "/" + name);
      Object lowering = lookup(argument, "lower");
      Ownership ownership = optionalOwnership(argument, origin, function + "/" + name);

      if (type.directRaw() != null) {
        if (lowering != null
            || ownership != null
            || !rawTypes.equals(List.of(type.directRaw()))) {
          throw invalid(origin, function + "/" + name + " must use its direct scalar mapping");
        }
        arguments.add(new Argument(name, type, rawTypes, false, null));
        continue;
      }

      if (!type.memoryValue()
          || !pointerLength(lowering)
          || !rawTypes.equals(List.of(RawType.I32, RawType.I32))
          || (ownership != Ownership.BORROWED && ownership != Ownership.TRANSFERRED)) {
        throw invalid(
            origin,
            function
                + "/"
                + name
                + " must be :string or :bytes lowered as [:pointer :length] with input ownership");
      }
      arguments.add(new Argument(name, type, rawTypes, true, ownership));
    }
    return Collections.unmodifiableList(arguments);
  }

  private static Result parseResult(Object value, String origin, String function) {
    IMapType<?, ?> result = requireMap(value, origin, function + " result");
    rejectUnknownKeys(result, RESULT_FIELDS, origin, function + " result");
    Type type = Type.parse(requireKeyword(result, "hara/type", origin), origin, function);
    RawType rawType =
        RawType.parse(requireKeyword(result, "wasm/type", origin), origin, function);
    Object lifting = lookup(result, "lift");
    Ownership ownership = optionalOwnership(result, origin, function + " result");

    if (type.directRaw() != null) {
      if (lifting != null || ownership != null || rawType != type.directRaw()) {
        throw invalid(origin, function + " result must use its direct scalar mapping");
      }
      return new Result(type, rawType, false, null);
    }

    if (!type.memoryValue()
        || !keywordEquals(lifting, "packed-i64")
        || rawType != RawType.I64
        || (ownership != Ownership.CALLER && ownership != Ownership.CALLEE)) {
      throw invalid(
          origin,
          function
              + " result must be :string or :bytes lifted from :packed-i64 with result ownership");
    }
    return new Result(type, rawType, true, ownership);
  }

  private static List<RawType> parseRawTypes(Object value, String origin, String field) {
    ILinearType<?> vector = requireVector(value, origin, field);
    ArrayList<RawType> result = new ArrayList<>();
    for (Object item : vector) {
      if (!(item instanceof Keyword)) throw invalid(origin, field + " must contain keywords");
      result.add(RawType.parse(((Keyword) item).getName(), origin, field));
    }
    return Collections.unmodifiableList(result);
  }

  private static boolean pointerLength(Object value) {
    if (!(value instanceof ILinearType<?>)) return false;
    ArrayList<String> names = new ArrayList<>();
    for (Object item : (ILinearType<?>) value) {
      if (!(item instanceof Keyword)) return false;
      names.add(((Keyword) item).getName());
    }
    return names.equals(List.of("pointer", "length"));
  }

  private static Ownership optionalOwnership(
      IMapType<?, ?> map, String origin, String subject) {
    Object value = lookup(map, "ownership");
    if (value == null) return null;
    if (!(value instanceof Keyword)) throw invalid(origin, subject + " ownership must be a keyword");
    return Ownership.parse(((Keyword) value).getName(), origin, subject);
  }

  private static IMapType<?, ?> requireMap(Object value, String origin, String subject) {
    if (!(value instanceof IMapType<?, ?>)) throw invalid(origin, subject + " must be a map");
    return (IMapType<?, ?>) value;
  }

  private static ILinearType<?> requireVector(Object value, String origin, String subject) {
    if (!(value instanceof ILinearType<?>)) throw invalid(origin, subject + " must be a vector");
    return (ILinearType<?>) value;
  }

  private static String requireString(IMapType<?, ?> map, String field, String origin) {
    Object value = lookup(map, field);
    if (!(value instanceof String) || ((String) value).isBlank()) {
      throw invalid(origin, ":" + field + " must be a non-empty string");
    }
    return (String) value;
  }

  private static void requireString(
      IMapType<?, ?> map, String field, String origin, String expected) {
    String value = requireString(map, field, origin);
    if (!expected.equals(value)) {
      throw invalid(origin, ":" + field + " must be \"" + expected + "\"");
    }
  }

  private static String optionalString(IMapType<?, ?> map, String field, String origin) {
    Object value = lookup(map, field);
    if (value == null) return null;
    if (!(value instanceof String) || ((String) value).isBlank()) {
      throw invalid(origin, ":" + field + " must be a non-empty string");
    }
    return (String) value;
  }

  private static String requireSymbol(IMapType<?, ?> map, String field, String origin) {
    Object value = lookup(map, field);
    if (!(value instanceof Symbol)) throw invalid(origin, ":" + field + " must be a symbol");
    Symbol symbol = (Symbol) value;
    return symbol.getNamespace() == null
        ? symbol.getName()
        : symbol.getNamespace() + "/" + symbol.getName();
  }

  private static String requireKeyword(IMapType<?, ?> map, String field, String origin) {
    Object value = lookup(map, field);
    if (!(value instanceof Keyword)) throw invalid(origin, ":" + field + " must be a keyword");
    return keywordName((Keyword) value);
  }

  private static void requireKeyword(
      IMapType<?, ?> map, String field, String origin, String expected) {
    String value = requireKeyword(map, field, origin);
    if (!expected.equals(value)) {
      throw invalid(origin, ":" + field + " must be :" + expected);
    }
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  private static Object lookup(IMapType<?, ?> map, String field) {
    return ((IMapType) map).lookup(Keyword.create(field));
  }

  private static void rejectUnknownKeys(
      IMapType<?, ?> map, Set<String> allowed, String origin, String subject) {
    Iterator<?> iterator = map.iterator();
    while (iterator.hasNext()) {
      Object key = ((Map.Entry<?, ?>) iterator.next()).getKey();
      if (!(key instanceof Keyword)) throw invalid(origin, subject + " keys must be keywords");
      String name = keywordName((Keyword) key);
      if (!allowed.contains(name)) {
        throw invalid(origin, "unsupported " + subject + " field: " + name);
      }
    }
  }

  private static String keywordName(Keyword keyword) {
    return keyword.getNamespace() == null
        ? keyword.getName()
        : keyword.getNamespace() + "/" + keyword.getName();
  }

  private static boolean keywordEquals(Object value, String expected) {
    return value instanceof Keyword && expected.equals(keywordName((Keyword) value));
  }

  private static HaraException invalid(String origin, String message) {
    return new HaraException("extension/binding-invalid: " + origin + " (" + message + ")");
  }

  private static HaraException invalid(String origin, String message, RuntimeException cause) {
    return new HaraException(
        "extension/binding-invalid: " + origin + " (" + message + ": " + cause.getMessage() + ")");
  }

  private static HaraException mismatch(String message) {
    return new HaraException("extension/manifest-mismatch: " + message);
  }

  enum Type {
    I32("i32", RawType.I32),
    I64("i64", RawType.I64),
    F32("f32", RawType.F32),
    F64("f64", RawType.F64),
    BOOLEAN("boolean", RawType.I32),
    STRING("string", null),
    BYTES("bytes", null),
    VOID("void", RawType.VOID);

    private final String keyword;
    private final RawType directRaw;

    Type(String keyword, RawType directRaw) {
      this.keyword = keyword;
      this.directRaw = directRaw;
    }

    String keyword() {
      return keyword;
    }

    RawType directRaw() {
      return directRaw;
    }

    boolean memoryValue() {
      return this == STRING || this == BYTES;
    }

    static Type parse(String value, String origin, String subject) {
      for (Type type : values()) if (type.keyword.equals(value)) return type;
      throw invalid(origin, subject + " uses unsupported Hara type :" + value);
    }
  }

  enum RawType {
    I32("i32"),
    I64("i64"),
    F32("f32"),
    F64("f64"),
    VOID("void");

    private final String keyword;

    RawType(String keyword) {
      this.keyword = keyword;
    }

    String keyword() {
      return keyword;
    }

    static RawType parse(String value, String origin, String subject) {
      for (RawType type : values()) if (type.keyword.equals(value)) return type;
      throw invalid(origin, subject + " uses unsupported Wasm type :" + value);
    }
  }

  enum Ownership {
    BORROWED("borrowed"),
    TRANSFERRED("transferred"),
    CALLER("caller"),
    CALLEE("callee");

    private final String keyword;

    Ownership(String keyword) {
      this.keyword = keyword;
    }

    static Ownership parse(String value, String origin, String subject) {
      for (Ownership ownership : values()) {
        if (ownership.keyword.equals(value)) return ownership;
      }
      throw invalid(origin, subject + " uses unsupported ownership :" + value);
    }
  }

  static final class Memory {
    private final String export;
    private final String allocate;
    private final String release;

    private Memory(String export, String allocate, String release) {
      this.export = export;
      this.allocate = allocate;
      this.release = release;
    }

    String export() {
      return export;
    }

    String allocate() {
      return allocate;
    }

    String release() {
      return release;
    }
  }

  static final class Function {
    private final String name;
    private final String wasmExport;
    private final List<Argument> arguments;
    private final Result result;

    private Function(String name, String wasmExport, List<Argument> arguments, Result result) {
      this.name = name;
      this.wasmExport = wasmExport;
      this.arguments = Collections.unmodifiableList(new ArrayList<>(arguments));
      this.result = result;
    }

    String name() {
      return name;
    }

    String wasmExport() {
      return wasmExport;
    }

    List<Argument> arguments() {
      return arguments;
    }

    Result result() {
      return result;
    }

    List<String> publicArgumentTypes() {
      ArrayList<String> result = new ArrayList<>();
      for (Argument argument : arguments) result.add(argument.type().keyword());
      return Collections.unmodifiableList(result);
    }
  }

  static final class Argument {
    private final String name;
    private final Type type;
    private final List<RawType> rawTypes;
    private final boolean pointerLength;
    private final Ownership ownership;

    private Argument(
        String name,
        Type type,
        List<RawType> rawTypes,
        boolean pointerLength,
        Ownership ownership) {
      this.name = name;
      this.type = type;
      this.rawTypes = Collections.unmodifiableList(new ArrayList<>(rawTypes));
      this.pointerLength = pointerLength;
      this.ownership = ownership;
    }

    String name() {
      return name;
    }

    Type type() {
      return type;
    }

    List<RawType> rawTypes() {
      return rawTypes;
    }

    boolean pointerLength() {
      return pointerLength;
    }

    Ownership ownership() {
      return ownership;
    }
  }

  static final class Result {
    private final Type type;
    private final RawType rawType;
    private final boolean packedI64;
    private final Ownership ownership;

    private Result(Type type, RawType rawType, boolean packedI64, Ownership ownership) {
      this.type = type;
      this.rawType = rawType;
      this.packedI64 = packedI64;
      this.ownership = ownership;
    }

    Type type() {
      return type;
    }

    RawType rawType() {
      return rawType;
    }

    boolean packedI64() {
      return packedI64;
    }

    Ownership ownership() {
      return ownership;
    }
  }
}
