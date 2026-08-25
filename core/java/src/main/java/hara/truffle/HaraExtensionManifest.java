package hara.truffle;

import hara.kernel.base.Parser;
import hara.lang.data.Keyword;
import hara.lang.data.Symbol;
import hara.lang.protocol.ILinearType;
import hara.lang.protocol.IMapType;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Iterator;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.Map;
import java.util.Set;
import java.util.regex.Pattern;

/** Strict metadata for a provider-generated namespace loaded through {@code :require}. */
public final class HaraExtensionManifest {
  private static final Pattern NAMESPACE =
      Pattern.compile("[a-z][a-z0-9-]*(\\.[a-z0-9][a-z0-9-]*)+");
  private static final Pattern IDENTITY =
      Pattern.compile("[a-z][a-z0-9-]*/[a-z][a-z0-9-]*(\\.[a-z0-9][a-z0-9-]*)*");
  private static final Pattern HANDLE_TAG =
      Pattern.compile("[a-z][a-z0-9-]*(\\.[a-z][a-z0-9-]*)*");
  private static final Pattern HANDLE_TYPE = Pattern.compile("[a-z][a-z0-9-]*");
  private static final Set<String> FIELDS =
      Set.of(
          "namespace",
          "identity",
          "version",
          "provider",
          "module",
          "abi",
          "exports",
          "callbacks",
          "capabilities",
          "host-calls",
          "handles",
          "targets",
          "assets");
  private static final Set<String> REQUIRED_FIELDS =
      Set.of("namespace", "version", "provider", "abi", "exports", "capabilities");
  private static final Set<String> EXPORT_FIELDS =
      Set.of("wasm/export", "args", "returns", "async", "operation", "reentrant");
  private static final Set<String> HANDLE_FIELDS = Set.of("tag", "release");

  private final String namespace;
  private final String identity;
  private final String version;
  private final String provider;
  private final String module;
  private final String abi;
  private final Map<String, Target> targets;
  private final java.util.List<String> assets;
  private final Map<String, Export> exports;
  private final Map<String, Export> callbacks;
  private final java.util.List<String> capabilities;
  private final Map<String, java.util.List<String>> hostCalls;
  private final Map<String, java.util.List<String>> hostCallCapabilities;
  private final Map<String, String> handleTags;
  private final Map<String, String> handleReleases;

  private HaraExtensionManifest(
      String namespace,
      String identity,
      String version,
      String provider,
      String module,
      String abi,
      Map<String, Target> targets,
      java.util.List<String> assets,
      Map<String, Export> exports,
      Map<String, Export> callbacks,
      java.util.List<String> capabilities,
      Map<String, java.util.List<String>> hostCalls,
      Map<String, java.util.List<String>> hostCallCapabilities,
      Map<String, String> handleTags,
      Map<String, String> handleReleases) {
    this.namespace = namespace;
    this.identity = identity;
    this.version = version;
    this.provider = provider;
    this.module = module;
    this.abi = abi;
    this.targets = Collections.unmodifiableMap(new LinkedHashMap<>(targets));
    this.assets = Collections.unmodifiableList(new ArrayList<>(assets));
    this.exports = Collections.unmodifiableMap(new LinkedHashMap<>(exports));
    this.callbacks = Collections.unmodifiableMap(new LinkedHashMap<>(callbacks));
    this.capabilities = Collections.unmodifiableList(new ArrayList<>(capabilities));
    LinkedHashMap<String, java.util.List<String>> copiedHostCalls = new LinkedHashMap<>();
    hostCalls.forEach(
        (service, methods) ->
            copiedHostCalls.put(service, Collections.unmodifiableList(new ArrayList<>(methods))));
    this.hostCalls = Collections.unmodifiableMap(copiedHostCalls);
    LinkedHashMap<String, java.util.List<String>> copiedHostCallCapabilities = new LinkedHashMap<>();
    hostCallCapabilities.forEach(
        (call, required) ->
            copiedHostCallCapabilities.put(
                call, Collections.unmodifiableList(new ArrayList<>(required))));
    this.hostCallCapabilities = Collections.unmodifiableMap(copiedHostCallCapabilities);
    this.handleTags = Collections.unmodifiableMap(new LinkedHashMap<>(handleTags));
    this.handleReleases = Collections.unmodifiableMap(new LinkedHashMap<>(handleReleases));
  }

  public static HaraExtensionManifest parse(String source, String origin) {
    final Object value;
    try {
      value = Parser.LispReader.readString(source, null);
    } catch (RuntimeException error) {
      throw invalid(origin, "cannot parse manifest", error);
    }
    if (!(value instanceof IMapType<?, ?>)) throw invalid(origin, "manifest must be a map");
    IMapType<?, ?> map = (IMapType<?, ?>) value;
    rejectUnknownKeys(map, FIELDS, origin, "manifest");

    String namespace = requireString(map, "namespace", origin);
    if (!NAMESPACE.matcher(namespace).matches()) {
      throw invalid(origin, "namespace must be a qualified lower-case symbol");
    }
    Object identityValue = lookup(map, "identity");
    String identity = identityValue == null ? null : requireString(map, "identity", origin);
    if (identity != null && !IDENTITY.matcher(identity).matches()) {
      throw invalid(origin, "identity must be a lower-case owner/name coordinate");
    }
    String version = requireString(map, "version", origin);
    String provider = requireKeyword(map, "provider", origin);
    String abi = requireKeyword(map, "abi", origin);
    Object moduleValue = lookup(map, "module");
    String module = moduleValue == null ? null : requireString(map, "module", origin);
    Map<String, Target> targets = parseTargets(lookup(map, "targets"), origin);
    java.util.List<String> assets = parseAssetPaths(lookup(map, "assets"), origin);
    if ("wasm".equals(provider)) {
      if (module == null || !targets.isEmpty()) {
        throw invalid(origin, "WASM providers require :module and cannot declare :targets");
      }
      requireRelativePath(module, ".wasm", origin, "module");
    } else if ("hta".equals(provider)) {
      if (module != null || targets.isEmpty() || !"hta.v1".equals(abi)) {
        throw invalid(origin, "HTA providers require :abi :hta.v1 and :targets, without :module");
      }
    } else {
      throw invalid(origin, "unsupported provider :" + provider);
    }
    Map<String, Export> exports = parseExports(lookup(map, "exports"), origin);
    Map<String, Export> callbacks = parseCallbacks(lookup(map, "callbacks"), origin);
    java.util.List<String> capabilities =
        parseKeywords(lookup(map, "capabilities"), origin, "capabilities");
    Map<String, java.util.List<String>> hostCalls =
        parseHostCalls(lookup(map, "host-calls"), origin);
    Map<String, java.util.List<String>> hostCallCapabilities =
        parseHostCallCapabilities(lookup(map, "host-calls"), origin);
    Map<String, String> handleTags = new LinkedHashMap<>();
    Map<String, String> handleReleases = parseHandleTags(lookup(map, "handles"), handleTags, origin);
    return new HaraExtensionManifest(
        namespace,
        identity,
        version,
        provider,
        module,
        abi,
        targets,
        assets,
        exports,
        callbacks,
        capabilities,
        hostCalls,
        hostCallCapabilities,
        handleTags,
        handleReleases);
  }

  public String namespace() {
    return namespace;
  }

  public String identity() {
    return identity;
  }

  public String version() {
    return version;
  }

  public String provider() {
    return provider;
  }

  public String module() {
    return module;
  }

  public String abi() {
    return abi;
  }

  public Map<String, Target> targets() {
    return targets;
  }

  public java.util.List<String> assets() {
    return assets;
  }

  public Target target(String host) {
    return targets.get(host);
  }

  public Map<String, Export> exports() {
    return exports;
  }

  public Map<String, Export> callbacks() {
    return callbacks;
  }

  public java.util.List<String> capabilities() {
    return capabilities;
  }

  public Map<String, java.util.List<String>> hostCalls() {
    return hostCalls;
  }

  public java.util.List<String> hostCallCapabilities(String service, String method) {
    return hostCallCapabilities.getOrDefault(service + "/" + method, java.util.List.of());
  }

  public Map<String, java.util.List<String>> hostCallCapabilities() {
    return hostCallCapabilities;
  }

  public String handleTag(String type) {
    return handleTags.get(type);
  }

  public String handleRelease(String type) {
    return handleReleases.get(type);
  }

  public boolean declaresHandles() {
    return !handleTags.isEmpty();
  }

  public boolean permitsHostCall(String service, String method) {
    return hostCalls.getOrDefault(service, java.util.List.of()).contains(method);
  }

  private static Map<String, Target> parseTargets(Object value, String origin) {
    if (value == null) return Map.of();
    if (!(value instanceof IMapType<?, ?>)) throw invalid(origin, "targets must be a map");
    LinkedHashMap<String, Target> result = new LinkedHashMap<>();
    Iterator<?> iterator = ((IMapType<?, ?>) value).iterator();
    while (iterator.hasNext()) {
      Map.Entry<?, ?> entry = (Map.Entry<?, ?>) iterator.next();
      if (!(entry.getKey() instanceof Keyword) || !(entry.getValue() instanceof IMapType<?, ?>)) {
        throw invalid(origin, "targets must map host keywords to target maps");
      }
      String host = ((Keyword) entry.getKey()).getName();
      if (!"node".equals(host) && !"browser".equals(host)) {
        throw invalid(origin, "unsupported target :" + host);
      }
      IMapType<?, ?> target = (IMapType<?, ?>) entry.getValue();
      rejectUnknownKeys(target, Set.of("provider", "runtime"), origin, "target " + host);
      String targetProvider = requireString(target, "provider", origin);
      requireRelativePath(targetProvider, ".mjs", origin, "target provider");
      String runtime = requireKeyword(target, "runtime", origin);
      if (("node".equals(host) && !"process".equals(runtime))
          || ("browser".equals(host) && !"web-worker".equals(runtime))) {
        throw invalid(origin, "target " + host + " has incompatible runtime :" + runtime);
      }
      result.put(host, new Target(targetProvider, runtime));
    }
    return result;
  }

  private static java.util.List<String> parseAssetPaths(Object value, String origin) {
    if (value == null) return java.util.List.of();
    if (!(value instanceof ILinearType<?>)) throw invalid(origin, "assets must be a vector");
    ArrayList<String> result = new ArrayList<>();
    for (Object item : (ILinearType<?>) value) {
      if (!(item instanceof String)) throw invalid(origin, "assets must contain strings");
      String asset = (String) item;
      requireRelativePath(asset, null, origin, "asset");
      if (result.contains(asset)) throw invalid(origin, "duplicate asset " + asset);
      result.add(asset);
    }
    return result;
  }

  private static void requireRelativePath(
      String value, String suffix, String origin, String field) {
    Path path;
    try {
      path = Path.of(value);
    } catch (RuntimeException error) {
      throw invalid(origin, field + " has an invalid path", error);
    }
    if (path.isAbsolute()
        || path.normalize().startsWith("..")
        || value.contains(":")
        || (suffix != null && !value.endsWith(suffix))) {
      throw invalid(
          origin,
          field + " must be a relative " + (suffix == null ? "package" : suffix) + " file");
    }
  }

  private static Map<String, String> parseHandleTags(
      Object value, Map<String, String> tags, String origin) {
    if (value == null) return Map.of();
    if (!(value instanceof IMapType<?, ?>)) throw invalid(origin, "handles must be a map");
    LinkedHashMap<String, String> result = new LinkedHashMap<>();
    Iterator<?> iterator = ((IMapType<?, ?>) value).iterator();
    while (iterator.hasNext()) {
      Map.Entry<?, ?> entry = (Map.Entry<?, ?>) iterator.next();
      if (!(entry.getKey() instanceof String)
          || !HANDLE_TYPE.matcher((String) entry.getKey()).matches()) {
        throw invalid(origin, "handle types must be lower-case strings");
      }
      if (!(entry.getValue() instanceof IMapType<?, ?>)) {
        throw invalid(origin, "handle " + entry.getKey() + " must be a map");
      }
      IMapType<?, ?> spec = (IMapType<?, ?>) entry.getValue();
      rejectUnknownKeys(spec, HANDLE_FIELDS, origin, "handle " + entry.getKey());
      Object tagValue = lookup(spec, "tag");
      if (!(tagValue instanceof Symbol) || ((Symbol) tagValue).getNamespace() != null) {
        throw invalid(origin, "handle tags must be unqualified symbols");
      }
      String tag = ((Symbol) tagValue).getName();
      if (!HANDLE_TAG.matcher(tag).matches()) {
        throw invalid(origin, "handle tags must be lower-case symbols");
      }
      tags.put((String) entry.getKey(), tag);
      Object releaseValue = lookup(spec, "release");
      if (releaseValue != null) result.put((String) entry.getKey(), requireString(spec, "release", origin));
    }
    return result;
  }

  private static Map<String, java.util.List<String>> parseHostCalls(
      Object value, String origin) {
    if (value == null) return Map.of();
    if (!(value instanceof IMapType<?, ?>)) throw invalid(origin, "host-calls must be a map");
    LinkedHashMap<String, java.util.List<String>> result = new LinkedHashMap<>();
    Iterator<?> iterator = ((IMapType<?, ?>) value).iterator();
    while (iterator.hasNext()) {
      Map.Entry<?, ?> entry = (Map.Entry<?, ?>) iterator.next();
      if (!(entry.getKey() instanceof String) || ((String) entry.getKey()).isBlank()) {
        throw invalid(origin, "host-call services must be non-empty strings");
      }
      Object methods = entry.getValue();
      if (methods instanceof IMapType<?, ?>) methods = lookup((IMapType<?, ?>) methods, "methods");
      if (!(methods instanceof ILinearType<?>)) throw invalid(origin, "host-call methods must be vectors");
      ArrayList<String> names = new ArrayList<>();
      for (Object method : (ILinearType<?>) methods) {
        if (!(method instanceof String) || ((String) method).isBlank()) {
          throw invalid(origin, "host-call methods must be non-empty strings");
        }
        names.add((String) method);
      }
      result.put((String) entry.getKey(), names);
    }
    return result;
  }

  private static Map<String, java.util.List<String>> parseHostCallCapabilities(
      Object value, String origin) {
    if (value == null) return Map.of();
    if (!(value instanceof IMapType<?, ?>)) throw invalid(origin, "host-calls must be a map");
    LinkedHashMap<String, java.util.List<String>> result = new LinkedHashMap<>();
    Iterator<?> iterator = ((IMapType<?, ?>) value).iterator();
    while (iterator.hasNext()) {
      Map.Entry<?, ?> entry = (Map.Entry<?, ?>) iterator.next();
      if (!(entry.getKey() instanceof String) || !(entry.getValue() instanceof IMapType<?, ?>)) {
        continue;
      }
      String service = (String) entry.getKey();
      Object methods = lookup((IMapType<?, ?>) entry.getValue(), "methods");
      Object capabilities = lookup((IMapType<?, ?>) entry.getValue(), "capabilities");
      if (capabilities == null) continue;
      if (!(capabilities instanceof ILinearType<?>)) {
        throw invalid(origin, "host-call capabilities must be a vector");
      }
      ArrayList<String> required = new ArrayList<>();
      for (Object capability : (ILinearType<?>) capabilities) {
        if (!(capability instanceof Keyword) || ((Keyword) capability).getNamespace() != null) {
          throw invalid(origin, "host-call capabilities must be keywords");
        }
        required.add(((Keyword) capability).getName());
      }
      if (!(methods instanceof ILinearType<?>)) throw invalid(origin, "host-call methods must be vectors");
      for (Object method : (ILinearType<?>) methods) {
        if (!(method instanceof String)) throw invalid(origin, "host-call methods must be non-empty strings");
        result.put(service + "/" + method, required);
      }
    }
    return result;
  }

  private static Map<String, Export> parseExports(Object value, String origin) {
    if (!(value instanceof IMapType<?, ?>)) throw invalid(origin, "exports must be a map");
    Map<String, Export> result = new LinkedHashMap<>();
    Iterator<?> iterator = ((IMapType<?, ?>) value).iterator();
    while (iterator.hasNext()) {
      Map.Entry<?, ?> entry = (Map.Entry<?, ?>) iterator.next();
      if (!(entry.getKey() instanceof String) || ((String) entry.getKey()).isBlank()) {
        throw invalid(origin, "export names must be non-empty strings");
      }

      if (!(entry.getValue() instanceof IMapType<?, ?>)) {
        throw invalid(origin, "export " + entry.getKey() + " must be a map");
      }
      String name = (String) entry.getKey();
      IMapType<?, ?> spec = (IMapType<?, ?>) entry.getValue();
      rejectUnknownKeys(spec, EXPORT_FIELDS, origin, "export " + name);
      Object wasmExportValue = lookup(spec, "wasm/export");
      String wasmExport =
          wasmExportValue == null ? name : requireString(spec, "wasm/export", origin);
      java.util.List<String> arguments =
          parseKeywords(lookup(spec, "args"), origin, "export args");
      String returns = requireKeyword(spec, "returns", origin);
      Object asyncValue = lookup(spec, "async");
      if (asyncValue != null && !(asyncValue instanceof Boolean)) {
        throw invalid(origin, "export async must be boolean");
      }
      Object operationValue = lookup(spec, "operation");
      String operation =
          operationValue == null ? name : requireString(spec, "operation", origin);
      result.put(
          name,
          new Export(wasmExport, operation, arguments, returns, Boolean.TRUE.equals(asyncValue)));
    }
    if (result.isEmpty()) throw invalid(origin, "exports cannot be empty");
    return result;
  }

  private static Map<String, Export> parseCallbacks(Object value, String origin) {
    if (value == null) return Map.of();
    if (!(value instanceof IMapType<?, ?> map)) throw invalid(origin, "callbacks must be a map");
    Iterator<?> iterator = map.iterator();
    while (iterator.hasNext()) {
      Map.Entry<?, ?> entry = (Map.Entry<?, ?>) iterator.next();
      if (!(entry.getValue() instanceof IMapType<?, ?> specification)) {
        throw invalid(origin, "callback specification must be a map");
      }
      Object reentrant = lookup(specification, "reentrant");
      if (Boolean.TRUE.equals(reentrant)) {
        throw invalid(origin, "callbacks cannot be reentrant");
      }
    }
    if (!map.iterator().hasNext()) return Map.of();
    return parseExports(value, origin);
  }

  private static java.util.List<String> parseKeywords(
      Object value, String origin, String field) {
    if (!(value instanceof ILinearType<?>)) throw invalid(origin, field + " must be a vector");
    ArrayList<String> result = new ArrayList<>();
    for (Object item : (ILinearType<?>) value) {
      if (!(item instanceof Keyword)) throw invalid(origin, field + " must contain keywords");
      result.add(((Keyword) item).getName());
    }
    return result;
  }

  private static String requireString(IMapType<?, ?> map, String field, String origin) {
    Object value = lookup(map, field);
    if (!(value instanceof String) || ((String) value).isBlank()) {
      throw invalid(origin, field + " must be a non-empty string");
    }
    return (String) value;
  }

  private static String requireKeyword(IMapType<?, ?> map, String field, String origin) {
    Object value = lookup(map, field);
    if (!(value instanceof Keyword)) throw invalid(origin, field + " must be a keyword");
    return ((Keyword) value).getName();
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  private static Object lookup(IMapType<?, ?> map, String field) {
    return ((IMapType) map).lookup(Keyword.create(field));
  }

  private static void rejectUnknownKeys(
      IMapType<?, ?> map, Set<String> allowed, String origin, String subject) {
    LinkedHashSet<String> seen = new LinkedHashSet<>();
    Iterator<?> iterator = map.iterator();
    while (iterator.hasNext()) {
      Object key = ((Map.Entry<?, ?>) iterator.next()).getKey();
      if (!(key instanceof Keyword)) throw invalid(origin, subject + " keys must be keywords");
      String name = keywordName((Keyword) key);
      if (!allowed.contains(name)) {
        throw invalid(origin, "unsupported " + subject + " field: " + name);
      }
      seen.add(name);
    }
    if (subject.equals("manifest") && !seen.containsAll(REQUIRED_FIELDS)) {
      LinkedHashSet<String> missing = new LinkedHashSet<>(REQUIRED_FIELDS);
      missing.removeAll(seen);
      throw invalid(origin, "missing manifest fields: " + missing);
    }
  }

  private static String keywordName(Keyword keyword) {
    return keyword.getNamespace() == null
        ? keyword.getName()
        : keyword.getNamespace() + "/" + keyword.getName();
  }

  private static IllegalArgumentException invalid(String origin, String message) {
    return new IllegalArgumentException("extension/malformed " + origin + ": " + message);
  }

  private static IllegalArgumentException invalid(
      String origin, String message, RuntimeException cause) {
    return new IllegalArgumentException("extension/malformed " + origin + ": " + message, cause);
  }

  public static final class Target {
    private final String provider;
    private final String runtime;

    private Target(String provider, String runtime) {
      this.provider = provider;
      this.runtime = runtime;
    }

    public String provider() {
      return provider;
    }

    public String runtime() {
      return runtime;
    }
  }

  public static final class Export {
    private final String wasmExport;
    private final String operation;
    private final java.util.List<String> arguments;
    private final String returns;
    private final boolean async;

    private Export(
        String wasmExport,
        String operation,
        java.util.List<String> arguments,
        String returns,
        boolean async) {
      this.wasmExport = wasmExport;
      this.operation = operation;
      this.arguments = Collections.unmodifiableList(new ArrayList<>(arguments));
      this.returns = returns;
      this.async = async;
    }

    public String wasmExport() {
      return wasmExport;
    }

    public String operation() {
      return operation;
    }

    public java.util.List<String> arguments() {
      return arguments;
    }

    public String returns() {
      return returns;
    }

    public boolean async() {
      return async;
    }
  }
}
