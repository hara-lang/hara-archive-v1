package hara.truffle;

import com.oracle.truffle.api.CompilerDirectives.TruffleBoundary;
import com.oracle.truffle.api.TruffleLanguage;
import com.oracle.truffle.api.TruffleFile;
import com.oracle.truffle.api.source.Source;
import hara.kernel.base.Parser;
import hara.kernel.builtin.BuiltinStruct;
import hara.kernel.flavor.NativeCapability;
import hara.kernel.flavor.NativeFlavorAccess;
import hara.kernel.flavor.NativeFlavorException;
import hara.kernel.flavor.NativeFlavorImportSpecs;
import hara.kernel.flavor.NativeFlavorProvider;
import hara.kernel.flavor.NativeFlavorRegistry;
import hara.kernel.jvm.JvmFlavorProvider;
import hara.lang.base.Iter;
import hara.lang.base.Reduced;
import hara.lang.base.Eq;
import hara.lang.base.G;
import hara.lang.base.NumUtils;
import hara.lang.base.primitive.Num;
import hara.lang.base.iter.CloseableIterator;
import hara.lang.data.Symbol;
import hara.lang.data.List;
import hara.lang.data.Keyword;
import hara.lang.data.HaraCharacter;
import hara.lang.protocol.IMapType;
import hara.lang.protocol.ILinearType;
import hara.lang.protocol.Constant;
import hara.lang.data.types.ObjFn;
import hara.lang.protocol.IFn;
import hara.lang.protocol.IMetadata;
import hara.lang.protocol.ILookup;
import hara.lang.protocol.IDeref;
import hara.lang.protocol.IDerefTimeout;
import hara.lang.protocol.IDisplay;
import hara.lang.protocol.ICount;
import hara.lang.protocol.IConj;
import hara.lang.protocol.ICons;
import hara.lang.protocol.IEmpty;
import hara.lang.protocol.IPushLast;
import hara.truffle.bytecode.HbcProgram;
import hara.lang.protocol.INth;
import hara.lang.protocol.IPromise;
import java.io.File;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.InetSocketAddress;
import java.net.ServerSocket;
import java.net.Socket;
import java.nio.charset.StandardCharsets;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.util.LinkedHashMap;
import java.util.Collections;
import java.util.Map;
import java.util.Set;
import java.util.Deque;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Iterator;
import java.util.HexFormat;
import java.util.NoSuchElementException;
import java.util.LinkedHashSet;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionException;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicLong;
import java.util.concurrent.atomic.AtomicReference;
import java.util.function.Consumer;
import java.util.function.DoubleUnaryOperator;
import java.util.function.Function;
import java.util.function.Supplier;

public final class HaraContext {
  private static final String INTRINSIC_NAMESPACE = "hara.lang.intrinsic";
  private static final String FOUNDATION_NAMESPACE = "std.foundation";
  private static final String PROTOCOL_NAMESPACE_PREFIX = "std.protocol.";
  private record NativeLibraryInstaller(HaraVar.Origin origin, Consumer<HaraContext> install) {}

  private static final Map<String, NativeLibraryInstaller> NATIVE_LIBRARY_INSTALLERS =
      Map.ofEntries(
          Map.entry(
              "std.native.String",
              new NativeLibraryInstaller(
                  HaraVar.Origin.RUNTIME_PRIMITIVE, HaraContext::defineStringLibrary)),
          Map.entry(
              "std.native.Bytes",
              new NativeLibraryInstaller(
                  HaraVar.Origin.RUNTIME_PRIMITIVE, HaraContext::defineBytesLibrary)),
          Map.entry(
              "std.native.Promise",
              new NativeLibraryInstaller(
                  HaraVar.Origin.RUNTIME_PRIMITIVE, HaraContext::definePromiseLibrary)),
          Map.entry(
              "std.native.Coroutine",
              new NativeLibraryInstaller(
                  HaraVar.Origin.RUNTIME_PRIMITIVE,
                  context -> StdFoundationCoroutine.install(context, "std.native.Coroutine"))),
          Map.entry(
              "std.native.File",
              new NativeLibraryInstaller(
                  HaraVar.Origin.JAVA_LIBRARY, HaraContext::defineFileLibrary)),
          Map.entry(
              "std.native.Json",
              new NativeLibraryInstaller(
                  HaraVar.Origin.JAVA_LIBRARY, HaraContext::defineJsonLibrary)),
          Map.entry(
              "std.native.Socket",
              new NativeLibraryInstaller(
                  HaraVar.Origin.JAVA_LIBRARY, HaraContext::defineSocketLibrary)),
          Map.entry(
              "std.native.OS",
              new NativeLibraryInstaller(
                  HaraVar.Origin.JAVA_LIBRARY, HaraContext::defineOsLibrary)));
  private final TruffleLanguage.Env environment;
  private final ThreadLocal<Deque<OutputStream>> printerOutputs =
      ThreadLocal.withInitial(ArrayDeque::new);
  private final HaraEvaluationRuntime evaluationRuntime;
  private final Keyword testRunner;
  private final boolean sandboxRestricted;
  private final SessionKernel sessionKernel;
  private final HaraInstrumentationRuntime instrumentationRuntime;
  private final FilesystemRuntimeBinding filesystemRuntime;
  private final HaraNativeCapabilityBoundary nativeCapabilityBoundary;
  private final IFilesystem ownedFilesystem;
  private final java.util.List<Object> nativeTestResults = new ArrayList<>();
  private final Map<String, HaraNamespace> namespaces = new ConcurrentHashMap<>();
  private final Map<String, Map<String, HaraMacro>> macros = new ConcurrentHashMap<>();
  private final Map<String, Map<String, String>> aliases = new ConcurrentHashMap<>();
  private final Map<String, String> globalAliases = new ConcurrentHashMap<>();
  private final Map<String, String> globalImports = new ConcurrentHashMap<>();
  private final Map<String, NamespaceLoadState> namespaceStates = new ConcurrentHashMap<>();
  private final Map<String, String> namespaceFailures = new ConcurrentHashMap<>();
  private final Map<String, String> nativeFlavors = new ConcurrentHashMap<>();
  private final Map<String, Map<String, Object>> nativeImports = new ConcurrentHashMap<>();
  private final NativeFlavorRegistry nativeFlavorRegistry =
      new NativeFlavorRegistry().register(JvmFlavorProvider.INSTANCE);
  private final HaraExtensionRegistry extensionRegistry =
      new HaraExtensionRegistry(HaraContext.class.getClassLoader());
  private final HaraLibraryLoader libraryLoader = new HaraLibraryLoader();
  private final HbxBundleLibrary bytecodeLibrary =
      new HbxBundleLibrary(HaraContext.class.getClassLoader());
  private HaraVar.Origin definitionOrigin = HaraVar.Origin.SOURCE;
  private boolean eagerFallbacksLoading;
  private boolean eagerFallbacksLoaded;
  private volatile HaraProject project;
  private volatile boolean projectDiscovered;
  private final Map<String, HaraExtensionRuntime> loadedExtensions = new ConcurrentHashMap<>();
  private final Map<String, JvmPackageLoader.LoadedArtifact> loadedJvmFlavors =
      new ConcurrentHashMap<>();
  private volatile ClassLoader nativeFlavorLoader;
  private final Map<String, ModuleRecord> modules = new ConcurrentHashMap<>();
  private final Map<String, Set<String>> moduleDependencies = new ConcurrentHashMap<>();
  private final Map<String, Object> libraryStates = new ConcurrentHashMap<>();
  private final Map<String, Object> intrinsicCollectionBuiltins = new ConcurrentHashMap<>();
  private volatile Object intrinsicFirstFunction;
  private volatile Object intrinsicRestFunction;
  private final Set<String> loadingModules = ConcurrentHashMap.newKeySet();
  private final Set<String> preparedNamespaceReloads = ConcurrentHashMap.newKeySet();
  private final Set<String> blankNamespaces = ConcurrentHashMap.newKeySet();
  private final Deque<String> loadingStack = new ArrayDeque<>();
  private boolean preparingNamespace;
  private volatile HaraNamespace currentNamespace;
  private final Map<String, Map<String, BuiltinExport>> builtinCatalogs = new ConcurrentHashMap<>();
  private boolean collectingBuiltins;
  private String collectingBuiltinNamespace;
  private HaraProtocol ifnProtocol;
  private Map<String, HaraProtocol> protocolDeclarations = Map.of();
  private final AtomicLong gensymCounter = new AtomicLong();
  HaraContext(TruffleLanguage.Env environment) {
    this.environment = environment;
    this.evaluationRuntime =
        new HaraEvaluationRuntime(source -> environment.parsePublic(source).call());
    this.testRunner = runtimeTestRunner(environment.getOptions().get(HaraLanguage.TEST_RUNNER));
    this.sandboxRestricted = environment.getOptions().get(HaraLanguage.SANDBOX_RESTRICTED);
    this.sessionKernel =
        sandboxRestricted
            ? null
            : SessionKernel.embedding(environment.getOptions().get(HaraLanguage.KERNEL_TOKEN));
    String instrumentationSessionId =
        sessionKernel == null ? null : environment.getOptions().get(HaraLanguage.SESSION_ID);
    this.instrumentationRuntime =
        new HaraInstrumentationRuntime(sessionKernel, instrumentationSessionId);
    FilesystemRuntimeBinding attachedFilesystem =
        FilesystemContextBindings.claim(
            environment.getOptions().get(HaraLanguage.FILESYSTEM_BINDING_TOKEN));
    if (attachedFilesystem != null) {
      this.filesystemRuntime = attachedFilesystem;
      this.ownedFilesystem = null;
    } else if (environment.isFileIOAllowed()) {
      String workingDirectory =
          environment.getPublicTruffleFile(".").getAbsoluteFile().normalize().getPath();
      NativeFilesystem nativeFilesystem =
          FilesystemContextBindings.nativeFilesystem(Path.of(workingDirectory));
      this.filesystemRuntime = new FilesystemRuntimeBinding(nativeFilesystem);
      this.ownedFilesystem = nativeFilesystem;
    } else {
      this.filesystemRuntime = null;
      this.ownedFilesystem = null;
    }
    this.nativeCapabilityBoundary =
        new HaraNativeCapabilityBoundary(
            sessionKernel != null,
            sessionKernel != null && !sandboxRestricted,
            filesystemRuntime != null,
            environment.isSocketIOAllowed(),
            environment.isCreateProcessAllowed(),
            false);

    currentNamespace = namespace(INTRINSIC_NAMESPACE);
    withDefinitionOrigin(
        HaraVar.Origin.RUNTIME_PRIMITIVE,
        () -> {
          installNativeTypeDescriptors();
          installNativeResultBuiltins();
          HaraProtocolDeclarations.Registry registry = HaraProtocolDeclarations.install(this);
          protocolDeclarations = registry.protocols();
          ifnProtocol = registry.protocols().get("IFn");
          if (ifnProtocol == null) throw new HaraException("Missing injected IFn protocol");
          HaraProtocolRuntime.install(this, registry);
          installNativeStreamBuiltins();
          collectBuiltins(
              FOUNDATION_NAMESPACE,
              () -> {
                installNumericBuiltins(namespace(FOUNDATION_NAMESPACE));
                installCoreBuiltins(namespace(FOUNDATION_NAMESPACE));
              });
          installFoundationBootstrapSeeds();
        });
    installProjectMacro();
    installNativeLibraries();
    installEnvironmentLibraries();
    libraryLoader.installEagerJava(this);
    hideIteratorImplementationBindings();
    namespaceStates.put(FOUNDATION_NAMESPACE, NamespaceLoadState.UNLOADED);
    for (String namespace : HaraBuiltinCatalog.GENERATED_LIBRARIES.values()) {
      namespaceStates.put(namespace, NamespaceLoadState.UNLOADED);
    }
    for (String namespace : bytecodeLibrary.namespaces()) {
      namespaceStates.put(namespace, NamespaceLoadState.UNLOADED);
    }
    currentNamespace = namespace("user");
    initializeUserNamespace(currentNamespace);
  }

  void markInstrumentationReady() {
    instrumentationRuntime.markReady();
  }

  void publishInterpreterEvent(
      InstrumentationModel.EventKind event,
      com.oracle.truffle.api.source.SourceSection source,
      java.util.Map<String, String> data) {
    instrumentationRuntime.publishInterpreterEvent(event, source, data);
  }

  void publishHbcEvent(
      InstrumentationModel.EventKind event,
      int instructionPointer,
      String function,
      String sourceId,
      java.util.Map<String, String> data) {
    instrumentationRuntime.publishHbcEvent(
        event, instructionPointer, function, sourceId, data);
  }

  void publishHbcEvent(
      InstrumentationModel.EventKind event,
      int instructionPointer,
      String function,
      String sourceId,
      hara.truffle.bytecode.HbcProgram.Position position,
      java.util.Map<String, String> data,
      InstrumentationEventAccess access) {
    instrumentationRuntime.publishHbcEvent(
        event, instructionPointer, function, sourceId, position, data, access);
  }

  boolean hbcInstrumentationEnabled(InstrumentationModel.EventKind event) {
    return instrumentationRuntime.hbcInstrumentationEnabled(event);
  }

  public boolean hbcNativeExecutionAllowed() {
    return instrumentationRuntime.hbcNativeExecutionAllowed();
  }

  boolean hbcPromisePending(Object value) {
    Object unwrapped = HaraBox.unwrap(value);
    return unwrapped instanceof IPromise promise
        && Keyword.create("pending").equals(promise.state());
  }

  Object hbcPromiseValue(Object value) {
    Object unwrapped = HaraBox.unwrap(value);
    if (unwrapped instanceof IPromise promise) return HaraBox.unwrap(promise.value());
    return HaraBox.unwrap(value);
  }

  InstrumentationModel.InstrumentDirective pollHbcDirective() {
    return instrumentationRuntime.pollHbcDirective();
  }

  synchronized HbcMachine.HbcContinuation hbcContinuation(HbcProgram program) {
    return instrumentationRuntime.hbcContinuation(program);
  }

  synchronized void retainHbcContinuation(HbcMachine.HbcContinuation continuation) {
    instrumentationRuntime.retainHbcContinuation(continuation);
  }

  synchronized void clearHbcContinuation(HbcMachine.HbcContinuation continuation) {
    instrumentationRuntime.clearHbcContinuation(continuation);
  }

  synchronized void clearHbcContinuation() {
    instrumentationRuntime.clearHbcContinuation();
  }

  public boolean enterInterpreterRoot() {
    return instrumentationRuntime.enterInterpreterRoot();
  }

  public void exitInterpreterRoot() {
    instrumentationRuntime.exitInterpreterRoot();
  }

  public void publishInterpreterTerminal(
      com.oracle.truffle.api.source.SourceSection source, String status) {
    instrumentationRuntime.publishInterpreterTerminal(source, status);
  }

  public void publishInterpreterSemanticBoundary(
      com.oracle.truffle.api.source.SourceSection source) {
    instrumentationRuntime.publishInterpreterSemanticBoundary(source);
  }

  public void publishInterpreterTopLevelFailure(
      com.oracle.truffle.api.source.SourceSection source, RuntimeException error) {
    instrumentationRuntime.publishInterpreterTopLevelFailure(source, error);
  }

  void installHalcSchemas(HalcArtifact.SchemaIndex schemas) {
    evaluationRuntime.installHalcSchemas(schemas);
  }

  Object halcSchema(String qualifiedVar) {
    return evaluationRuntime.halcSchema(qualifiedVar);
  }

  Object halcFunctionSchema(String qualifiedVar) {
    return evaluationRuntime.halcFunctionSchema(qualifiedVar);
  }

  HalcSchema.Type halcSchemaType(String qualifiedVar) {
    return evaluationRuntime.halcSchemaType(qualifiedVar);
  }

  HalcSchema.Type halcFunctionType(String qualifiedVar) {
    return evaluationRuntime.halcFunctionType(qualifiedVar);
  }

  HalcSchema.Type halcInferredFunctionType(String qualifiedVar) {
    return evaluationRuntime.halcInferredFunctionType(qualifiedVar);
  }

  HalcSchema.Type halcBestFunctionType(String qualifiedVar) {
    HalcSchema.Type declared = halcFunctionType(qualifiedVar);
    return declared != null ? declared : halcInferredFunctionType(qualifiedVar);
  }

  public void installHbcTypes(
      Map<String, HalcSchema.Type> schemaTypes,
      Map<String, HalcSchema.Type> functionTypes,
      Map<String, HalcSchema.Type> inferredFunctionTypes) {
    evaluationRuntime.installHbcTypes(schemaTypes, functionTypes, inferredFunctionTypes);
  }

  TruffleLanguage.Env environment() {
    return environment;
  }

  void closeExtensions() {
    for (HaraExtensionRuntime extension : loadedExtensions.values()) {
      extension.close();
    }
    loadedExtensions.clear();
  }

  void closeContext() {
    instrumentationRuntime.close();
    evaluationRuntime.close();
    closeExtensions();
    for (JvmPackageLoader.LoadedArtifact loaded : loadedJvmFlavors.values()) {
      try {
        loaded.close();
      } catch (Exception ignored) {
        // Context teardown is terminal; the loader has no remaining authority after close.
      }
    }
    loadedJvmFlavors.clear();
    nativeFlavorLoader = null;
    if (ownedFilesystem == null) return;
    filesystemRuntime.close();
    try {
      ownedFilesystem
          .close(IFilesystem.CallContext.create())
          .toCompletableFuture()
          .join();
    } catch (RuntimeException ignored) {
      // Context teardown is already terminal; provider failures cannot restore authority.
    }
  }

  private HaraNamespace namespace(String name) {
    HaraNamespace namespace = namespaces.computeIfAbsent(name, HaraNamespace::new);
    namespaceStates.putIfAbsent(name, NamespaceLoadState.LOADED);
    return namespace;
  }

  private void withDefinitionOrigin(HaraVar.Origin origin, Runnable action) {
    HaraVar.Origin previous = definitionOrigin;
    definitionOrigin = origin;
    try {
      action.run();
    } finally {
      definitionOrigin = previous;
    }
  }

  synchronized void ensureEagerFallbacks() {
    if (eagerFallbacksLoaded || eagerFallbacksLoading) return;
    eagerFallbacksLoading = true;
    try {
      // The canonical Foundation family is source-owned. Load generated HALC
      // before child modules so every public source definition is interned;
      // the HBX inventory selects modules but never gates individual Vars.
      requiredNamespace(FOUNDATION_NAMESPACE);
      if (bytecodeLibrary.available()) {
        for (HbxBundleLibrary.Module module : bytecodeLibrary.eagerModules()) {
          requiredNamespace(module.namespace());
        }
      } else {
        requiredNamespace(FOUNDATION_NAMESPACE);
        for (String namespace : HaraBuiltinCatalog.GENERATED_LIBRARIES.values()) {
          requiredNamespace(namespace);
        }
      }
      // The user namespace exists before the lazy HBX library is activated.
      // Refresh every ordinary namespace after eager modules load so newly
      // materialized std.foundation Vars have the same builtin-alias behavior
      // as the source/Java startup path.
      for (HaraNamespace namespace : namespaces.values()) {
        if (!blankNamespaces.contains(namespace.name())) referFoundation(namespace);
      }
      eagerFallbacksLoaded = true;
    } finally {
      eagerFallbacksLoading = false;
    }
  }

  void collectBuiltins(String namespaceName, Runnable definitions) {
    boolean previousCollecting = collectingBuiltins;
    String previousNamespace = collectingBuiltinNamespace;
    collectingBuiltins = true;
    collectingBuiltinNamespace = namespaceName;
    try {
      definitions.run();
    } finally {
      collectingBuiltins = previousCollecting;
      collectingBuiltinNamespace = previousNamespace;
    }
    installNativeExports(namespaceName);
  }

  private void installNativeTypeDescriptors() {
    withDeclarationTransaction(
        () -> {
          namespace("std.native");
          for (hara.lang.declaration.HaraNativeBinding binding : HaraNativeDeclarations.bindings()) {
            publishNativeDescriptor(binding);
          }
          return null;
        });
  }

  /** Publishes one annotated native descriptor and all spec-defined aliases. */
  private void publishNativeDescriptor(hara.lang.declaration.HaraNativeBinding binding) {
    String name = binding.name();
    String canonicalName = HaraNativeDeclarations.namespace(name);
    if (sandboxRestricted && sandboxForbiddenNamespace(canonicalName)) return;
    HaraNamespace intrinsic = namespace(INTRINSIC_NAMESPACE);
    HaraVar descriptor =
        intrinsic.define(
            canonicalName,
            new HaraNativeType(
                name,
                HaraNativeDeclarations.methods(name),
                binding.availability(),
                binding.capability()),
            null,
            HaraVar.Origin.RUNTIME_PRIMITIVE);
    intrinsic.refer(name, descriptor);
    namespace(FOUNDATION_NAMESPACE).refer(name, descriptor);
  }

  private void installNativeExports(String sourceNamespace) {
    Map<String, BuiltinExport> exports = builtinCatalogs.getOrDefault(sourceNamespace, Map.of());
    if (exports.isEmpty()) return;
    if (FOUNDATION_NAMESPACE.equals(sourceNamespace)) {
      installNativeExportGroup("Maths", exports, HaraNativeDeclarations.methods("Maths"), Map.of());
      installNativeExportGroup("Num", exports, HaraNativeDeclarations.methods("Num"), Map.of());
      installNativeExportGroup(
          "Bits",
          exports,
          HaraNativeDeclarations.methods("Bits"),
          Map.of(
              "and", "bit-and",
              "or", "bit-or",
              "xor", "bit-xor",
              "not", "bit-not",
              "shift-left", "bit-shift-left",
              "shift-right", "bit-shift-right"));
      installNativeExportGroup("Crypto", exports, HaraNativeDeclarations.methods("Crypto"), Map.of());
      installNativeExportGroup(
          "Arr", exports, java.util.List.of("new"), Map.of("new", "array"));
      installNativeExportGroup(
          "Obj", exports, java.util.List.of("new"), Map.of("new", "object"));
      installNativeExportGroup(
          "Runtime",
          exports,
          java.util.List.of(
              "load-string", "macroexpand-1", "gensym", "ns-publics", "ns-aliases", "ns-find", "ns-create", "ns-name"),
          Map.of());
      installNativeExportGroup(
          "Printer", exports, HaraNativeDeclarations.methods("Printer"), Map.of());
      installNativeExportGroup(
          "RegExp",
          exports,
          HaraNativeDeclarations.methods("RegExp"),
          Map.of(
              "compile", "regexp",
              "pattern", "re-pattern",
              "find", "re-find",
              "matches", "re-matches",
              "replace", "re-replace",
              "split", "re-split"));
      installNativeExportGroup(
          "Exception",
          exports,
          HaraNativeDeclarations.methods("Exception"),
          Map.of(
              "new", "ex-info",
              "message", "ex-message"));
      namespace("std.native.Exception")
          .define(
              "class",
              new UnaryBuiltin(
                  "std.native.Exception/class", value -> portableType(value).getName()));
      installNativeExportGroup("Base", exports, HaraNativeDeclarations.methods("Base"), Map.of());
      installNativeExportGroup("Iter", exports, HaraNativeDeclarations.methods("Iter"), Map.of());
      return;
    }
    String type =
        switch (sourceNamespace) {
          case "std.foundation.string" -> "String";
          case "std.foundation.bytes" -> "Bytes";
          case "std.foundation.promise" -> "Promise";
          case "std.foundation.coroutine" -> "Coroutine";
          default -> null;
        };
    if (type != null) {
      installNativeExportGroup(type, exports, HaraNativeDeclarations.methods(type), Map.of());
    }
  }

  /**
   * Seeds only the callables required to evaluate the canonical Foundation
   * source itself. The HAL module adopts these values into source-owned Vars;
   * they are not the public Foundation implementation.
   */
  private void installFoundationBootstrapSeeds() {
    Map<String, BuiltinExport> seeds =
        builtinCatalogs.getOrDefault(FOUNDATION_NAMESPACE, Map.of());
    if (seeds.isEmpty()) throw new HaraException("Missing Foundation bootstrap callables");
    HaraNamespace foundation = namespace(FOUNDATION_NAMESPACE);
    for (Map.Entry<String, BuiltinExport> entry : seeds.entrySet()) {
      BuiltinExport seed = entry.getValue();
      foundation.define(
          entry.getKey(), seed.value, seed.metadata, HaraVar.Origin.RUNTIME_PRIMITIVE);
    }
    BuiltinExport pair = seeds.get("pair");
    if (pair == null) throw new HaraException("Missing Foundation pair bootstrap callable");
    namespace("global")
        .define("pair", pair.value, pair.metadata, HaraVar.Origin.RUNTIME_PRIMITIVE);
  }

  /** Keeps native iterator combinators available through Iter, not the portable root surface. */
  private void hideIteratorImplementationBindings() {
    HaraNamespace foundation = namespace(FOUNDATION_NAMESPACE);
    foundation.vars.keySet().removeIf(
        name ->
            name.startsWith("iter-")
                && !name.equals("iter-next?")
                && !name.equals("iter-next"));
  }

  /** Keeps runtime value/collection primitives available through Base only. */
  private void hideBaseImplementationBindings() {
    HaraNamespace foundation = namespace(FOUNDATION_NAMESPACE);
    foundation.vars.keySet().removeIf(name -> name.startsWith("__base-"));
  }

  private void installNativeExportGroup(
      String type,
      Map<String, BuiltinExport> exports,
      java.util.List<String> methods,
      Map<String, String> sourceNames) {
    String namespaceName = "std.native." + type;
    HaraNamespace target = namespace(namespaceName);
    for (String method : methods) {
      String sourceName = sourceNames.getOrDefault(method, method);
      BuiltinExport export = exports.get(sourceName);
      if (export != null) {
        target.define(method, export.value, export.metadata, HaraVar.Origin.RUNTIME_PRIMITIVE);
      }
    }
    namespaceStates.put(namespaceName, NamespaceLoadState.LOADED);
    namespaceFailures.remove(namespaceName);
  }

  private void installEnvironmentLibraries() {
    HaraNamespace runtime = namespace("std.native.Runtime");
    runtime.define(
        "var-sym",
        new UnaryBuiltin(
            "std.native.Runtime/var-sym",
            value -> {
              Object raw = HaraBox.unwrap(value);
              if (!(raw instanceof HaraVar variable)) {
                throw new HaraException("std.native.Runtime/var-sym expects a Var");
              }
              return Symbol.create(variable.namespaceName(), variable.symbolName());
            }));
    runtime.define(
        "current",
        new VariadicBuiltin(
            "std.native.Runtime/current",
            values -> {
              if (values.length != 0) {
                throw new HaraException("std.native.Runtime/current expects no arguments");
              }
              return Symbol.create(currentNamespace.name());
            }));
    runtime.define("snapshot", new VariadicBuiltin("std.native.Runtime/snapshot", this::environmentSnapshot));
    runtime.define("vars", new VariadicBuiltin("std.native.Runtime/vars", this::environmentVars));
    runtime.define("namespaces", new VariadicBuiltin("std.native.Runtime/namespaces", this::environmentNamespaces));
    runtime.define("namespace", new UnaryBuiltin("std.native.Runtime/namespace", this::environmentNamespace));
    runtime.define("module", new UnaryBuiltin("std.native.Runtime/module", this::environmentModule));
    runtime.define("alias-state", new VariadicBuiltin("std.native.Runtime/alias-state", this::namespaceAliasState));
    runtime.define("intern-var", new VariadicBuiltin("std.native.Runtime/intern-var", this::internVar));
    runtime.define("eval-in", new VariadicBuiltin("std.native.Runtime/eval-in", this::evalInNamespace));
    runtime.define("eval", new UnaryBuiltin("std.native.Runtime/eval", this::evalForm));
    runtime.define(
        "gensym",
        new VariadicBuiltin(
            "std.native.Runtime/gensym",
            values -> {
              if (values.length > 1) {
                throw new HaraException("gensym expects zero or one prefix");
              }
              return gensym(values.length == 0 ? null : String.valueOf(HaraBox.unwrap(values[0])));
            }));
    runtime.define(
        "macroexpand-1",
        new UnaryBuiltin("std.native.Runtime/macroexpand-1", value -> macroExpand(value, false)));
    runtime.define(
        "ns-publics",
        new UnaryBuiltin("std.native.Runtime/ns-publics", this::namespacePublics));
    runtime.define(
        "ns-aliases",
        new UnaryBuiltin("std.native.Runtime/ns-aliases", this::namespaceAliases));
    runtime.define("ns-find", new UnaryBuiltin("std.native.Runtime/ns-find", this::namespaceFind));
    runtime.define("ns-create", new UnaryBuiltin("std.native.Runtime/ns-create", this::namespaceCreate));
    runtime.define("ns-name", new UnaryBuiltin("std.native.Runtime/ns-name", this::namespaceName));
    namespaceStates.put("std.native.Runtime", NamespaceLoadState.LOADED);

    HaraNamespace base = namespace("std.native.Base");
    base.define(
        "apply",
        new VariadicBuiltin("std.native.Base/apply", this::applyFunction));
    base.define(
        "resolve",
        new UnaryBuiltin(
            "std.native.Base/resolve",
            value -> resolveAvailableValue(value, "std.native.Base/resolve")));
    base.define(
        "special-symbol?",
        new UnaryBuiltin(
            "std.native.Base/special-symbol?",
            value -> {
              Object raw = HaraBox.unwrap(value);
              return raw instanceof Symbol symbol && isSpecialSymbol(symbol);
            }));
    namespaceStates.put("std.native.Base", NamespaceLoadState.LOADED);

    HaraNamespace packages = namespace("std.native.Package");
    packages.define("catalog", new VariadicBuiltin("std.native.Package/catalog", values -> packageUnsupported("catalog", values, 0)));
    packages.define("find", new VariadicBuiltin("std.native.Package/find", values -> packageUnsupported("find", values, 1)));
    packages.define("ensure", new VariadicBuiltin("std.native.Package/ensure", values -> packageUnsupported("ensure", values, 1)));
    packages.define("load", new VariadicBuiltin("std.native.Package/load", values -> packageUnsupported("load", values, 1)));
    packages.define("unload", new VariadicBuiltin("std.native.Package/unload", values -> packageUnsupported("unload", values, values.length == 2 ? 2 : 1)));
    packages.define("state", new VariadicBuiltin("std.native.Package/state", values -> packageUnsupported("state", values, 1)));
    namespaceStates.put("std.native.Package", NamespaceLoadState.LOADED);
  }

  private Object packageUnsupported(String operation, Object[] values, int arity) {
    if (values.length != arity) {
      throw new HaraException("std.native.Package/" + operation + " expects " + arity + " arguments");
    }
    if (!nativeCapabilityBoundary.granted("kernel")) {
      throw HaraNativeCapabilityBoundary.denied("Package", operation, "kernel");
    }
    throw new HaraException("package/unsupported: Package capability provider is unavailable");
  }

  private void requireNativeCapability(String nativeType, String method, String capability) {
    nativeCapabilityBoundary.require(nativeType, method, capability);
  }

  private Object rejectedNativeCapabilityPromise(
      String nativeType, String method, String capability) {
    CompletableFuture<Object> future = new CompletableFuture<>();
    future.completeExceptionally(
        HaraNativeCapabilityBoundary.denied(nativeType, method, capability));
    return new HaraPromise(future);
  }

  private Object environmentSnapshot(Object[] values) {
    if (values.length != 0) throw new HaraException("std.native.Runtime/snapshot expects no arguments");
    return hara.lang.data.OrderedMap.Standard.from(
        null,
        Keyword.create("env/current"), Symbol.create(currentNamespace.name()),
        Keyword.create("env/namespaces"), environmentNamespaces(new Object[0]));
  }

  private Object environmentNamespaces(Object[] values) {
    if (values.length != 0) throw new HaraException("std.native.Runtime/namespaces expects no arguments");
    java.util.TreeSet<String> names = new java.util.TreeSet<>(namespaces.keySet());
    names.addAll(namespaceStates.keySet());
    ArrayList<Object> output = new ArrayList<>();
    for (String name : names) output.add(environmentNamespaceDescriptor(name));
    return hara.lang.data.Vector.Standard.from(null, output.toArray());
  }

  private Object environmentNamespace(Object value) {
    String name = namespaceIdentifier(value, "std.native.Runtime/namespace");
    if (!namespaces.containsKey(name) && !namespaceStates.containsKey(name)) return null;
    return environmentNamespaceDescriptor(name);
  }

  private Object environmentNamespaceDescriptor(String name) {
    NamespaceLoadState state = namespaceStates.get(name);
    if (state == null && namespaces.containsKey(name)) state = NamespaceLoadState.LOADED;
    String origin = name.startsWith("std.native") ? "embedded" : namespaces.containsKey(name) ? "runtime" : "registered";
    return hara.lang.data.OrderedMap.Standard.from(
        null,
        Keyword.create("namespace/name"), Symbol.create(name),
        Keyword.create("namespace/state"), Keyword.create(state == null ? "unknown" : state.keyword),
        Keyword.create("namespace/role"), Keyword.create(
            namespaces.containsKey(name) ? namespaces.get(name).role : "standard"),
        Keyword.create("namespace/revision"), modules.values().stream()
            .filter(module -> name.equals(module.namespace))
            .mapToLong(module -> module.revision)
            .max()
            .orElse(0L),
        Keyword.create("namespace/origin"), Keyword.create(origin));
  }

  private Object environmentModule(Object value) {
    Object raw = HaraBox.unwrap(value);
    if (!(raw instanceof String requested)) {
      throw new HaraException("std.native.Runtime/module expects a path string");
    }
    String key =
        requested.startsWith("classpath:")
            ? requested
            : (getResource(requested) == null
                ? canonicalPath(requested)
                : "classpath:" + requested);
    ModuleRecord module = modules.get(key);
    if (module == null) return null;
    Set<String> dependencies = moduleDependencies.getOrDefault(key, Set.of());
    return hara.lang.data.OrderedMap.Standard.from(
        null,
        Keyword.create("module/path"), module.path,
        Keyword.create("module/namespace"), Symbol.create(module.namespace),
        Keyword.create("module/revision"), module.revision,
        Keyword.create("module/dependencies"),
            BuiltinStruct.vector(new LinkedHashSet<>(dependencies).toArray()));
  }

  private Object environmentVars(Object[] values) {
    if (values.length > 1) throw new HaraException("std.native.Runtime/vars expects zero or one namespace");
    String name = values.length == 0 ? currentNamespace.name() : namespaceIdentifier(values[0], "std.native.Runtime/vars");
    HaraNamespace target = namespaces.get(name);
    if (target == null) throw new HaraException("namespace/not-found: " + name);
    ArrayList<Object> entries = new ArrayList<>();
    for (String symbol : target.sortedSymbolNames()) {
      HaraVar variable = target.lookup(symbol);
      if (name.equals(variable.namespaceName())) {
        entries.add(Symbol.create(symbol));
        entries.add(variable);
      }
    }
    return hara.lang.data.OrderedMap.Standard.from(null, entries.toArray());
  }

  private Object resolveAvailableValue(Object value, String operation) {
    Object raw = HaraBox.unwrap(value);
    if (!(raw instanceof Symbol symbol)) {
      throw new HaraException(operation + " expects a symbol");
    }
    return resolveAvailable(symbol);
  }

  /** Resolves only Vars already materialized in the registry; this never invokes the loader. */
  @TruffleBoundary
  private HaraVar resolveAvailable(Symbol symbol) {
    if (sandboxRestricted && sandboxForbidden(symbol)) return null;
    String namespaceName = symbol.getNamespace();
    if (namespaceName != null) {
      if ("-".equals(namespaceName)) namespaceName = currentNamespace.name();
      namespaceName =
          aliases
              .getOrDefault(currentNamespace.name(), Map.of())
              .getOrDefault(namespaceName, namespaceName);
      if (sandboxRestricted && sandboxForbiddenNamespace(namespaceName)) return null;
      HaraNamespace target = namespaces.get(namespaceName);
      return target == null ? null : target.lookup(symbol.getName());
    }
    HaraVar variable = currentNamespace.lookup(symbol.getName());
    if (variable == null) {
      String canonical = globalImports.get(symbol.getName());
      if (canonical != null && !canonical.equals(symbol.display())) {
        variable = resolveAvailable(Symbol.create(canonical));
      }
    }
    if (variable == null && symbol.getName().startsWith(PROTOCOL_NAMESPACE_PREFIX)) {
      HaraNamespace protocolNamespace = namespaces.get(symbol.getName());
      if (protocolNamespace != null) {
        String protocolName =
            symbol.getName().substring(symbol.getName().lastIndexOf('.') + 1);
        variable = protocolNamespace.lookup(protocolName);
      }
    }
    if (variable != null
        && sandboxRestricted
        && sandboxForbiddenNamespace(variable.namespaceName())) {
      return null;
    }
    return variable;
  }

  private void registerBuiltin(
      String namespaceName,
      String symbolName,
      Object value,
      IMetadata metadata,
      HaraVar.Origin origin) {
    Map<String, BuiltinExport> catalog =
        builtinCatalogs.computeIfAbsent(namespaceName, ignored -> new LinkedHashMap<>());
    BuiltinExport previous =
        catalog.putIfAbsent(
            symbolName, new BuiltinExport(namespaceName, symbolName, value, metadata, origin));
    if (previous != null) {
      throw new HaraException("Duplicate builtin export: " + namespaceName + "/" + symbolName);
    }
  }

  void defineLibraryFunction(
      String namespaceName,
      String symbolName,
      Function<Object[], Object> implementation,
      IMetadata metadata) {
    namespace(namespaceName)
        .define(
            symbolName,
            new VariadicBuiltin(namespaceName + "/" + symbolName, implementation),
            metadata,
            HaraVar.Origin.JAVA_LIBRARY);
  }

  void defineNativeFunction(
      String namespaceName,
      String symbolName,
      Function<Object[], Object> implementation,
      IMetadata metadata) {
    namespace(namespaceName)
        .define(
            symbolName,
            new VariadicBuiltin(namespaceName + "/" + symbolName, implementation),
            metadata,
            HaraVar.Origin.RUNTIME_PRIMITIVE);
  }

  void defineLibraryValue(
      String namespaceName, String symbolName, Object value, IMetadata metadata) {
    namespace(namespaceName)
        .define(symbolName, value, metadata, HaraVar.Origin.JAVA_LIBRARY);
  }

  void defineLibraryMacro(
      String namespaceName,
      String symbolName,
      Function<List<?>, Object> expander,
      IMetadata metadata,
      boolean intrinsic) {
    HaraMacro macro = HaraMacro.nativeMacro(Symbol.create(symbolName), expander);
    namespace(namespaceName)
        .define(symbolName, macro, metadata, HaraVar.Origin.JAVA_LIBRARY);
    macros
        .computeIfAbsent(namespaceName, ignored -> new ConcurrentHashMap<>())
        .put(symbolName, macro);
    if (intrinsic) defineIntrinsicMacro(Symbol.create(symbolName), macro);
  }

  Object libraryFunction(String name, Function<Object[], Object> implementation) {
    return new VariadicBuiltin(name, implementation);
  }

  public String currentNamespaceName() {
    return currentNamespace.name();
  }

  public Symbol gensym(String prefix) {
    String base = prefix == null || prefix.isEmpty() ? "G__" : prefix + "__";
    return Symbol.create(base + gensymCounter.incrementAndGet());
  }

  Object macroAliases() {
    ArrayList<Object> entries = new ArrayList<>();
    for (Map.Entry<String, String> entry :
        aliases.getOrDefault(currentNamespace.name(), Map.of()).entrySet()) {
      entries.add(Symbol.create(entry.getKey()));
      entries.add(Symbol.create(entry.getValue()));
    }
    return hara.lang.data.Map.Standard.from(null, entries.toArray());
  }

  void runInNamespace(String namespaceName, Runnable operation) {
    HaraNamespace previous = currentNamespace;
    try {
      currentNamespace = namespace(namespaceName);
      operation.run();
    } finally {
      currentNamespace = previous;
    }
  }

  public <T> T callInNamespace(String namespaceName, java.util.function.Supplier<T> operation) {
    HaraNamespace previous = currentNamespace;
    try {
      currentNamespace = namespace(namespaceName);
      return operation.get();
    } finally {
      currentNamespace = previous;
    }
  }

  @SuppressWarnings("unchecked")
  <T> T libraryState(String name, Supplier<T> factory) {
    return (T) libraryStates.computeIfAbsent(name, ignored -> factory.get());
  }

  @TruffleBoundary
  public void setCurrentNamespace(Symbol symbol) {
    setCurrentNamespace(symbol, new Object[0]);
  }

  @TruffleBoundary
  public void setCurrentNamespace(Symbol symbol, Object[] clauses) {
    HaraNamespaceDeclaration declaration = HaraNamespaceDeclaration.parse(symbol, clauses);
    ContextSnapshot snapshot = snapshot();
    try {
      applyNamespaceDeclaration(declaration);
    } catch (RuntimeException error) {
      restore(snapshot);
      throw error;
    }
  }

  @TruffleBoundary
  void prepareCurrentNamespace(Symbol symbol, Object[] clauses) {
    boolean previous = preparingNamespace;
    preparingNamespace = true;
    try {
      setCurrentNamespace(symbol, clauses);
    } finally {
      preparingNamespace = previous;
    }
  }

  private void applyNamespaceDeclaration(HaraNamespaceDeclaration declaration) {
    currentNamespace = namespace(declaration.name.getName());
    registerGlobalAlias(declaration.globalAlias, currentNamespace.name());
    for (String imported : declaration.globalImports) {
      registerGlobalImport(imported);
    }
    currentNamespace.role = declaration.role;
    if (declaration.blank) {
      blankNamespaces.add(currentNamespace.name());
      currentNamespace.removeReferredVars();
    } else {
      blankNamespaces.remove(currentNamespace.name());
    }
    if (!declaration.blank) referRuntimeIntrinsics(currentNamespace);
    referNativeTypeDescriptors(currentNamespace);
    configureNativeAliases(currentNamespace);
    if (!declaration.blank) referFoundation(currentNamespace);
    configureProtocolAliases(currentNamespace);
    configureFoundationAliases(declaration);
    configureNativeFlavor(declaration.structuralClauses);
    applyNamespaceRequires(declaration.structuralClauses);
    applyNamespaceUses(declaration.structuralClauses);
    configureGlobalAliases(declaration);
    configureGlobalImports();
    if (declaration.selectiveFoundation) {
      for (String name : namespace(FOUNDATION_NAMESPACE).vars.keySet()) {
        if (!declaration.exposedFoundation.contains(name)) removeFoundationRefer(name);
      }
    } else {
      for (String name : declaration.excludedFoundation) removeFoundationRefer(name);
    }
  }

  private void removeFoundationRefer(String name) {
    currentNamespace.removeReferredVar(name);
    Map<String, HaraMacro> namespaceMacros = macros.get(currentNamespace.name());
    HaraMacro foundationMacro = macros.getOrDefault(FOUNDATION_NAMESPACE, Map.of()).get(name);
    if (namespaceMacros != null && namespaceMacros.get(name) == foundationMacro) {
      namespaceMacros.remove(name);
    }
  }

  private void configureFoundationAliases(HaraNamespaceDeclaration declaration) {
    Map<String, String> namespaceAliases =
        aliases.computeIfAbsent(currentNamespace.name(), ignored -> new ConcurrentHashMap<>());
    namespaceAliases
        .entrySet()
        .removeIf(entry -> HaraBuiltinCatalog.GENERATED_LIBRARIES.containsValue(entry.getValue()));
    namespaceAliases.keySet().removeAll(globalAliases.keySet());
    for (Map.Entry<String, String> library : HaraBuiltinCatalog.GENERATED_LIBRARIES.entrySet()) {
      if (declaration.excludedFoundationLibraries.contains(library.getKey())) continue;
      String alias =
          declaration.foundationAliases.getOrDefault(
              library.getKey(), HaraBuiltinCatalog.DEFAULT_LIBRARY_ALIASES.get(library.getKey()));
      putAlias(namespaceAliases, alias, library.getValue());
    }
  }

  private void configureGlobalAliases(HaraNamespaceDeclaration declaration) {
    Map<String, String> namespaceAliases =
        aliases.computeIfAbsent(currentNamespace.name(), ignored -> new ConcurrentHashMap<>());
    for (Map.Entry<String, String> global : globalAliases.entrySet()) {
      String library = global.getValue().startsWith("std.foundation.")
          ? global.getValue().substring("std.foundation.".length())
          : null;
      if (library != null && declaration.excludedFoundationLibraries.contains(library)) continue;
      if (!currentNamespace.name().equals(global.getValue())) {
        namespaceAliases.putIfAbsent(global.getKey(), global.getValue());
      }
    }
  }

  private void registerGlobalAlias(String alias, String namespace) {
    if (alias == null) return;
    String previous = globalAliases.putIfAbsent(alias, namespace);
    if (previous != null && !previous.equals(namespace)) {
      throw new HaraException(
          "Global namespace alias already refers to " + previous + ": " + alias);
    }
  }

  private void registerGlobalImport(String shorthand) {
    String canonical = canonicalGlobalImport(shorthand);
    int separator = shorthand.lastIndexOf('/');
    String local = separator < 0 ? shorthand : shorthand.substring(separator + 1);
    String previous = globalImports.putIfAbsent(local, canonical);
    if (previous != null && !previous.equals(canonical)) {
      throw new HaraException(
          "Global import already refers to " + previous + ": " + local);
    }
  }

  private String canonicalGlobalImport(String shorthand) {
    int separator = shorthand.lastIndexOf('/');
    if (separator <= 0 || separator == shorthand.length() - 1) return shorthand;
    String protocolName = shorthand.substring(0, separator);
    if (!protocolDeclarations.containsKey(protocolName)) return shorthand;
    return builtinProtocolNamespace(protocolName) + shorthand.substring(separator);
  }

  private void configureGlobalImports() {
    for (Map.Entry<String, String> entry : globalImports.entrySet()) {
      if (currentNamespace.lookup(entry.getKey()) != null) continue;
      HaraVar imported = resolve(Symbol.create(entry.getValue()));
      if (imported != null) currentNamespace.refer(entry.getKey(), imported);
    }
  }

  private void configureProtocolAliases(HaraNamespace target) {
    Map<String, String> namespaceAliases =
        aliases.computeIfAbsent(target.name(), ignored -> new ConcurrentHashMap<>());
    for (String name : protocolDeclarations.keySet()) {
      putAlias(namespaceAliases, name, builtinProtocolNamespace(name));
    }
  }

  private void configureNativeAliases(HaraNamespace target) {
    Map<String, String> namespaceAliases =
        aliases.computeIfAbsent(target.name(), ignored -> new ConcurrentHashMap<>());
    for (hara.lang.declaration.HaraNativeBinding binding : HaraNativeDeclarations.bindings()) {
      String name = binding.name();
      String namespace = HaraNativeDeclarations.namespace(name);
      if (!sandboxRestricted || !sandboxForbiddenNamespace(namespace)) {
        putAlias(namespaceAliases, name, namespace);
      }
    }
    if (sandboxRestricted) {
      // ns-find is the one read-only Runtime operation exposed by the
      // Foundation facade in a sandbox. Keep its ordinary Runtime alias so
      // source-owned ns-find remains executable without exposing other native
      // Runtime methods.
      putAlias(namespaceAliases, "Runtime", "std.native.Runtime");
    }
  }

  private void referNativeTypeDescriptors(HaraNamespace target) {
    HaraNamespace intrinsic = namespace(INTRINSIC_NAMESPACE);
    for (hara.lang.declaration.HaraNativeBinding binding : HaraNativeDeclarations.bindings()) {
      String name = binding.name();
      String namespace = HaraNativeDeclarations.namespace(name);
      if (sandboxRestricted && sandboxForbiddenNamespace(namespace)) continue;
      HaraVar descriptor = intrinsic.lookup(name);
      if (descriptor == null) continue;
      if (target.lookup(name) == null) target.refer(name, descriptor);
      if (target.lookup(namespace) == null) target.refer(namespace, descriptor);
    }
  }

  private void applyNamespaceRequires(Object[] clauses) {
    Map<String, String> namespaceAliases =
        aliases.computeIfAbsent(currentNamespace.name(), ignored -> new ConcurrentHashMap<>());
    for (Object clauseValue : clauses) {
      List<?> clause = (List<?>) clauseValue;
      if (!(clause.nth(0) instanceof Keyword keyword)
          || !"require".equals(keyword.getName())) {
        continue;
      }
      for (int index = 1; index < clause.count(); index++) {
        applyGeneratedRequire(clause.nth(index), namespaceAliases);
      }
    }
  }

  private void applyNamespaceUses(Object[] clauses) {
    for (Object clauseValue : clauses) {
      List<?> clause = (List<?>) clauseValue;
      if (!(clause.nth(0) instanceof Keyword keyword)
          || !"use".equals(keyword.getName())) {
        continue;
      }
      for (int index = 1; index < clause.count(); index++) {
        Object targetValue = clause.nth(index);
        if (!(targetValue instanceof Symbol target)
            || target.getNamespace() != null) {
          throw new HaraException(":use expects unqualified namespace symbols");
        }
        if (requiredNamespace(target.getName()) == null) {
          throw new HaraException("Cannot use missing namespace: " + target.getName());
        }
        referNamespace(target.getName());
      }
    }
  }

  private void referMacro(String target, String name) {
    HaraMacro macro = macros.getOrDefault(target, Map.of()).get(name);
    if (macro != null) {
      macros
          .computeIfAbsent(currentNamespace.name(), ignored -> new ConcurrentHashMap<>())
          .put(name, macro);
    }
  }

  private void configureNativeFlavor(Object[] clauses) {
    List<?> flavorClause = null;
    for (Object clauseValue : clauses) {
      if (!(clauseValue instanceof List<?>)) continue;
      List<?> clause = (List<?>) clauseValue;
      if (clause.count() > 0
          && clause.nth(0) instanceof Keyword
          && "flavor".equals(((Keyword) clause.nth(0)).getName())) {
        if (flavorClause != null) throw new HaraException("ns accepts only one :flavor clause");
        flavorClause = clause;
      }
    }
    if (flavorClause != null) {
      if (flavorClause.count() < 2 || !(flavorClause.nth(1) instanceof Keyword)) {
        throw new HaraException(":flavor expects a host keyword followed by import specs");
      }
      Keyword flavor = (Keyword) flavorClause.nth(1);
      if (flavor.getNamespace() != null) {
        throw new HaraException(":flavor expects an unqualified keyword");
      }
      if ("wasm".equals(flavor.getName())) {
        throw new HaraException(":wasm is not a host flavor; use :import for Wasm modules");
      }
      if ("jvm".equals(flavor.getName())) activateJvmFlavor();
      NativeFlavorProvider provider = nativeFlavorRegistry.require(flavor.getName());
      java.util.ArrayList<Object> specifications = new java.util.ArrayList<>();
      for (int i = 2; i < flavorClause.count(); i++) {
        specifications.add(flavorClause.nth(i));
      }
      java.util.List<NativeFlavorImportSpecs.Spec> imports;
      try {
        imports = NativeFlavorImportSpecs.parse(specifications);
      } catch (IllegalArgumentException error) {
        throw new HaraException(error.getMessage());
      }
      Map<String, Object> resolved = new LinkedHashMap<>();
      for (NativeFlavorImportSpecs.Spec specification : imports) {
        importNativeType(provider, resolved, specification);
      }
      nativeFlavors.put(currentNamespace.name(), flavor.getName());
      nativeImports.put(currentNamespace.name(), new ConcurrentHashMap<>(resolved));
    }

    for (Object clauseValue : clauses) {
      if (!(clauseValue instanceof List<?>)) continue;
      List<?> clause = (List<?>) clauseValue;
      if (clause.count() == 0
          || !(clause.nth(0) instanceof Keyword)
          || !"import".equals(((Keyword) clause.nth(0)).getName())) continue;
      Map<String, String> namespaceAliases =
          aliases.computeIfAbsent(currentNamespace.name(), ignored -> new ConcurrentHashMap<>());
      for (int i = 1; i < clause.count(); i++) {
        Object spec = clause.nth(i);
        if (spec instanceof Symbol) {
          importWasmNamespace(namespaceAliases, ((Symbol) spec).display(), ((Symbol) spec).display());
        } else if (spec instanceof ILinearType<?>) {
          ILinearType<?> group = (ILinearType<?>) spec;
          if (group.count() == 0) continue;
          Object packageValue = group.nth(0);
          String packageName =
              packageValue instanceof Symbol
                  ? ((Symbol) packageValue).display()
                  : String.valueOf(packageValue);
          for (int j = 1; j < group.count(); j++) {
            Object classValue = group.nth(j);
            String className =
                classValue instanceof Symbol
                    ? ((Symbol) classValue).display()
                    : String.valueOf(classValue);
            importWasmNamespace(namespaceAliases, className, packageName + "." + className);
          }
        } else {
          throw new HaraException(":import expects Wasm module symbols or package vectors");
        }
      }
    }
  }

  private void importWasmNamespace(
      Map<String, String> namespaceAliases, String localName, String moduleName) {
    HaraNamespace loaded = namespaces.get(moduleName);
    if (loaded == null) {
      HaraExtensionPackage packageExtension = extensionRegistry.discoverWasmImport(moduleName);
      if (packageExtension != null) loaded = installExtension(packageExtension);
    }
    if (loaded == null) loaded = requiredNamespace(moduleName);
    if (loaded == null) {
      throw new HaraException("native/import-missing: " + moduleName);
    }
    HaraExtensionRuntime extension = loadedExtensions.get(moduleName);
    if (!(extension instanceof HaraWasmExtension)) {
      throw new HaraException("native/import-not-wasm: " + moduleName);
    }
    if (!((HaraWasmExtension) extension).supportsDirectImport()) {
      throw new HaraException("native/import-abi-unsupported: " + moduleName + " requires core.v1");
    }
    putAlias(namespaceAliases, localName, moduleName);
  }

  private void importNativeType(
      NativeFlavorProvider provider,
      Map<String, Object> imports,
      NativeFlavorImportSpecs.Spec specification) {
    Object type = provider.resolveType(specification.typeName(), nativeAccess());
    Object previous = imports.putIfAbsent(specification.localName(), type);
    if (previous != null && previous != type) {
      throw new HaraException("Native import already exists: " + specification.localName());
    }
  }

  private void activateJvmFlavor() {
    HaraExtensionRegistry.JvmFlavorPackage candidate = extensionRegistry.discoverJvmFlavor();
    if (candidate == null) return;
    HaraPackageManifest.JvmFlavor flavor = candidate.manifest().jvmFlavor();
    if (!flavor.hostCalls().isEmpty()) {
      throw new HaraException("package/host-call-denied: JVM flavors cannot request host calls");
    }
    if (!flavor.requiredCapabilities().isEmpty()) {
      HaraProject currentProject = project();
      if (currentProject == null) {
        throw new HaraException(
            "package/capability-denied: JVM flavor requires project capabilities");
      }
      for (String capability : flavor.requiredCapabilities()) {
        if (!currentProject.hasCapability(capability)) {
          throw new HaraException("package/capability-denied: " + capability);
        }
      }
    }
    String key = candidate.manifest().identity() + "@" + candidate.manifest().version();
    JvmPackageLoader.LoadedArtifact loaded = loadedJvmFlavors.get(key);
    if (loaded == null) {
      Path artifact = candidate.manifest().verifyJvmFlavor(candidate.root());
      JvmPackageLoader.FlavorSelection selection =
          new JvmPackageLoader.FlavorSelection(
              candidate.manifest().identity(),
              artifact,
              flavor.sha256(),
              flavor.target(),
              flavor.abi(),
              flavor.entryPoint(),
              candidate.manifest().jvmDependencyFiles(candidate.root()));
      loaded = JvmPackageLoader.loadFlavor(selection);
      JvmPackageLoader.LoadedArtifact previous = loadedJvmFlavors.putIfAbsent(key, loaded);
      if (previous != null) {
        try {
          loaded.close();
        } catch (IOException error) {
          throw new HaraException("package/JVM-loader-close-failed: " + error.getMessage());
        }
        loaded = previous;
      }
    }
    nativeFlavorLoader = loaded.classLoader();
  }

  private void initializeUserNamespace(HaraNamespace target) {
    referRuntimeIntrinsics(target);
    configureNativeAliases(target);
    referFoundation(target);
    configureProtocolAliases(target);
    Map<String, String> namespaceAliases =
        aliases.computeIfAbsent(target.name(), ignored -> new ConcurrentHashMap<>());
    for (Map.Entry<String, String> entry : HaraBuiltinCatalog.DEFAULT_LIBRARY_ALIASES.entrySet()) {
      namespaceAliases.putIfAbsent(
          entry.getValue(), HaraBuiltinCatalog.GENERATED_LIBRARIES.get(entry.getKey()));
    }
  }

  private void referRuntimeIntrinsics(HaraNamespace target) {
    HaraNamespace intrinsic = namespace(INTRINSIC_NAMESPACE);
    for (Map.Entry<String, HaraVar> entry : intrinsic.vars.entrySet()) {
      if (target.lookup(entry.getKey()) == null) target.refer(entry.getKey(), entry.getValue());
    }
  }

  private void referFoundation(HaraNamespace target) {
    // Native namespaces expose only their catalogued runtime surface. They must
    // not inherit Foundation Vars, or removed native names reappear as aliases.
    if (target.name().startsWith("std.native.")) return;
    HaraNamespace core = namespace(FOUNDATION_NAMESPACE);
    for (Map.Entry<String, HaraVar> entry : core.vars.entrySet()) {
      if (target.lookup(entry.getKey()) == null) target.refer(entry.getKey(), entry.getValue());
    }
  }

  private void putAlias(Map<String, String> namespaceAliases, String alias, String target) {
    if ("-".equals(alias)) throw new HaraException("Namespace alias is reserved: -");
    String previous = namespaceAliases.putIfAbsent(alias, target);
    if (previous != null && !previous.equals(target)) {
      throw new HaraException("Namespace alias already refers to " + previous + ": " + alias);
    }
  }

  private void applyGeneratedRequire(Object specValue, Map<String, String> namespaceAliases) {
    if (!(specValue instanceof ILinearType<?>)) {
      throw new HaraException(":require expects vectors such as [std.foundation.string :as str]");
    }
    ILinearType<?> spec = (ILinearType<?>) specValue;
    if (spec.count() == 0 || !(spec.nth(0) instanceof Symbol)) {
      throw new HaraException(":require namespace must be a symbol");
    }
    String target = ((Symbol) spec.nth(0)).display();
    boolean lazy = false;
    boolean reload = false;
    for (int i = 1; i < spec.count(); i += 2) {
      if (i + 1 >= spec.count() || !(spec.nth(i) instanceof Keyword)) {
        throw new HaraException("Malformed :require options for " + target);
      }
      if ("lazy".equals(((Keyword) spec.nth(i)).getName())) {
        if (!Boolean.TRUE.equals(spec.nth(i + 1))) {
          throw new HaraException(":require :lazy expects true");
        }
        lazy = true;
      } else if ("reload".equals(((Keyword) spec.nth(i)).getName())) {
        if (!Boolean.TRUE.equals(spec.nth(i + 1))) {
          throw new HaraException(":require :reload expects true");
        }
        reload = true;
      }
    }
    if (lazy) {
      boolean hasAlias = false;
      for (int i = 1; i < spec.count(); i += 2) {
        String option = ((Keyword) spec.nth(i)).getName();
        if ("as".equals(option)) hasAlias = true;
        if ("refer".equals(option) || "refer-macros".equals(option)) {
          throw new HaraException(":require :lazy cannot be combined with :" + option);
        }
      }
      if (!hasAlias) throw new HaraException(":require :lazy requires :as");
    }
    if (lazy && !namespaces.containsKey(target)) {
      namespaceStates.putIfAbsent(target, NamespaceLoadState.UNLOADED);
    }
    String reloadKey = currentNamespace.name() + "\u0000" + target;
    boolean executeReload = reload;
    if (reload && preparingNamespace) {
      preparedNamespaceReloads.add(reloadKey);
    } else if (reload && preparedNamespaceReloads.remove(reloadKey)) {
      executeReload = false;
    }
    HaraNamespace required =
        lazy && !executeReload ? null : requiredNamespace(target, executeReload);
    if ((!lazy || reload) && required == null) {
      throw new HaraException("Cannot require missing namespace: " + target);
    }
    java.util.Set<String> excludedRefers = new java.util.HashSet<>();
    for (int i = 1; i < spec.count(); i += 2) {
      if (i + 1 >= spec.count() || !(spec.nth(i) instanceof Keyword)) {
        throw new HaraException("Malformed :require options for " + target);
      }
      if ("exclude".equals(((Keyword) spec.nth(i)).getName())) {
        Object excluded = spec.nth(i + 1);
        if (!(excluded instanceof ILinearType<?>)) {
          throw new HaraException(":require :exclude expects a vector of symbols");
        }
        for (Object value : (ILinearType<?>) excluded) {
          if (!(value instanceof Symbol) || ((Symbol) value).getNamespace() != null) {
            throw new HaraException(":require :exclude expects unqualified symbols");
          }
          excludedRefers.add(((Symbol) value).getName());
        }
      }
    }
    for (String excluded : excludedRefers) {
      currentNamespace.removeReferredVar(excluded, target);
    }
    for (int i = 1; i < spec.count(); i += 2) {
      if (i + 1 >= spec.count() || !(spec.nth(i) instanceof Keyword)) {
        throw new HaraException("Malformed :require options for " + target);
      }
      String option = ((Keyword) spec.nth(i)).getName();
      Object value = spec.nth(i + 1);
      if ("as".equals(option)) {
        if (!(value instanceof Symbol) || ((Symbol) value).getNamespace() != null) {
          throw new HaraException(":require :as expects an unqualified symbol");
        }
        putAlias(namespaceAliases, ((Symbol) value).getName(), target);
      } else if ("lazy".equals(option)) {
        // Validated above; aliases retain the target name until first resolution.
      } else if ("reload".equals(option)) {
        // Validated and executed before namespace bindings are published.
      } else if ("access".equals(option)) {
        if (!Boolean.TRUE.equals(value)) {
          throw new HaraException(":require :access expects true");
        }
      } else if ("refer".equals(option)) {
        if (value instanceof Keyword && "all".equals(((Keyword) value).getName())) {
          for (String referred : required.symbolNames()) {
            if (!excludedRefers.contains(referred)) {
              currentNamespace.refer(referred, required.lookup(referred));
              referMacro(target, referred);
            }
          }
        } else {
          if (!(value instanceof ILinearType<?>)) {
            throw new HaraException(":require :refer expects a vector of symbols or :all");
          }
          for (Object referred : (ILinearType<?>) value) {
            if (!(referred instanceof Symbol) || ((Symbol) referred).getNamespace() != null) {
              throw new HaraException(":require :refer expects unqualified symbols");
            }
            String name = ((Symbol) referred).getName();
            if (excludedRefers.contains(name)) continue;
            HaraVar variable = required.lookup(name);
            if (variable == null) {
              throw new HaraException("Cannot refer missing var " + name + " from " + target);
            }
            currentNamespace.refer(name, variable);
            referMacro(target, name);
          }
        }
      } else if ("refer-macros".equals(option)) {
        if (!(value instanceof ILinearType<?>)) {
          throw new HaraException(":require :refer-macros expects a vector of symbols");
        }
        Map<String, HaraMacro> targetMacros = macros.get(target);
        for (Object referred : (ILinearType<?>) value) {
          if (!(referred instanceof Symbol) || ((Symbol) referred).getNamespace() != null) {
            throw new HaraException(":require :refer-macros expects unqualified symbols");
          }
          String name = ((Symbol) referred).getName();
          HaraMacro macro = targetMacros == null ? null : targetMacros.get(name);
          if (macro == null) {
            throw new HaraException("Cannot refer missing macro " + name + " from " + target);
          }
          macros
              .computeIfAbsent(
                  currentNamespace.name(), ignored -> new ConcurrentHashMap<>())
              .put(name, macro);
        }
      } else if ("exclude".equals(option)) {
        // Applied while processing :refer above. Keeping it as an explicit
        // option here prevents the validation pass from rejecting the
        // namespace declaration after it has already collected exclusions.
      } else {
        throw new HaraException("Unsupported :require option: :" + option);
      }
    }
  }

  private synchronized HaraNamespace requiredNamespace(String target) {
    if (sandboxRestricted && sandboxForbiddenNamespace(target)) return null;
    Path projectSource = resolveProjectSource(target);
    HaraNamespace existing = namespaces.get(target);
    if (existing != null
        && namespaceStates.get(target) == NamespaceLoadState.LOADED
        && (projectSource == null && bytecodeLibrary.provides(target)
            || !libraryLoader.provides(target)
            || sourceNamespaceLoaded(target))) {
      return existing;
    }
    NamespaceLoadState state = namespaceStates.get(target);
    if (state == NamespaceLoadState.LOADING
        && target.equals(currentNamespace.name())
        && existing != null) {
      return existing;
    }
    if (state == NamespaceLoadState.LOADING) {
      throw new HaraException("Cyclic namespace require: " + target);
    }
    if (state == NamespaceLoadState.FAILED) {
      String detail = namespaceFailures.get(target);
      throw new HaraException(
          "Namespace load previously failed; use explicit reload to retry: "
              + target
              + (detail == null ? "" : " (initial failure: " + detail + ")"));
    }

    ContextSnapshot snapshot = snapshot();
    try {
      ensureFoundationRoot(target);
      namespaceStates.put(target, NamespaceLoadState.LOADING);
      HaraNamespace loaded = null;
      if (projectSource != null) {
        requireResolvedSource(projectSource.toString(), false);
        loaded = loadedSourceNamespace(target, projectSource.toString());
      }
      if (loaded == null) loaded = loadBytecodeNamespace(target);
      if (loaded == null) libraryLoader.ensure(this, target);
      if (loaded == null && libraryLoader.provides(target)) {
        loaded = loadLibraryResource(target, false);
      }
      if (loaded == null
          && (libraryLoader.provides(target) || sourceNamespaceLoaded(target))) {
        loaded = namespaces.get(target);
      }
      if (loaded == null) loaded = requireSourceNamespace(target);
      if (loaded == null) {
        java.util.List<Path> extensionRoots = java.util.List.of();
        if (environment.isFileIOAllowed()) {
          HaraProject currentProject = project();
          extensionRoots =
              currentProject == null ? java.util.List.of() : currentProject.extensionRoots();
        }
        HaraExtensionPackage extensionPackage = extensionRegistry.discover(target, extensionRoots);
        if (extensionPackage != null) loaded = installExtension(extensionPackage);
      }
      if (loaded == null) {
        restore(snapshot);
        namespaceStates.put(target, NamespaceLoadState.FAILED);
        namespaceFailures.put(
            target, "no library, source, or extension provided this namespace");
        return null;
      }
      namespaceStates.put(target, NamespaceLoadState.LOADED);
      namespaceFailures.remove(target);
      return loaded;
    } catch (RuntimeException failure) {
      restore(snapshot);
      namespaceStates.put(target, NamespaceLoadState.FAILED);
      namespaceFailures.put(
          target,
          failure.getMessage() == null ? failure.getClass().getSimpleName() : failure.getMessage());
      throw failure;
    }
  }

  private HaraNamespace loadBytecodeNamespace(String target) {
    HbxBundleLibrary.Module module = bytecodeLibrary.module(target);
    if (module == null) return null;
    boolean trace = Boolean.getBoolean("hara.hbx.trace");
    if (trace) System.err.println("HBX0 load start " + target);
    String previousNamespace = currentNamespace.name();
    HaraVar.Origin previousOrigin = definitionOrigin;
    try {
      for (String dependency : module.descriptor().dependencies()) {
        if (!dependency.equals(target) && requiredNamespace(dependency) == null) {
          throw new HaraException(
              "Cannot require HBX0 dependency " + dependency + " for " + target);
        }
      }
      // The embedded HBX is the compiled representation of the portable HAL
      // fallback library. Host primitives keep ownership of same-named Vars;
      // application HBC evaluated through the public artifact API still uses
      // BYTECODE origin and can define ordinary program Vars.
      definitionOrigin = HaraVar.Origin.HAL_FALLBACK;
      currentNamespace = namespace(module.namespace());
      parseAndExecute(module.descriptor().namespaceForm(), module.descriptor().resource() + "#ns");
      HbcProgram program = module.program();
      currentNamespace = namespace(module.namespace());
      installHbcTypes(
          program.schemaTypes(), program.functionTypes(), program.inferredFunctionTypes());
      HbcMachine.execute(program, this);
      if (FOUNDATION_NAMESPACE.equals(target)) captureSequenceIntrinsics();
      if (trace) System.err.println("HBX0 load done " + target);
      return namespaces.get(target);
    } finally {
      definitionOrigin = previousOrigin;
      currentNamespace = namespace(previousNamespace);
    }
  }

  private synchronized HaraNamespace requiredNamespace(String target, boolean reload) {
    if (sandboxRestricted && sandboxForbiddenNamespace(target)) return null;
    if (!reload) return requiredNamespace(target);
    NamespaceLoadState previousState = namespaceStates.get(target);
    ContextSnapshot snapshot = snapshot();
    namespaceStates.put(target, NamespaceLoadState.LOADING);
    try {
      libraryLoader.ensure(this, target);
      HaraNamespace required =
          libraryLoader.provides(target)
              ? loadLibraryResource(target, true)
              : requireSourceNamespace(target, true);
      if (required == null) required = namespaces.get(target);
      if (required == null) {
        restore(snapshot);
        if (previousState != NamespaceLoadState.LOADED) {
          namespaceStates.put(target, NamespaceLoadState.FAILED);
          namespaceFailures.put(
              target, "no library, source, or extension provided this namespace");
        }
      } else {
        namespaceStates.put(target, NamespaceLoadState.LOADED);
        namespaceFailures.remove(target);
      }
      return required;
    } catch (RuntimeException failure) {
      restore(snapshot);
      if (previousState != NamespaceLoadState.LOADED) {
        namespaceStates.put(target, NamespaceLoadState.FAILED);
        namespaceFailures.put(
            target,
            failure.getMessage() == null
                ? failure.getClass().getSimpleName()
                : failure.getMessage());
      }
      throw failure;
    }
  }

  HaraNamespace loadLibraryResource(String namespaceName, boolean reload) {
    String previousNamespace = currentNamespace.name();
    HaraVar.Origin previousOrigin = definitionOrigin;
    ContextSnapshot snapshot = snapshot();
    currentNamespace = namespace(namespaceName);
    definitionOrigin = HaraVar.Origin.HAL_FALLBACK;
    try {
      HaraNamespace loaded = requireSourceNamespace(namespaceName, reload);
      if (FOUNDATION_NAMESPACE.equals(namespaceName)) {
        captureSequenceIntrinsics();
      }
      return loaded;
    } catch (RuntimeException error) {
      restore(snapshot);
      throw error;
    } finally {
      definitionOrigin = previousOrigin;
      currentNamespace = namespace(previousNamespace);
    }
  }

  private HaraNamespace requireSourceNamespace(String target) {
    return requireSourceNamespace(target, false);
  }

  private HaraNamespace requireSourceNamespace(String target, boolean reload) {
    ensureFoundationRoot(target);
    String resourceName = namespaceResource(target);
    Path source = resolveProjectSource(target);
    if (source != null) {
      requireResolvedSource(source.toString(), reload);
      return loadedSourceNamespace(target, source.toString());
    }
    if (getResource(resourceName) != null) {
      requireResolvedSource("classpath:" + resourceName, reload);
      return loadedSourceNamespace(target, "classpath:" + resourceName);
    }
    return null;
  }

  private Path resolveProjectSource(String target) {
    if (!environment.isFileIOAllowed()) return null;
    HaraProject currentProject = project();
    if (bytecodeLibrary.provides(target)) {
      return null;
    }
    return currentProject == null
        ? null
        : currentProject.resolve(target, target.endsWith("-test"));
  }

  private void requireResolvedSource(String source, boolean reload) {
    if (!reload) {
      requireModule(new Object[] {source});
      return;
    }
    requireModule(
        new Object[] {
          source,
          hara.lang.data.Map.Standard.from(
              null, Keyword.create("reload"), Boolean.TRUE)
        });
  }

  private HaraNamespace loadedSourceNamespace(String target, String source) {
    HaraNamespace loaded = namespaces.get(target);
    if (loaded == null) {
      throw new HaraException(
          "Namespace source " + source + " did not declare requested namespace " + target);
    }
    return loaded;
  }

  private boolean sourceNamespaceLoaded(String target) {
    return modules.values().stream().anyMatch(module -> target.equals(module.namespace));
  }

  private HaraProject project() {
    if (!projectDiscovered) {
      synchronized (this) {
        if (!projectDiscovered) {
          String workingDirectory =
              environment.getPublicTruffleFile(".").getAbsoluteFile().normalize().getPath();
          project = HaraProject.discover(Path.of(workingDirectory));
          projectDiscovered = true;
        }
      }
    }
    return project;
  }

  private static String namespaceResource(String namespace) {
    return namespace.replace('.', '/').replace('-', '_') + ".hal";
  }

  private void ensureFoundationRoot(String target) {
    if (!target.startsWith(FOUNDATION_NAMESPACE + ".")
        || FOUNDATION_NAMESPACE.equals(target)
        || eagerFallbacksLoading) {
      return;
    }
    if (namespaceStates.get(FOUNDATION_NAMESPACE) != NamespaceLoadState.LOADED) {
      requiredNamespace(FOUNDATION_NAMESPACE);
    }
  }

  private void ensureFoundationResource(String path) {
    String target = foundationResourceNamespace(path);
    if (target != null && !FOUNDATION_NAMESPACE.equals(target)) ensureFoundationRoot(target);
  }

  private static String foundationResourceNamespace(String path) {
    String resource = path.startsWith("classpath:") ? path.substring(10) : path;
    if ("std/foundation.hal".equals(resource) || "std/foundation.hbx".equals(resource)) {
      return FOUNDATION_NAMESPACE;
    }
    if (!resource.startsWith("std/foundation/")
        || !(resource.endsWith(".hal") || resource.endsWith(".hbx"))) {
      return null;
    }
    String child = resource.substring("std/foundation/".length());
    child = child.substring(0, child.lastIndexOf('.')).replace('/', '.').replace('_', '-');
    return child.isEmpty() ? null : FOUNDATION_NAMESPACE + "." + child;
  }

  private HaraNamespace installExtension(HaraExtensionPackage extensionPackage) {
    HaraExtensionManifest manifest = extensionPackage.manifest();
    HaraExtensionRuntime extension =
        "wasm".equals(manifest.provider())
            ? new HaraWasmExtension(extensionPackage)
            : new HaraProcessExtension(
                extensionPackage, environment.isCreateProcessAllowed());
    HaraNamespace generated = namespace(manifest.namespace());
    for (Map.Entry<String, HaraExtensionManifest.Export> export : manifest.exports().entrySet()) {
      String name = export.getKey();
      generated.define(
          name,
          new VariadicBuiltin(
              manifest.namespace() + "/" + name,
              values -> invokeExtension(extension, name, export.getValue(), values)));
    }
    loadedExtensions.put(manifest.namespace(), extension);
    return generated;
  }

  private Object invokeExtension(
      HaraExtensionRuntime extension,
      String name,
      HaraExtensionManifest.Export export,
      Object[] values) {
    if (extension.asynchronous()) return new HaraPromise(extension.invokeAsync(name, values));
    if (!export.async()) return extension.invoke(name, values);
    return new HaraPromise(CompletableFuture.supplyAsync(() -> extension.invoke(name, values)));
  }

  @TruffleBoundary
  public HaraVar resolve(Symbol symbol) {
    if (sandboxRestricted && sandboxForbidden(symbol)) return null;
    String namespaceName = symbol.getNamespace();
    if (namespaceName != null) {
      if ("-".equals(namespaceName)) namespaceName = currentNamespace.name();
      Map<String, String> currentAliases =
          aliases.getOrDefault(currentNamespace.name(), Map.of());
      boolean alias = currentAliases.containsKey(namespaceName);
      namespaceName = currentAliases.getOrDefault(namespaceName, namespaceName);
      libraryLoader.ensure(this, namespaceName);
      if (alias) {
        HaraNamespace required =
            sandboxRestricted
                    && "std.native.Runtime".equals(namespaceName)
                    && "ns-find".equals(symbol.getName())
                ? namespaces.get(namespaceName)
                : requiredNamespace(namespaceName);
        if (required == null) return null;
      }
    }
    HaraNamespace namespace =
        namespaceName == null ? currentNamespace : namespaces.get(namespaceName);
    HaraVar variable = namespace == null ? null : namespace.lookup(symbol.getName());
    if (variable == null && namespaceName == null) {
      String canonical = globalImports.get(symbol.getName());
      if (canonical != null && !canonical.equals(symbol.display())) {
        variable = resolve(Symbol.create(canonical));
      }
    }
    if (variable == null
        && namespaceName == null
        && symbol.getName().startsWith(PROTOCOL_NAMESPACE_PREFIX)) {
      HaraNamespace protocolNamespace = namespaces.get(symbol.getName());
      if (protocolNamespace != null) {
        String protocolName =
            symbol.getName().substring(symbol.getName().lastIndexOf('.') + 1);
        variable = protocolNamespace.lookup(protocolName);
      }
    }
    if (variable == null
        && namespaceName != null
        && (bytecodeLibrary.provides(namespaceName) || libraryLoader.provides(namespaceName))
        && namespaceStates.get(namespaceName) != NamespaceLoadState.LOADING) {
      namespace = requiredNamespace(namespaceName);
      variable = namespace == null ? null : namespace.lookup(symbol.getName());
    }
    return variable;
  }

  public Object resolveNamespaceValue(Symbol symbol) {
    if (symbol.getNamespace() != null) return null;
    String requested = symbol.getName();
    if (sandboxRestricted && sandboxForbiddenNamespace(requested)) return null;
    String target = aliases.getOrDefault(currentNamespace.name(), Map.of()).get(requested);
    if (target != null) return namespaces.get(target);
    return namespaces.get(requested);
  }

  private static boolean sandboxForbidden(Symbol symbol) {
    String namespace = symbol.getNamespace();
    if (namespace != null) {
      // Namespace lookup is a read-only capability: the public ns-find wrapper
      // must be able to report nil for a forbidden namespace without exposing
      // the rest of the Runtime surface to a sandbox.
      if ("std.native.Runtime".equals(namespace) && "ns-find".equals(symbol.getName())) {
        return false;
      }
      return sandboxForbiddenNamespace(namespace);
    }
    return switch (symbol.getName()) {
      case "Runtime", "Kernel", "Sandbox", "Package", "Crypto", "OS", "Process", "File", "Socket", "Host", "Work" -> true;
      default -> false;
    };
  }

  private static boolean sandboxForbiddenNamespace(String namespace) {
    return switch (namespace) {
      case "std.native.Runtime",
          "std.native.Kernel",
          "std.native.Sandbox",
          "std.native.Crypto",
          "std.native.File",
          "std.native.Socket",
          "std.native.Process",
          "std.native.OS",
          "std.native.Package",
          "std.native.Host",
          "std.native.Work" -> true;
      default -> false;
    };
  }

  boolean namespaceQualifierTargets(String qualifier, String target) {
    if (target.equals(qualifier)) return true;
    return target.equals(
        aliases.getOrDefault(currentNamespace.name(), Map.of()).get(qualifier));
  }

  Symbol canonicalSymbol(Symbol symbol) {
    if (hasNativeSymbol(symbol)) return symbol;
    HaraVar variable = resolve(symbol);
    if (variable != null) {
      return Symbol.create(variable.namespaceName(), variable.symbolName());
    }
    return symbol.getNamespace() == null
        ? Symbol.create(currentNamespace.name(), symbol.getName())
        : symbol;
  }

  void declareCurrent(Symbol symbol) {
    HaraVar existing = currentNamespace.lookup(symbol.getName());
    if (existing != null && !currentNamespace.name().equals(existing.namespaceName())) {
      currentNamespace.removeReferredVar(symbol.getName());
      existing = null;
    }
    if (existing == null) {
      currentNamespace.define(
          symbol.getName(), null, symbol.meta(), definitionOrigin);
    }
  }

  private HaraException protectedReferredVar(Symbol symbol, HaraVar existing) {
    return new HaraException(
        "Cannot replace referred Var without ns omission: "
            + symbol.getName()
            + " (referred from "
            + existing.namespaceName()
            + " into "
            + currentNamespace.name()
            + ")");
  }

  public void requireOwnedCurrent(Symbol symbol) {
    HaraVar existing = resolve(symbol);
    if (existing != null && !currentNamespace.name().equals(existing.namespaceName())) {
      currentNamespace.removeReferredVar(symbol.getName());
    }
  }

  /** Names visible in the current namespace, used by interactive tooling. */
  public java.util.List<String> currentSymbolNames() {
    LinkedHashSet<String> names = new LinkedHashSet<>(currentNamespace.symbolNames());
    names.addAll(HaraBuiltinCatalog.MARKER_METHOD_NAMES);
    nativeImports
        .getOrDefault(currentNamespace.name(), Map.of())
        .forEach(
            (simpleName, type) -> {
              names.add(simpleName);
              if (!(type instanceof Class<?>)) return;
              Class<?> cls = (Class<?>) type;
              java.util.Arrays.stream(cls.getFields())
                  .filter(field -> java.lang.reflect.Modifier.isStatic(field.getModifiers()))
                  .forEach(field -> names.add(simpleName + "/" + field.getName()));
              java.util.Arrays.stream(cls.getMethods())
                  .filter(method -> java.lang.reflect.Modifier.isStatic(method.getModifiers()))
                  .forEach(method -> names.add(simpleName + "/" + method.getName()));
            });
    namespaces.forEach(
        (namespaceName, namespace) -> {
          if (!namespaceName.startsWith("hara.native.")) return;
          for (String name : namespace.symbolNames()) names.add(namespaceName + "/" + name);
        });
    for (Map.Entry<String, String> alias :
        aliases.getOrDefault(currentNamespace.name(), Map.of()).entrySet()) {
      HaraNamespace target = namespaces.get(alias.getValue());
      if (target == null && libraryLoader.provides(alias.getValue())) {
        target = requiredNamespace(alias.getValue());
      }
      if (target == null) continue;
      for (String name : target.sortedSymbolNames()) names.add(alias.getKey() + "/" + name);
    }
    ArrayList<String> result = new ArrayList<>(names);
    result.sort(
        (left, right) -> {
          boolean leftPublic = isPublicCompletionSymbol(left);
          boolean rightPublic = isPublicCompletionSymbol(right);
          if (leftPublic != rightPublic) return leftPublic ? -1 : 1;
          return left.compareTo(right);
        });
    return result;
  }

  @TruffleBoundary
  @SuppressWarnings("unchecked")
  private boolean isPublicCompletionSymbol(String name) {
    HaraVar variable = resolve(Symbol.create(name));
    if (variable == null || !(variable.meta() instanceof ILookup<?, ?> metadata)) return false;
    Object value = ((ILookup<Object, Object>) metadata).lookup(Keyword.create("public"));
    return Boolean.TRUE.equals(value);
  }

  boolean isSpecialSymbol(Symbol symbol) {
    return symbol.getNamespace() == null
        && HaraBuiltinCatalog.SPECIAL_SYMBOLS.contains(symbol.getName());
  }

  @TruffleBoundary
  public Object macroExpand(Object form, boolean recursive) {
    Object result = form;
    int expansions = 0;
    do {
      Object expanded = macroExpandOnce(result, runtimeMacroEnvironment(result));
      if (expanded == result) return result;
      result = expanded;
      expansions++;
      if (expansions > 1000) throw new HaraException("macro expansion exceeded 1000 steps");
    } while (recursive);
    return result;
  }

  private Object macroExpandOnce(Object form, Object environment) {
    if (!(form instanceof List<?> list)
        || list.count() == 0
        || !(list.nth(0) instanceof Symbol operator)) {
      return form;
    }
    if (isSpecialSymbol(operator)) return form;
    HaraMacro macro = resolveMacro(operator);
    if (macro == null) return form;
    Object expansion = macro.expand(list, environment);
    EvaluationJournal.macro(operator.toString(), form, expansion);
    return expansion;
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  private Object runtimeMacroEnvironment(Object form) {
    ArrayList<Object> entries = new ArrayList<>();
    entries.add(Keyword.create("ns"));
    entries.add(Symbol.create(currentNamespace.name()));
    entries.add(Keyword.create("locals"));
    entries.add(hara.lang.data.Map.Standard.EMPTY);
    entries.add(Keyword.create("aliases"));
    entries.add(macroAliases());
    if (form instanceof hara.lang.protocol.IObjType object
        && object.meta() instanceof IMapType<?, ?> metadata) {
      for (String key : new String[] {"file", "line", "column"}) {
        Object value = ((IMapType) metadata).lookup(Keyword.create(key));
        if (value != null) {
          entries.add(Keyword.create(key));
          entries.add(value);
        }
      }
    }
    return hara.lang.data.Map.Standard.from(null, entries.toArray());
  }
  @TruffleBoundary
  public void defineAlias(Symbol alias, Symbol target) {
    if (alias.getNamespace() != null || target.getNamespace() != null) {
      throw new HaraException("alias names must be unqualified");
    }
    if (!namespaces.containsKey(target.getName())) {
      throw new HaraException("Cannot alias missing namespace: " + target.getName());
    }
    aliases
        .computeIfAbsent(currentNamespace.name(), ignored -> new ConcurrentHashMap<>())
        .put(alias.getName(), target.getName());
  }

  @TruffleBoundary
  public HaraVar define(Symbol symbol, Object value) {
    if (symbol.getNamespace() != null && !symbol.getNamespace().equals(currentNamespace.name())) {
      throw new HaraException("Cannot define a var in another namespace: " + symbol.display());
    }
    return currentNamespace.define(symbol.getName(), value, symbol.meta(), definitionOrigin);
  }

  @TruffleBoundary
  public <T> T withDeclarationTransaction(Supplier<T> operation) {
    ContextSnapshot snapshot = snapshot();
    try {
      return operation.get();
    } catch (RuntimeException error) {
      restore(snapshot);
      throw error;
    }
  }

  @TruffleBoundary
  public HaraType defineNamedType(
      Symbol symbol, HalcSchema.NamedField[] fields, boolean mutable) {
    if (symbol.getNamespace() != null) {
      throw new HaraException("named type must be defined in the current namespace");
    }
    HalcSchema.NamedField[] declaredSpecifications = fields.clone();
    String[] declaredFields =
        java.util.Arrays.stream(declaredSpecifications)
            .map(HalcSchema.NamedField::name)
            .toArray(String[]::new);
    String qualifiedName = currentNamespace.name() + "/" + symbol.getName();
    Object schema = HalcSchema.namedTypeSchema(qualifiedName, mutable, declaredSpecifications);
    IMapType<Object, Object> metadata =
        symbol.meta() instanceof IMapType<?, ?> existing
            ? (IMapType<Object, Object>) existing
            : hara.lang.data.Map.Standard.EMPTY;
    metadata =
        (IMapType<Object, Object>) metadata.assoc(Keyword.create("schema"), schema);
    Symbol typedSymbol = symbol.withMeta(metadata);
    HaraType type =
        mutable
            ? new HaraMutableType(
                qualifiedName, declaredFields, declaredSpecifications, schema)
            : new HaraType(qualifiedName, declaredFields, declaredSpecifications, schema);
    define(typedSymbol, type);
    define(Symbol.create("->" + symbol.getName()), type);
    define(
        Symbol.create("map->" + symbol.getName()),
        new VariadicBuiltin(
            currentNamespace.name() + "/map->" + symbol.getName(),
            values -> {
              if (values.length != 1) {
                throw new HaraException("map constructor expects one associative value");
              }
              Object source = HaraBox.unwrap(values[0]);
              if (!(source instanceof ILookup<?, ?> lookup)) {
                throw new HaraException("map constructor expects one associative value");
              }
              Object[] members = new Object[declaredFields.length];
              for (int index = 0; index < declaredFields.length; index++) {
                members[index] =
                    ((ILookup<Object, Object>) lookup).lookup(Keyword.create(declaredFields[index]));
              }
              try {
                return type.construct(members);
              } catch (com.oracle.truffle.api.interop.ArityException impossible) {
                throw new IllegalStateException("named type arity was checked", impossible);
              }
            }));
    return type;
  }

  public HaraProtocol ifnProtocol() {
    return ifnProtocol;
  }

  HaraProtocol defineInjectedProtocol(
      String name, Map<String, Integer> methodArities, java.util.List<HaraProtocol> parents) {
    return withDeclarationTransaction(
        () -> {
          HaraProtocol existing = protocol(name);
          if (existing != null) return existing;
          String canonicalNamespace = builtinProtocolNamespace(name);
          HaraProtocol protocol =
              new HaraProtocol(canonicalNamespace, methodArities, parents);
          HaraVar descriptor =
              namespace(canonicalNamespace).define(name, protocol, null, definitionOrigin);
          namespace(FOUNDATION_NAMESPACE).refer(name, descriptor);
          defineBuiltinProtocolMethods(canonicalNamespace, protocol, definitionOrigin);
          return protocol;
        });
  }

  HaraProtocol protocol(String name) {
    HaraVar variable = namespace(builtinProtocolNamespace(name)).lookup(name);
    if (variable == null || !(variable.get() instanceof HaraProtocol protocol)) return null;
    return protocol;
  }

  @TruffleBoundary
  public HaraVar defineLanguageProtocol(Symbol symbol, HaraProtocol protocol) {
    return withDeclarationTransaction(
        () -> {
          validateLanguageProtocolMethods(currentNamespace, symbol.getName(), protocol);
          HaraVar variable = define(symbol, protocol);
          defineLanguageProtocolMethods(currentNamespace.name(), protocol);
          return variable;
        });
  }

  private void validateLanguageProtocolMethods(
      HaraNamespace target, String protocolName, HaraProtocol protocol) {
    HaraProtocol previousProtocol = null;
    HaraVar previous = target.lookup(protocolName);
    if (previous != null
        && target.name().equals(previous.namespaceName())
        && previous.get() instanceof HaraProtocol) {
      previousProtocol = (HaraProtocol) previous.get();
    }
    for (String methodName : protocol.methods().keySet()) {
      for (Map.Entry<String, HaraVar> entry : target.vars.entrySet()) {
        if (entry.getKey().equals(protocolName)
            || !target.name().equals(entry.getValue().namespaceName())
            || !(entry.getValue().get() instanceof HaraProtocol otherProtocol)) {
          continue;
        }
        if (otherProtocol.methods().containsKey(methodName)) {
          throw new HaraException(
              "Protocol method Var already belongs to "
                  + entry.getKey()
                  + ": "
                  + target.name()
                  + "/"
                  + methodName);
        }
      }
      HaraVar existing = target.lookup(methodName);
      boolean sameProtocolReload =
          existing != null
              && target.name().equals(existing.namespaceName())
              && previousProtocol != null
              && previousProtocol.methods().containsKey(methodName);
      if (existing != null
          && target.name().equals(existing.namespaceName())
          && !sameProtocolReload) {
        throw new HaraException(
            "Protocol method Var already exists: " + target.name() + "/" + methodName);
      }
    }
  }

  private static String builtinProtocolNamespace(String protocolName) {
    return
        PROTOCOL_NAMESPACE_PREFIX
            + protocolName.toLowerCase(java.util.Locale.ROOT)
            + "."
            + protocolName;
  }

  private void defineBuiltinProtocolMethods(
      String namespaceName, HaraProtocol protocol, HaraVar.Origin origin) {
    HaraNamespace target = namespace(namespaceName);
    protocol
        .methods()
        .forEach(
            (methodName, ignored) ->
                target.define(
                    methodName,
                    new VariadicBuiltin(
                        namespaceName + "/" + methodName,
                        values -> invokeProtocolMethod(protocol, methodName, values)),
                    null,
                    origin));
  }

  private void defineLanguageProtocolMethods(String namespaceName, HaraProtocol protocol) {
    HaraNamespace target = namespace(namespaceName);
    protocol
        .methods()
        .forEach(
            (methodName, ignored) -> {
              target.define(
                  methodName,
                  new VariadicBuiltin(
                      namespaceName + "/" + methodName,
                      values -> invokeProtocolMethod(protocol, methodName, values)),
                  null,
                  definitionOrigin);
            });
  }

  private Object invokeProtocolMethod(
      HaraProtocol protocol, String methodName, Object[] values) {
    if (values.length == 0) {
      throw new HaraException(
          "protocol/arity: " + protocol.name() + "/" + methodName + " expects a receiver");
    }
    Object receiver = HaraBox.unwrap(values[0]);
    if (isHostObject(receiver)) receiver = asHostObject(receiver);
    Object[] arguments = new Object[values.length - 1];
    System.arraycopy(values, 1, arguments, 0, arguments.length);
    return protocol.invoke(methodName, receiver, arguments);
  }

  public boolean hostInteropAllowed() {
    return environment.isHostLookupAllowed();
  }

  public Object asGuestValue(Object value) {
    return environment.asGuestValue(value);
  }

  public boolean isHostObject(Object value) {
    return environment.isHostObject(value);
  }

  public Object asHostObject(Object value) {
    return environment.asHostObject(value);
  }

  public Object lookupHostSymbol(String name) {
    return environment.lookupHostSymbol(name);
  }

  private NativeFlavorAccess nativeAccess() {
    ClassLoader loader = nativeFlavorLoader;
    if (loader == null) loader = Thread.currentThread().getContextClassLoader();
    if (loader == null) loader = HaraContext.class.getClassLoader();
    Set<NativeCapability> capabilities =
        hostInteropAllowed() ? Set.of(NativeCapability.REFLECTION) : Set.of();
    return NativeFlavorAccess.of(loader, capabilities);
  }

  private NativeFlavorProvider nativeProvider() {
    String flavor = nativeFlavors.get(currentNamespace.name());
    return nativeFlavorRegistry.require(flavor == null ? "jvm" : flavor);
  }

  private JvmFlavorProvider jvmProvider() {
    NativeFlavorProvider provider = nativeProvider();
    if (!(provider instanceof JvmFlavorProvider)) {
      throw new HaraException("JVM native operation requires the JVM native flavor");
    }
    return (JvmFlavorProvider) provider;
  }

  @TruffleBoundary
  public boolean hasNativeSymbol(Symbol symbol) {
    Map<String, Object> imports = nativeImports.get(currentNamespace.name());
    if (imports == null) return false;
    String importedName = symbol.getNamespace() == null ? symbol.getName() : symbol.getNamespace();
    return imports.containsKey(importedName);
  }

  @TruffleBoundary
  public Object resolveNativeSymbol(Symbol symbol) {
    Map<String, Object> imports = nativeImports.get(currentNamespace.name());
    if (imports == null) throw new HaraException("No native imports in the current namespace");
    String importedName = symbol.getNamespace() == null ? symbol.getName() : symbol.getNamespace();
    Object type = imports.get(importedName);
    if (type == null) throw new HaraException("Native type is not imported: " + importedName);
    if (symbol.getNamespace() == null) return type;
    NativeFlavorProvider provider = nativeProvider();
    try {
      return provider.readStatic(type, symbol.getName(), nativeAccess());
    } catch (NativeFlavorException error) {
      if (error.kind() != NativeFlavorException.Kind.UNSUPPORTED) throw error;
      return new VariadicBuiltin(
          symbol.display(),
          arguments -> provider.invokeStatic(type, symbol.getName(), arguments, nativeAccess()));
    }
  }

  @TruffleBoundary
  public boolean matchesNativeThrowable(Symbol type, Throwable throwable) {
    if (!hasNativeSymbol(type)) return false;
    NativeFlavorProvider provider = nativeProvider();
    return provider != null
        && provider.matchesThrowable(resolveNativeSymbol(type), throwable, nativeAccess());
  }

  @TruffleBoundary
  public Object constructNative(Object type, Object[] arguments) {
    NativeFlavorProvider provider = nativeProvider();
    return provider.construct(HaraBox.unwrap(type), arguments, nativeAccess());
  }

  @TruffleBoundary
  public Object readNativeMember(Object receiver, String member) {
    NativeFlavorProvider provider = nativeProvider();
    return provider.readMember(HaraBox.unwrap(receiver), member, nativeAccess());
  }

  @TruffleBoundary
  public Object indexNative(Object receiver, Object index) {
    NativeFlavorProvider provider = nativeProvider();
    return provider.index(HaraBox.unwrap(receiver), HaraBox.unwrap(index), nativeAccess());
  }

  @TruffleBoundary
  HaraMacro resolveMacro(Symbol symbol) {
    String namespace = symbol.getNamespace();
    if (namespace != null) {
      if ("-".equals(namespace)) namespace = currentNamespace.name();
      namespace = aliases.getOrDefault(currentNamespace.name(), Map.of())
          .getOrDefault(namespace, namespace);
    }
    String namespaceName = namespace == null ? currentNamespace.name() : namespace;
    Map<String, HaraMacro> namespaceMacros = macros.get(namespaceName);
    HaraMacro macro = namespaceMacros == null ? null : namespaceMacros.get(symbol.getName());
    if (macro == null && namespace == null) {
      Map<String, HaraMacro> foundationMacros = macros.get(FOUNDATION_NAMESPACE);
      macro = foundationMacros == null ? null : foundationMacros.get(symbol.getName());
    }
    if (macro != null || INTRINSIC_NAMESPACE.equals(namespaceName)) return macro;
    if (namespace != null) return null;
    Map<String, HaraMacro> intrinsicMacros = macros.get(INTRINSIC_NAMESPACE);
    return intrinsicMacros == null ? null : intrinsicMacros.get(symbol.getName());
  }

  void defineMacro(Symbol symbol, HaraMacro macro) {
    if (symbol.getNamespace() != null) {
      throw new HaraException("defmacro name must not be qualified");
    }
    HaraVar existing = currentNamespace.lookup(symbol.getName());
    if (existing != null
        && FOUNDATION_NAMESPACE.equals(existing.namespaceName())
        && !FOUNDATION_NAMESPACE.equals(currentNamespace.name())) {
      currentNamespace.removeReferredVar(symbol.getName(), FOUNDATION_NAMESPACE);
      existing = null;
    }
    if (definitionOrigin == HaraVar.Origin.HAL_FALLBACK
        && existing != null
        && (existing.origin() == HaraVar.Origin.JAVA_LIBRARY
            || existing.origin() == HaraVar.Origin.RUNTIME_PRIMITIVE)) {
      return;
    }
    requireOwnedCurrent(symbol);
    macros
        .computeIfAbsent(currentNamespace.name(), ignored -> new ConcurrentHashMap<>())
        .put(symbol.getName(), macro);
    currentNamespace.define(symbol.getName(), macro, symbol.meta(), definitionOrigin);
  }

  private void defineIntrinsicMacro(Symbol symbol, HaraMacro macro) {
    HaraNamespace previous = currentNamespace;
    try {
      currentNamespace = namespace(INTRINSIC_NAMESPACE);
      defineMacro(symbol, macro);
    } finally {
      currentNamespace = previous;
    }
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  private void installProjectMacro() {
    defineIntrinsicMacro(
        Symbol.create("defproject"),
        HaraMacro.nativeMacro(
            Symbol.create("defproject"),
            invocation -> {
              if (invocation.count() != 3
                  || !(invocation.nth(1) instanceof Symbol name)
                  || name.getNamespace() != null
                  || !(invocation.nth(2) instanceof IMapType<?, ?> options)) {
                throw new HaraException(
                    "defproject expects an unqualified project name and options map");
              }
              IMapType descriptor =
                  (IMapType)
                      ((IMapType) options)
                          .assoc(
                              Keyword.create("name"),
                              List.Standard.from(null, Symbol.create("quote"), name));
              return List.Standard.from(
                  null, Symbol.create("def"), Symbol.create("project"), descriptor);
            }));
  }

  private void installNativeResultBuiltins() {
    HaraNamespace result = namespace("std.native.Result");
    result.define(
        "create",
        new VariadicBuiltin(
            "std.native.Result/create",
            values -> {
              if (values.length < 2 || values.length > 3) {
                throw new HaraException(
                    "std.native.Result/create expects status, value, and optional context");
              }
              Object status = HaraBox.unwrap(values[0]);
              Object value = HaraBox.unwrap(values[1]);
              Object context = values.length == 3 ? HaraBox.unwrap(values[2]) : null;
              if (Keyword.create("success").equals(status)) {
                return values.length == 2
                    ? HaraResult.success(value)
                    : HaraResult.success(value, context);
              }
              if (Keyword.create("error").equals(status)) {
                return values.length == 2
                    ? HaraResult.error(value)
                    : HaraResult.error(value, context);
              }
              throw new HaraException("std.native.Result/create status must be :success or :error");
            }));
    result.define(
        "synchronize",
        new VariadicBuiltin(
            "std.native.Result/synchronize",
            values -> {
              if (values.length < 1 || values.length > 2) {
                throw new HaraException(
                    "std.native.Result/synchronize expects a value and optional options map");
              }
              return values.length == 1
                  ? HaraResult.synchronize(HaraBox.unwrap(values[0]))
                  : HaraResult.synchronize(HaraBox.unwrap(values[0]), HaraBox.unwrap(values[1]));
            }));
    result.define(
        "success?",
        new UnaryBuiltin(
            "std.native.Result/success?",
            value ->
                HaraBox.unwrap(value) instanceof HaraResult nativeResult
                    && nativeResult.isSuccess()));
    result.define(
        "error?",
        new UnaryBuiltin(
            "std.native.Result/error?",
            value ->
                HaraBox.unwrap(value) instanceof HaraResult nativeResult
                    && nativeResult.isError()));
    result.define(
        "status",
        new UnaryBuiltin(
            "std.native.Result/status",
            value -> requireNativeResult(value, "status").status()));
    result.define(
        "data",
        new UnaryBuiltin(
            "std.native.Result/data",
            value -> requireNativeResult(value, "data").data()));
    result.define(
        "error-value",
        new UnaryBuiltin(
            "std.native.Result/error-value",
            value -> requireNativeResult(value, "error-value").errorValue()));
    result.define(
        "context",
        new UnaryBuiltin(
            "std.native.Result/context",
            value -> {
              IMapType<Object, Object> context = requireNativeResult(value, "context").context();
              return context.count() == 0 ? null : context;
            }));
    result.define(
        "with-context",
        new VariadicBuiltin(
            "std.native.Result/with-context",
            values -> {
              if (values.length != 2) {
                throw new HaraException(
                    "std.native.Result/with-context expects a Result and context");
              }
              return requireNativeResult(values[0], "with-context")
                  .withContext(HaraBox.unwrap(values[1]));
            }));
  }

  private static HaraResult requireNativeResult(Object value, String operation) {
    Object raw = HaraBox.unwrap(value);
    if (raw instanceof HaraResult result) return result;
    throw new HaraException("std.native.Result/" + operation + " expects a Result");
  }

  private void installNativeStreamBuiltins() {
    HaraNamespace stream = namespace("std.native.Stream");
    stream.define(
        "create",
        new VariadicBuiltin(
            "std.native.Stream/create",
            values -> {
              if (values.length < 1 || values.length > 2) {
                throw new HaraException("Stream/create expects next and optional close functions");
              }
              Object next = HaraBox.unwrap(values[0]);
              Object close = values.length == 2 ? HaraBox.unwrap(values[1]) : null;
              if (!isFunctionValue(next)
                  || (close != null && !isFunctionValue(close))) {
                throw new HaraException("Stream/create expects callable next and close values");
              }
              return new HaraCallbackStream(this, next, close);
            }));
    stream.define(
        "generate",
        new VariadicBuiltin(
            "std.native.Stream/generate",
            values -> {
              if (values.length == 0) throw new HaraException("Stream/generate expects a function");
              Object function = HaraBox.unwrap(values[0]);
              if (!(function instanceof HaraFunction)
                  && !(function instanceof HaraMultiFunction)
                  && !(function instanceof HbcMachine.HbcClosure)
                  && !(function instanceof HbcMachine.HbcMultiArity)
                  && !(function instanceof hara.lang.protocol.IFn)) {
                throw new HaraException("Stream/generate expects a function");
              }
              return new HaraStream(
                  this, function, java.util.Arrays.copyOfRange(values, 1, values.length));
            }));
    stream.define(
        "next",
        new UnaryBuiltin(
            "std.native.Stream/next", value -> requireStream(value, "Stream/next").next()));
  }

  private hara.lang.protocol.IStream requireStream(Object value, String operation) {
    Object input = HaraBox.unwrap(value);
    if (!(input instanceof hara.lang.protocol.IStream)) {
      throw new HaraException(operation + " expects a stream");
    }
    return (hara.lang.protocol.IStream) input;
  }

  private void installNumericBuiltins(HaraNamespace target) {
    target.define("+", new VariadicBuiltin("+", values -> arithmetic("+", values)));
    target.define("-", new VariadicBuiltin("-", values -> arithmetic("-", values)));
    target.define("*", new VariadicBuiltin("*", values -> arithmetic("*", values)));
    target.define("/", new VariadicBuiltin("/", values -> arithmetic("/", values)));
    target.define("quot", new VariadicBuiltin("quot", values -> arithmetic("quot", values)));
    target.define("rem", new VariadicBuiltin("rem", values -> arithmetic("rem", values)));
    target.define("mod", new VariadicBuiltin("mod", values -> arithmetic("mod", values)));
    target.define("=", new VariadicBuiltin("=", values -> compare("=", values)));
    target.define("<", new VariadicBuiltin("<", values -> compare("<", values)));
    target.define("<=", new VariadicBuiltin("<=", values -> compare("<=", values)));
    target.define(">", new VariadicBuiltin(">", values -> compare(">", values)));
    target.define(">=", new VariadicBuiltin(">=", values -> compare(">=", values)));
    HaraNamespace num = namespace("std.native.Num");
    num.define(
        "long",
        new UnaryBuiltin("std.native.Num/long", HaraNumericConversions::toLongTruncating));
    num.define("double", new UnaryBuiltin("std.native.Num/double", HaraNumericConversions::toDouble));
    num.define("parse-long", new UnaryBuiltin("std.native.Num/parse-long", this::parseLong));
    num.define("parse-double", new UnaryBuiltin("std.native.Num/parse-double", this::parseDouble));
    target.define(
        "not-nil?",
        new UnaryBuiltin(
            "not-nil?",
            value -> {
              Object unwrapped = HaraBox.unwrap(value);
              return unwrapped != null && unwrapped != HaraNull.SINGLETON;
            }));
    target.define("false?", new UnaryBuiltin("false?", value -> Boolean.FALSE.equals(HaraBox.unwrap(value))));
    target.define("true?", new UnaryBuiltin("true?", value -> Boolean.TRUE.equals(HaraBox.unwrap(value))));
    target.define(
        "empty?",
        new UnaryBuiltin(
            "empty?", value -> !Boolean.TRUE.equals(iterHasNext(iterValue(value)))));
    target.define("vec", new UnaryBuiltin("vec", this::toVector));
    target.define("set", new UnaryBuiltin("set", this::toSet));
    target.define("reverse", new UnaryBuiltin("reverse", this::reverseValue));
    // Bootstrap seed only; canonical std.foundation/fn? is defined by HAL.
    target.define(
        "number?", new UnaryBuiltin("number?", HaraNumericConversions::isNumeric));
    target.define(
        "long?", new UnaryBuiltin("long?", HaraNumericConversions::fitsLong));
    target.define("satisfies?", new VariadicBuiltin("satisfies?", values -> {
      if (values.length != 2 || !(HaraBox.unwrap(values[0]) instanceof HaraProtocol protocol)) {
        throw new HaraException("satisfies? expects a protocol and value");
      }
      return protocolSatisfies(protocol, HaraBox.unwrap(values[1]));
    }));
    target.define(
        "instance?",
        new VariadicBuiltin(
            "instance?",
            values -> {
              if (values.length != 2) {
                throw new HaraException("instance? expects a Hara type and value");
              }
              Object type = HaraBox.unwrap(values[0]);
              Object value = HaraBox.unwrap(values[1]);
              if (type instanceof HaraNativeType nativeType) {
                return portableType(value).equals(Keyword.create("std.native." + nativeType.getName()));
              }
              if (!(type instanceof HaraType)) {
                throw new HaraException("instance? expects a type descriptor");
              }
              if (value instanceof HaraStruct struct) {
                return struct.type() == type;
              }
              return value instanceof HaraMutable mutable && mutable.type() == type;
            }));
    target.define("schema", new UnaryBuiltin("schema", value -> compileSchema(value, null)));
    target.define(
        "schema-of",
        new UnaryBuiltin(
            "schema-of",
            value -> {
              Object raw = HaraBox.unwrap(value);
              if (!(raw instanceof HaraVar variable)) {
                throw new HaraException("schema-of expects a Var");
              }
              return schemaContract(variable);
            }));
    HaraNamespace schemaNative = namespace("std.native.Schema");
    schemaNative.define(
        "compile",
        new UnaryBuiltin("std.native.Schema/compile", value -> compileSchema(value, null)));
    schemaNative.define(
        "of",
        new UnaryBuiltin(
            "std.native.Schema/of",
            value -> {
              Object raw = HaraBox.unwrap(value);
              if (!(raw instanceof HaraVar variable)) {
                throw new HaraException("Schema/of expects a Var");
              }
              return schemaContract(variable);
            }));
    schemaNative.define(
        "kind",
        new UnaryBuiltin(
            "std.native.Schema/kind",
            value -> Keyword.create(schemaKind(requireSchema(value, "Schema/kind").ast()))));
    schemaNative.define(
        "form", new UnaryBuiltin("std.native.Schema/form", value -> requireSchema(value, "Schema/form").form()));
    schemaNative.define(
        "ast", new UnaryBuiltin("std.native.Schema/ast", value -> schemaAst(requireSchema(value, "Schema/ast").ast())));
    schemaNative.define(
        "origin", new UnaryBuiltin("std.native.Schema/origin", value -> requireSchema(value, "Schema/origin").origin()));
    target.define(
        "symbol",
        new VariadicBuiltin(
            "symbol",
            values -> {
              if (values.length == 1) {
                Object value = HaraBox.unwrap(values[0]);
                if (value instanceof Symbol) return value;
                if (value instanceof Keyword keyword) {
                  return Symbol.create(keyword.display().substring(1));
                }
                if (value instanceof String text) return Symbol.create(text);
              } else if (values.length == 2) {
                return Symbol.create(
                    String.valueOf(HaraBox.unwrap(values[0])),
                    String.valueOf(HaraBox.unwrap(values[1])));
              }
              throw new HaraException("symbol expects a name or namespace and name");
            }));
    target.define(
        "keyword",
        new VariadicBuiltin(
            "keyword",
            values -> {
              if (values.length == 1) {
                Object unwrapped = HaraBox.unwrap(values[0]);
                if (unwrapped instanceof Keyword) return unwrapped;
                if (unwrapped instanceof Symbol symbol) return Keyword.create(symbol.display());
                if (unwrapped instanceof String text) return Keyword.create(text);
              } else if (values.length == 2) {
                return Keyword.create(
                    String.valueOf(HaraBox.unwrap(values[0])),
                    String.valueOf(HaraBox.unwrap(values[1])));
              }
              throw new HaraException("keyword expects a name or namespace and name");
            }));
    target.define(
        "ex",
        new VariadicBuiltin(
            "ex",
            values -> {
              if (values.length < 2 || values.length % 2 != 0) {
                throw new HaraException("ex expects a code, attributes map, and key/value pairs");
              }
              Object codeValue = values[0];
              Object attributesValue = values[1];
              if (values.length > 2) {
                Object[] assocValues = new Object[values.length - 1];
                assocValues[0] = attributesValue;
                System.arraycopy(values, 2, assocValues, 1, values.length - 2);
                attributesValue = associateValues(assocValues);
              }
              Object rawCode = HaraBox.unwrap(codeValue);
              Object rawAttributes = HaraBox.unwrap(attributesValue);
              if (!(rawCode instanceof Keyword inputCode)) {
                throw new HaraException(
                    "ex expects a registered standard keyword or namespaced keyword code");
              }
              Keyword code = normalizeExceptionCode(inputCode);
              if (!(rawAttributes instanceof IMapType attributes)) {
                throw new HaraException("ex expects an attributes map");
              }
              Object message = attributes.lookup(Keyword.create("ex", "message"));
              if (message != null && !(message instanceof String)) {
                throw new HaraException(":ex/message must be a string");
              }
              if (attributes.lookup(Keyword.create("ex", "code")) != null) {
                throw new HaraException(
                    "ex attributes must not contain :ex/code; pass the code as the first argument");
              }
              Object classValue = attributes.lookup(Keyword.create("ex", "class"));
              if (classValue != null
                  && (!(classValue instanceof Keyword exceptionClass)
                      || exceptionClass.getNamespace() == null)) {
                throw new HaraException(":ex/class must be a namespaced keyword");
              }
              Keyword registeredClass = defaultExceptionClass(code);
              if (classValue != null
                  && registeredClass != null
                  && !registeredClass.equals(classValue)) {
                throw new HaraException(
                    ":ex/class conflicts with the registered class for :ex/code");
              }
              Object causeValue = attributes.lookup(Keyword.create("ex", "cause"));
              if (causeValue != null
                  && causeValue != HaraNull.SINGLETON
                  && !(causeValue instanceof hara.lang.base.Ex.Info)) {
                throw new HaraException(":ex/cause must be an Exception");
              }
              Object contextValue = attributes.lookup(Keyword.create("ex", "context"));
              if (contextValue != null && !(contextValue instanceof IMapType)) {
                throw new HaraException(":ex/context must be a map");
              }
              Throwable cause =
                  causeValue instanceof hara.lang.base.Ex.Info
                      ? (hara.lang.base.Ex.Info) causeValue
                      : null;
              IMetadata data =
                  (IMetadata) attributes.assoc(Keyword.create("ex", "code"), code);
              if (classValue == null && registeredClass != null) {
                data =
                    (IMetadata)
                        ((IMapType) data).assoc(Keyword.create("ex", "class"), registeredClass);
              }
              return new hara.lang.base.Ex.Info(
                  message instanceof String ? (String) message : code.display(), data, cause);
            },
            true));
    target.define(
        "ex-info",
        new VariadicBuiltin(
            "ex-info",
            values -> {
              if (values.length < 2
                  || values.length > 3
                  || !(HaraBox.unwrap(values[0]) instanceof String)
                  || !(HaraBox.unwrap(values[1]) instanceof IMetadata)) {
                throw new HaraException("ex-info expects a message, metadata map, and optional cause");
              }
              Object rawCause = values.length == 3 ? HaraBox.unwrap(values[2]) : null;
              if (rawCause != null
                  && rawCause != HaraNull.SINGLETON
                  && !(rawCause instanceof hara.lang.base.Ex.Info)) {
                throw new HaraException("ex-info expects an Exception cause");
              }
              Throwable cause =
                  rawCause instanceof hara.lang.base.Ex.Info
                      ? (hara.lang.base.Ex.Info) rawCause
                      : null;
              return new hara.lang.base.Ex.Info(
                  (String) HaraBox.unwrap(values[0]),
                  (IMetadata) HaraBox.unwrap(values[1]),
                  cause);
            },
            true));
    target.define(
        "ex-data",
        new UnaryBuiltin(
            "ex-data",
            value -> {
              Object unwrapped = HaraBox.unwrap(value);
              return unwrapped instanceof hara.lang.protocol.IExInfo
                  ? ((hara.lang.protocol.IExInfo) unwrapped).getData()
                  : null;
            }));
    target.define(
        "ex-provenance",
        new UnaryBuiltin(
            "ex-provenance",
            value -> {
              Object raw = HaraBox.unwrap(value);
              if (!(raw instanceof hara.lang.base.Ex.Info info)) {
                throw new HaraException("ex-provenance expects an Exception");
              }
              Object created = exceptionSiteValue(info.createdAt());
              Object[] throwsAt = info.throwsAt().stream().map(this::exceptionSiteValue).toArray();
              return hara.lang.data.Map.Standard.from(
                  null,
                  Keyword.create("ex", "created-at"),
                  created,
                  Keyword.create("ex", "throws"),
                  hara.lang.data.Vector.Standard.from(null, throwsAt));
            }));
    target.define(
        "ex-message",
        new UnaryBuiltin(
            "ex-message",
            value -> {
              Object unwrapped = HaraBox.unwrap(value);
              return unwrapped instanceof Throwable
                  ? ((Throwable) unwrapped).getMessage()
                  : String.valueOf(unwrapped);
            }));
    target.define(
        "ex-cause",
        new UnaryBuiltin(
            "ex-cause",
            value -> {
              Object raw = HaraBox.unwrap(value);
              if (!(raw instanceof hara.lang.base.Ex.Info info)) {
                throw new HaraException("ex-cause expects an Exception");
              }
              return info.getCause();
            }));
    target.define(
        "ex-class",
        new UnaryBuiltin(
            "ex-class",
            value -> {
              Object unwrapped = HaraBox.unwrap(value);
              if (!(unwrapped instanceof hara.lang.base.Ex.Info info)) {
                throw new HaraException("ex-class expects an Exception");
              }
              if (!(info.getData() instanceof IMapType data)) {
                throw new HaraException("Exception data must be a map");
              }
              Object exceptionClass = data.lookup(Keyword.create("ex", "class"));
              if (exceptionClass == null) return null;
              if (!(exceptionClass instanceof Keyword keyword)
                  || keyword.getNamespace() == null) {
                throw new HaraException(":ex/class must be a namespaced keyword");
              }
              return exceptionClass;
            }));
    target.define(
        "ex-native-type",
        new UnaryBuiltin(
            "ex-native-type",
            value -> {
              Object unwrapped = HaraBox.unwrap(value);
              if (!(unwrapped instanceof Throwable throwable)) {
                throw new HaraException("ex-native-type expects an Exception");
              }
              return throwable instanceof hara.lang.base.Ex.Info
                  ? null
                  : throwable.getClass().getName();
            }));
    target.define("load-string", new UnaryBuiltin("load-string", this::loadString));
    target.define("read-string", new UnaryBuiltin("read-string", this::readString));
    target.define("eval", new UnaryBuiltin("eval", this::evalForm));
    target.define("load-file", new UnaryBuiltin("load-file", this::loadFile));
    target.define("load-resource", new UnaryBuiltin("load-resource", this::loadResource));
    target.define("read-forms", new VariadicBuiltin("read-forms", this::readForms));
    target.define("require", new VariadicBuiltin("require", this::requireModule));
    target.define("refer", new UnaryBuiltin("refer", this::referNamespace));
    target.define(
        "name",
        new UnaryBuiltin(
            "name", value -> protocolCall("INamespaced", "name", new Object[] {value})));
    target.define(
        "namespace",
        new UnaryBuiltin(
            "namespace",
            value -> protocolCall("INamespaced", "namespace", new Object[] {value})));
    target.define("in-ns", new UnaryBuiltin("in-ns", this::inNamespace));
    target.define("ns-aliases", new UnaryBuiltin("ns-aliases", this::namespaceAliases));
    target.define("intern-var", new VariadicBuiltin("intern-var", this::internVar));
    target.define("ns-state", new UnaryBuiltin("ns-state", this::namespaceState));
    target.define("ns-loaded?", new UnaryBuiltin("ns-loaded?", this::namespaceLoaded));
    target.define("ns-alias-state", new VariadicBuiltin("ns-alias-state", this::namespaceAliasState));
    target.define("eval-in-ns", new VariadicBuiltin("eval-in-ns", this::evalInNamespace));
    target.define(
        "resolve",
        new UnaryBuiltin(
            "resolve",
            value -> {
              Object raw = HaraBox.unwrap(value);
              if (!(raw instanceof Symbol symbol)) {
                throw new HaraException("resolve expects a symbol");
              }
              return resolveAvailable(symbol);
            }));
    target.define(
        "ns-current",
        new VariadicBuiltin(
            "ns-current",
            values -> {
              requireMethodArity("ns-current", values, 0);
              return currentNamespace.name();
            }));
    target.define(
        "current-symbols",
        new VariadicBuiltin(
            "current-symbols",
            values -> {
              requireMethodArity("current-symbols", values, 0);
              return currentSymbolNames().toArray();
            }));
    target.define("use", new UnaryBuiltin("use", this::useNamespace));
    target.define("iter", new UnaryBuiltin("iter", this::iterValue));
    target.define("seq", new VariadicBuiltin("seq", this::seqValue));
    target.define("iter-finite?", new UnaryBuiltin("iter-finite?", this::isIteratorFinite));
    target.define("iter-materialize", new UnaryBuiltin("iter-materialize", this::iterMaterialize));
    target.define("iter-next?", new UnaryBuiltin("iter-next?", this::iterHasNext));
    target.define("iter-next", new UnaryBuiltin("iter-next", this::iterNext));
    target.define("iter-close", new UnaryBuiltin("iter-close", this::iterClose));
    target.define("iter-concat", new VariadicBuiltin("iter-concat", this::concatIterators));
    target.define("iter-map", new VariadicBuiltin("iter-map", this::iterMap));
    target.define("iter-filter", new VariadicBuiltin("iter-filter", this::iterFilter));
    target.define("iter-take-while", new VariadicBuiltin("iter-take-while", this::iterTakeWhile));
    target.define("iter-drop-while", new VariadicBuiltin("iter-drop-while", this::iterDropWhile));
    target.define("iter-mapcat", new VariadicBuiltin("iter-mapcat", this::iterMapcat));
    target.define("iter-keep", new VariadicBuiltin("iter-keep", this::iterKeep));
    target.define("iter-interpose", new VariadicBuiltin("iter-interpose", this::iterInterpose));
    target.define("iter-interleave", new VariadicBuiltin("iter-interleave", this::iterInterleave));
    target.define("iter-every?", new VariadicBuiltin("iter-every?", this::iterEvery));
    target.define("iter-any?", new VariadicBuiltin("iter-any?", this::iterAny));
    target.define("iter-take", new VariadicBuiltin("iter-take", this::iterTake));
    target.define("iter-drop", new VariadicBuiltin("iter-drop", this::iterDrop));
    target.define("iter-zip", new VariadicBuiltin("iter-zip", this::iterZip));
    target.define("iter-cycle", new UnaryBuiltin("iter-cycle", this::iterCycle));
    target.define(
        "iter-partition-pair", new UnaryBuiltin("iter-partition-pair", this::iterPartitionPair));
    target.define(
        "iter-partition-all",
        new VariadicBuiltin("iter-partition-all", values -> iterPartition(values, true)));
    target.define(
        "iter-partition",
        new VariadicBuiltin("iter-partition", values -> iterPartition(values, false)));
    target.define("iter-range", new VariadicBuiltin("iter-range", this::iterRange));
    target.define("iter-constantly", new UnaryBuiltin("iter-constantly", Iter::constantly));
    target.define("iter-repeatedly", new UnaryBuiltin("iter-repeatedly", this::iterRepeatedly));
    target.define("iter-iterate", new VariadicBuiltin("iter-iterate", this::iterIterate));
    target.define(
        "reduced",
        new UnaryBuiltin("Base/reduced", value -> Reduced.mark(HaraBox.unwrap(value))));
    target.define(
        "unreduced",
        new UnaryBuiltin("Base/unreduced", value -> Reduced.unreduced(HaraBox.unwrap(value))));
    target.define("alter-var-root", new VariadicBuiltin("alter-var-root", this::alterVarRoot));
    target.define("module-revision", new UnaryBuiltin("module-revision", this::moduleRevision));
    target.define(
        "module-dependencies", new UnaryBuiltin("module-dependencies", this::moduleDependencies));
    target.define(
        "find", new VariadicBuiltin("find", values -> protocolCall("IFind", "find", values)));
    target.define(
        "has?",
        new VariadicBuiltin(
            "has?",
            values -> {
              if (values.length != 2) {
                throw new HaraException("has? expects a collection and key");
              }
              return !HaraBox.isNil(protocolCall("IFind", "find", values));
            }));
    target.define("conj", new VariadicBuiltin("conj", this::conjoin));
    target.define(
        "cons",
        new VariadicBuiltin(
            "cons",
            values -> {
              if (values.length != 2) {
                throw new HaraException("cons expects an item and a collection");
              }
              return protocolCall("ICons", "cons", new Object[] {values[1], values[0]});
            }));
    VariadicBuiltin nthBuiltin =
        new VariadicBuiltin("nth", values -> protocolCall("INth", "nth", values));
    target.define("nth", nthBuiltin);
    intrinsicCollectionBuiltins.put("nth", nthBuiltin);
    target.define(
        "empty",
        new UnaryBuiltin("empty", value -> protocolCall("IEmpty", "empty", new Object[] {value})));
    target.define(
        "peek",
        new UnaryBuiltin(
            "peek", value -> protocolCall("IPeekFirst", "peek-first", new Object[] {value})));
    target.define(
        "pop",
        new UnaryBuiltin(
            "pop", value -> protocolCall("IPopFirst", "pop-first", new Object[] {value})));
  }


  @SuppressWarnings({"rawtypes", "unchecked"})
  static Object lookupValue(IMapType<?, ?> map, Object key) {
    return ((IMapType) map).lookup(key);
  }

  private Object conjoin(Object[] values) {
    Object result;
    int firstValue;
    if (values.length < 2) {
      result = BuiltinStruct.vector(new Object[0]);
      firstValue = 0;
    } else {
      result = values[0];
      firstValue = 1;
    }
    for (int i = firstValue; i < values.length; i++) {
      result = protocolCall("IConj", "conj", new Object[] {result, values[i]});
    }
    return result;
  }

  private Object associateValues(Object[] values) {
    if (values.length < 3 || values.length % 2 == 0) {
      throw new HaraException("assoc expects a collection and key/value pairs");
    }
    Object result = values[0];
    for (int i = 1; i < values.length; i += 2) {
      result = protocolCall("IAssoc", "assoc", new Object[] {result, values[i], values[i + 1]});
    }
    return result;
  }

  private Object toVector(Object value) {
    Object raw = HaraBox.unwrap(value);
    if (raw instanceof hara.lang.data.Vector<?>) return value;
    ArrayList<Object> elements = new ArrayList<>();
    Iterator<?> iterator = (Iterator<?>) iterValue(raw);
    while (iterator.hasNext()) elements.add(iterator.next());
    return hara.lang.data.Vector.Standard.from(null, elements.toArray());
  }

  private Object toSet(Object value) {
    Object raw = HaraBox.unwrap(value);
    if (raw instanceof hara.lang.protocol.ISetType<?>) return value;
    return hara.lang.data.Set.Standard.into((Iterator<?>) iterValue(raw));
  }

  private Object reverseValue(Object value) {
    ArrayList<Object> elements = new ArrayList<>();
    Iterator<?> iterator = (Iterator<?>) iterValue(value);
    while (iterator.hasNext()) elements.add(iterator.next());
    java.util.Collections.reverse(elements);
    return hara.lang.data.Vector.Standard.from(null, elements.toArray());
  }

  Object mapValues(Object[] values) {
    if (values.length == 1) {
      Object function = values[0];
      return new VariadicBuiltin(
          "map",
          inputs -> {
            if (inputs.length == 0) {
              throw new HaraException("map transform expects at least one collection");
            }
            Object[] arguments = new Object[inputs.length + 1];
            arguments[0] = function;
            System.arraycopy(inputs, 0, arguments, 1, inputs.length);
            return transformLike(inputs[0], iterMap(arguments));
          });
    }
    if (values.length < 2) throw new HaraException("map expects a function and collections");
    return transformLike(values[1], iterMap(values));
  }

  Object materializeVector(Object values) {
    Iterator<?> iterator = (Iterator<?>) iterValue(values);
    ArrayList<Object> output = new ArrayList<>();
    while (iterator.hasNext()) output.add(iterator.next());
    return hara.lang.data.Vector.Standard.from(null, output.toArray());
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  Object transformLike(Object source, Object values) {
    Object unwrapped = HaraBox.unwrap(source);
    if (unwrapped instanceof hara.lang.data.Seq sequence) {
      Object output = seqValue(new Object[] {values});
      if (output instanceof hara.lang.data.Seq result && sequence.meta() != null) {
        return result.withMeta(sequence.meta());
      }
      return output;
    }
    if (unwrapped instanceof Iterator<?>) return values;
    return materializeVector(values);
  }

  Object partitionValues(Object[] values, boolean includePartial) {
    if (values.length == 1) {
      Object amount = values[0];
      return new UnaryBuiltin(
          includePartial ? "partition-all" : "partition",
          input -> {
            Object partitioned = iterPartition(new Object[] {amount, input}, includePartial);
            return transformLike(input, partitioned);
          });
    }
    requireMethodArity(includePartial ? "partition-all" : "partition", values, 2);
    return transformLike(values[1], iterPartition(values, includePartial));
  }

  Object filterValues(Object[] values) {
    if (values.length == 1) {
      Object predicate = values[0];
      return new UnaryBuiltin(
          "filter", input -> transformLike(input, iterFilter(new Object[] {predicate, input})));
    }
    requireMethodArity("filter", values, 2);
    return transformLike(values[1], iterFilter(values));
  }

  Object takeValues(Object[] values) {
    if (values.length == 1) {
      Object amount = values[0];
      return new UnaryBuiltin(
          "take", input -> transformLike(input, iterTake(new Object[] {amount, input})));
    }
    requireMethodArity("take", values, 2);
    return transformLike(values[1], iterTake(values));
  }

  Object dropValues(Object[] values) {
    if (values.length == 1) {
      Object amount = values[0];
      return new UnaryBuiltin(
          "drop", input -> transformLike(input, iterDrop(new Object[] {amount, input})));
    }
    requireMethodArity("drop", values, 2);
    return transformLike(values[1], iterDrop(values));
  }

  Object removeValues(Object[] values) {
    requireMethodArity("remove", values, 2);
    Object predicate = values[0];
    Iterator<?> iterator = (Iterator<?>) iterValue(values[1]);
    ArrayList<Object> kept = new ArrayList<>();
    while (iterator.hasNext()) {
      Object item = iterator.next();
      Object result = invokeCallable(predicate, new Object[] {item});
      if (result == null || Boolean.FALSE.equals(result)) kept.add(item);
    }
    return hara.lang.data.Vector.Standard.from(null, kept.toArray());
  }

  private static String displayText(Object unwrapped) {
    if (unwrapped instanceof HaraCharacter character) {
      return character.text();
    }
    if (unwrapped instanceof Number) {
      return G.display(unwrapped);
    }
    if (unwrapped instanceof IDisplay) {
      return ((IDisplay) unwrapped).display();
    }
    if (unwrapped instanceof Iterator) {
      return "#<lazy-iterator>";
    }
    return String.valueOf(unwrapped);
  }

  private static String concatenateStrings(Object[] values) {
    StringBuilder result = new StringBuilder();
    for (Object value : values) {
      Object unwrapped = HaraBox.unwrap(value);
      if (unwrapped == null || unwrapped == HaraNull.SINGLETON) {
        continue;
      }
      result.append(displayText(unwrapped));
    }
    return result.toString();
  }

  private Object printValues(Object[] values, boolean newline) {
    String text;
    if (newline) {
      java.util.List<String> parts = new java.util.ArrayList<>(values.length);
      for (Object value : values) {
        Object unwrapped = HaraBox.unwrap(value);
        parts.add(unwrapped == null || unwrapped == HaraNull.SINGLETON
            ? "nil"
            : displayText(unwrapped));
      }
      text = String.join(" ", parts) + "\n";
    } else {
      text = concatenateStrings(values);
    }
    try {
      Deque<OutputStream> outputs = printerOutputs.get();
      OutputStream output = outputs.isEmpty() ? environment.out() : outputs.peekLast();
      output.write(text.getBytes(StandardCharsets.UTF_8));
      output.flush();
      return null;
    } catch (IOException error) {
      throw new HaraException("Printer output failed: " + error.getMessage());
    }
  }

  private Object exceptionSiteValue(hara.lang.base.Ex.Info.Site site) {
    if (site == null) return null;
    return hara.lang.data.Map.Standard.from(
        null,
        Keyword.create("namespace"), site.namespace(),
        Keyword.create("resource"), site.resource(),
        Keyword.create("line"), site.line(),
        Keyword.create("column"), site.column());
  }

  private static Keyword defaultExceptionClass(Keyword code) {
    if (!"hara".equals(code.getNamespace())) return null;
    String name = code.getName();
    return switch (name) {
      case "security",
          "timeout",
          "not-found",
          "conflict",
          "limit",
          "syntax",
          "io",
          "database",
          "dependency",
          "serialization",
          "argument",
          "state",
          "host" -> Keyword.create("ex.class", name);
      case "generic" -> Keyword.create("ex.class", "internal");
      default -> null;
    };
  }

  private static Keyword normalizeExceptionCode(Keyword code) {
    if (code.getNamespace() != null) return code;
    Keyword canonical = Keyword.create("hara", code.getName());
    if (defaultExceptionClass(canonical) != null) return canonical;
    throw new HaraException(
        "ex expects a registered standard keyword or namespaced keyword code");
  }

  private void installCoreBuiltins(HaraNamespace target) {
    target.define("type", new UnaryBuiltin("type", this::portableType));
    target.define("str", new VariadicBuiltin("str", HaraContext::concatenateStrings));
    target.define(
        "sha256",
        new UnaryBuiltin(
            "sha256",
            value -> {
              try {
                return HexFormat.of()
                    .formatHex(
                        MessageDigest.getInstance("SHA-256")
                            .digest(bytesValue(value, "sha256")));
              } catch (java.security.NoSuchAlgorithmException impossible) {
                throw new HaraException("SHA-256 is unavailable");
              }
            }));
    target.define("p", new VariadicBuiltin("p", values -> printValues(values, false)));
    target.define(
        "println", new VariadicBuiltin("println", values -> printValues(values, true)));
    target.define(
        "capture",
        new UnaryBuiltin(
            "Printer/capture",
            callable -> {
              ByteArrayOutputStream output = new ByteArrayOutputStream();
              Deque<OutputStream> outputs = printerOutputs.get();
              outputs.addLast(output);
              try {
                invokeCallable(callable, new Object[0]);
                return output.toString(StandardCharsets.UTF_8);
              } finally {
                outputs.removeLast();
                if (outputs.isEmpty()) printerOutputs.remove();
              }
            }));
    target.define(
        "list",
        new VariadicBuiltin(
            "list", values -> hara.lang.data.List.Standard.from(null, values)));
    target.define(
        "vector",
        new VariadicBuiltin(
            "vector", values -> hara.lang.data.Vector.Standard.from(null, values)));
    target.define(
        "pair",
        new VariadicBuiltin(
            "pair",
            values -> {
              requireMethodArity("pair", values, 2);
              return hara.kernel.builtin.BuiltinStruct.pair(
                  HaraBox.unwrap(values[0]), HaraBox.unwrap(values[1]));
            }));
    target.define(
        "tup",
        new VariadicBuiltin(
            "tup",
            values -> {
              if (values.length > 8) throw new HaraException("tuple expects at most 8 arguments");
              Object[] unwrapped = java.util.Arrays.stream(values).map(HaraBox::unwrap).toArray();
              return hara.kernel.builtin.BuiltinStruct.tuple(unwrapped);
            }));
    target.define(
        "hash",
        new UnaryBuiltin(
            "Base/hash",
            value -> G.hashCalc(Constant.HashType.RAPID, HaraBox.unwrap(value))));
    target.define(
        "hash-map",
        new VariadicBuiltin(
            "hash-map",
            values -> {
              if (values.length % 2 != 0) {
                throw new HaraException("hash-map expects an even number of arguments");
              }
              return hara.kernel.builtin.BuiltinStruct.hashMap(values);
            }));
    target.define(
        "hash-set",
        new VariadicBuiltin("hash-set", hara.kernel.builtin.BuiltinStruct::hashSet));
    target.define(
        "atom",
        new UnaryBuiltin(
            "atom", value -> new hara.lang.data.Atom.Standard<>(HaraBox.unwrap(value))));
    target.define(
        "pointer",
        new UnaryBuiltin(
            "pointer", value -> hara.lang.data.Pointer.fromDescriptor(HaraBox.unwrap(value))));
    target.define(
        "pr-str",
        new UnaryBuiltin(
            "pr-str",
            value -> hara.kernel.builtin.BuiltinUtil.prStr(HaraBox.unwrap(value))));
    target.define(
        "uuid?",
        new UnaryBuiltin("uuid?", value -> HaraBox.unwrap(value) instanceof java.util.UUID));
    target.define("uuid", new VariadicBuiltin("Base/uuid", this::uuidValue));
    target.define(
        "regexp?",
        new UnaryBuiltin(
            "regexp?", value -> HaraBox.unwrap(value) instanceof java.util.regex.Pattern));
    target.define("promise", new UnaryBuiltin("promise", this::promiseRun));
    target.define("bytes", new VariadicBuiltin("bytes", this::createBytes));
    target.define("array", new VariadicBuiltin("array", HaraArray::new));
    target.define("object", new VariadicBuiltin("object", HaraObject::new));
    HaraNamespace arr = namespace("std.native.Arr");
    for (String method : HaraNativeDeclarations.methods("Arr")) {
      arr.define(method, new VariadicBuiltin("std.native.Arr/" + method,
          values -> nativeMutableCall("Arr", method, values)));
    }
    HaraNamespace obj = namespace("std.native.Obj");
    for (String method : HaraNativeDeclarations.methods("Obj")) {
      obj.define(method, new VariadicBuiltin("std.native.Obj/" + method,
          values -> nativeMutableCall("Obj", method, values)));
    }
    HaraNamespace bits = namespace("std.native.Bits");
    bits.define("and", new VariadicBuiltin("std.native.Bits/and", values -> bitOperation("and", values)));
    bits.define("or", new VariadicBuiltin("std.native.Bits/or", values -> bitOperation("or", values)));
    bits.define("xor", new VariadicBuiltin("std.native.Bits/xor", values -> bitOperation("xor", values)));
    bits.define(
        "not",
        new UnaryBuiltin(
            "std.native.Bits/not",
            value -> Num.not(HaraNumericConversions.toInteger(value, "bit-not"))));
    bits.define(
        "shift-left", new VariadicBuiltin("std.native.Bits/shift-left", values -> bitShift(values, true)));
    bits.define(
        "shift-right",
        new VariadicBuiltin("std.native.Bits/shift-right", values -> bitShift(values, false)));
    HaraNamespace maths = namespace("std.native.Maths");
    maths.define("abs", new UnaryBuiltin("std.native.Maths/abs", HaraContext::numericAbs));
    maths.define("acos", mathUnary("std.native.Maths/acos", Math::acos));
    maths.define("acosh", mathUnary("std.native.Maths/acosh", HaraContext::acosh));
    maths.define("asin", mathUnary("std.native.Maths/asin", Math::asin));
    maths.define("asinh", mathUnary("std.native.Maths/asinh", HaraContext::asinh));
    maths.define("atan", mathUnary("std.native.Maths/atan", Math::atan));
    maths.define("atan2", new VariadicBuiltin("std.native.Maths/atan2", values -> mathBinary("atan2", values)));
    maths.define("atanh", mathUnary("std.native.Maths/atanh", HaraContext::atanh));
    maths.define("ceil", mathUnary("std.native.Maths/ceil", Math::ceil));
    maths.define("cos", mathUnary("std.native.Maths/cos", Math::cos));
    maths.define("cosh", mathUnary("std.native.Maths/cosh", Math::cosh));
    maths.define("exp", mathUnary("std.native.Maths/exp", Math::exp));
    maths.define("floor", mathUnary("std.native.Maths/floor", Math::floor));
    maths.define("pow", new VariadicBuiltin("std.native.Maths/pow", values -> mathBinary("pow", values)));
    maths.define("sin", mathUnary("std.native.Maths/sin", Math::sin));
    maths.define("sinh", mathUnary("std.native.Maths/sinh", Math::sinh));
    maths.define("sqrt", mathUnary("std.native.Maths/sqrt", Math::sqrt));
    maths.define("tan", mathUnary("std.native.Maths/tan", Math::tan));
    maths.define("tanh", mathUnary("std.native.Maths/tanh", Math::tanh));
    HaraNamespace host = namespace("std.native.Host");
    host.define("call", new VariadicBuiltin("std.native.Host/call", this::hostCall));
    host.define("describe", new VariadicBuiltin("std.native.Host/describe", this::hostDescribe));
    host.define(
        "capabilities",
        new VariadicBuiltin("std.native.Host/capabilities", this::hostCapabilities));
    host.define(
        "capability?",
        new VariadicBuiltin("std.native.Host/capability?", this::hostCapability));
  }

  private Keyword portableType(Object value) {
    Object raw = HaraBox.unwrap(value);
    if (raw instanceof HaraStruct struct) return namedTypeKeyword(struct.type().name());
    if (raw instanceof HaraMutable mutable) return namedTypeKeyword(mutable.type().name());

    String type;
    if (raw == null || raw == HaraNull.SINGLETON) type = "Nil";
    else if (NumUtils.isLongValue(raw)) type = "Long";
    else if (NumUtils.isBigIntegerValue(raw)) type = "BigInteger";
    else if (raw instanceof Float || raw instanceof Double) type = "Float";
    else if (raw instanceof HaraCharacter || raw instanceof Character) type = "Character";
    else if (raw instanceof java.util.UUID) type = "UUID";
    else if (raw instanceof java.util.regex.Pattern) type = "RegExp";
    else if (raw instanceof hara.lang.data.TaggedLiteral)
      type = Reduced.isReduced(raw) ? "Reduced" : "TaggedLiteral";
    else if (raw instanceof Boolean) type = "Boolean";
    else if (raw instanceof String) type = "String";
    else if (raw instanceof Keyword) type = "Keyword";
    else if (raw instanceof Symbol) type = "Symbol";
    else if (raw instanceof hara.lang.data.Pointer) type = "Pointer";
    else if (raw instanceof HaraMutableType) type = "MutableType";
    else if (raw instanceof HaraType) type = "StructType";
    else if (raw instanceof HaraProtocol) type = "Protocol";
    else if (raw instanceof HaraNativeType) type = "NativeType";
    else if (raw instanceof HaraSchemaType) type = "SchemaType";
    else if (raw instanceof HaraResult) type = "Result";
    else if (raw instanceof hara.lang.protocol.IExInfo || raw instanceof HaraException) type = "Exception";
    else if (raw instanceof hara.lang.protocol.IStream) type = "Stream";
    else if (raw instanceof hara.lang.protocol.ICoroutine) type = "Coroutine";
    else if (raw instanceof IPromise) type = "Promise";
    else if (raw instanceof hara.lang.data.Atom.Struct<?, ?>) type = "Atom";
    else if (raw instanceof byte[]) type = "ByteBuffer";
    else if (raw instanceof HaraArray) type = "Array";
    else if (raw instanceof HaraObject) type = "Object";
    else if (raw instanceof hara.lang.data.types.ObjMutable) type = "MutableCollection";
    else if (raw instanceof hara.lang.data.List<?>) type = "List";
    else if (raw instanceof hara.lang.data.Cons<?>) type = "Cons";
    else if (raw instanceof hara.lang.data.Seq<?>) type = "Seq";
    else if (raw instanceof hara.lang.data.Queue<?>) type = "Queue";
    else if (raw instanceof hara.lang.data.Deque<?>) type = "Deque";
    else if (raw instanceof hara.lang.data.MapEntry<?, ?>) type = "MapEntry";
    else if (hara.lang.data.Tuple.isCompact(raw)) type = "Vector";
    else if (raw instanceof hara.lang.data.Vector<?>) type = "Vector";
    else if (raw instanceof hara.lang.data.OrderedMap<?, ?>) type = "OrderedMap";
    else if (raw instanceof hara.lang.data.PriorityMap<?, ?>) type = "PriorityMap";
    else if (raw instanceof hara.lang.data.SortedMap<?, ?>) type = "SortedMap";
    else if (raw instanceof hara.lang.data.Trie<?>) type = "Trie";
    else if (raw instanceof hara.lang.data.Map<?, ?>) type = "HashMap";
    else if (raw instanceof hara.lang.data.OrderedSet<?>) type = "OrderedSet";
    else if (raw instanceof hara.lang.data.SortedSet<?>) type = "SortedSet";
    else if (raw instanceof hara.lang.data.Set<?>) type = "HashSet";
    else if (raw instanceof Iterator<?>) type = "Iterator";
    else if (raw instanceof HaraVar || raw instanceof hara.kernel.base.Var) type = "Var";
    else if (raw instanceof HaraNamespace) type = "Namespace";
    else if (raw instanceof HtaHandle) type = "Extension";
    else if (raw instanceof HaraFunction
        || raw instanceof HaraMultiFunction
        || raw instanceof HaraBuiltinFunction
        || raw instanceof HbcMachine.HbcClosure
        || raw instanceof HbcMachine.HbcMultiArity
        || raw instanceof HbcMachine.HbcNativeCallable
        || raw instanceof IFn) type = "Function";
    else type = "Object";
    return Keyword.create("std.native." + type);
  }

  private Object uuidValue(Object[] values) {
    try {
      return switch (values.length) {
        case 0 -> java.util.UUID.randomUUID();
        case 1 -> {
          Object input = HaraBox.unwrap(values[0]);
          if (input instanceof String string) yield java.util.UUID.fromString(string);
          if (input instanceof byte[] bytes) yield java.util.UUID.nameUUIDFromBytes(bytes);
          if (input instanceof Keyword keyword) {
            String fullName =
                keyword.getNamespace() == null
                    ? keyword.getName()
                    : keyword.getNamespace() + "/" + keyword.getName();
            yield new java.util.UUID(fullName.hashCode(), keyword.getName().hashCode());
          }
          throw new HaraException("uuid expects a string, bytes, or keyword");
        }
        case 2 ->
            new java.util.UUID(
                HaraNumericConversions.toLong(values[0], "uuid"),
                HaraNumericConversions.toLong(values[1], "uuid"));
        default -> throw new HaraException("uuid expects zero, one, or two arguments");
      };
    } catch (IllegalArgumentException error) {
      throw new HaraException("uuid expects a valid UUID string");
    }
  }

  private Keyword namedTypeKeyword(String qualifiedName) {
    return Keyword.create(qualifiedName.replace('/', '.'));
  }

  private HaraSchemaType compileSchema(Object value, HaraVar origin) {
    Object raw = HaraBox.unwrap(value);
    if (raw instanceof HaraSchemaType schema) return schema;
    if (raw instanceof HaraVar variable) return compileSchema(variable.deref(), variable);
    HalcSchema.Type ast = HalcSchema.normalize(raw);
    if (ast instanceof HalcSchema.Unknown) {
      throw new HaraException("schema expects schema data");
    }
    return new HaraSchemaType(raw, ast, origin);
  }

  private static HaraSchemaType requireSchema(Object value, String operation) {
    Object raw = HaraBox.unwrap(value);
    if (raw instanceof HaraSchemaType schema) return schema;
    throw new HaraException(operation + " expects a schema");
  }

  @SuppressWarnings("unchecked")
  private Object schemaContract(HaraVar variable) {
    return variable.schemaContract();
  }

  @SuppressWarnings("unchecked")
  private HaraSchemaType declaredSchemaContract(HaraVar variable) {
    if (!(variable.meta() instanceof hara.lang.protocol.ILookup<?, ?> metadata)) return null;
    Object declared = ((hara.lang.protocol.ILookup<Object, Object>) metadata)
        .lookup(Keyword.create("schema"));
    if (declared == null) return null;
    if (declared instanceof hara.lang.data.List<?> reference
        && reference.count() == 2
        && reference.nth(0) instanceof Symbol operator
        && operator.getNamespace() == null
        && "var".equals(operator.getName())
        && reference.nth(1) instanceof Symbol target) {
      HaraVar schemaVar = resolve(target);
      if (schemaVar == null) {
        throw new HaraException("schema Var does not exist: " + target.display());
      }
      Object schemaValue = schemaVar.deref();
      if (schemaValue == null) {
        evaluationRuntime.deferSchemaContract(schemaVar, variable);
        return null;
      }
      try {
        return compileSchema(schemaValue, schemaVar);
      } catch (HaraException error) {
        throw new HaraException(
            error.getMessage() + " (schema Var " + schemaVar.display() + ": " + schemaValue + ")");
      }
    }
    try {
      return compileSchema(declared, variable);
    } catch (HaraException error) {
      throw new HaraException(
          error.getMessage() + " (declared " + declared.getClass().getName() + ": " + declared + ")");
    }
  }

  private void refreshSchemaContract(HaraVar variable) {
    try {
      variable.setSchemaContract(declaredSchemaContract(variable));
    } catch (HaraException error) {
      throw new HaraException(variable.display() + ": " + error.getMessage());
    }
  }

  private void resolvePendingSchemaContracts(HaraVar schemaVariable) {
    if (schemaVariable.deref() == null) return;
    ArrayList<HaraVar> dependents = evaluationRuntime.takePendingSchemaContracts(schemaVariable);
    if (dependents == null) return;
    for (HaraVar dependent : dependents) refreshSchemaContract(dependent);
  }

  private static String schemaKind(HalcSchema.Type ast) {
    if (ast instanceof HalcSchema.Properties decorated) return schemaKind(decorated.schema());
    if (ast instanceof HalcSchema.Primitive) return "primitive";
    if (ast instanceof HalcSchema.Reference) return "reference";
    if (ast instanceof HalcSchema.Union) return "union";
    if (ast instanceof HalcSchema.VectorType) return "vector";
    if (ast instanceof HalcSchema.SetType) return "set";
    if (ast instanceof HalcSchema.Tuple) return "tuple";
    if (ast instanceof HalcSchema.MapType) return "map";
    if (ast instanceof HalcSchema.StructType) return "struct";
    if (ast instanceof HalcSchema.FunctionType function && function.arities().size() == 1)
      return "fn";
    if (ast instanceof HalcSchema.FunctionType) return "function";
    if (ast instanceof HalcSchema.EnumType) return "enum";
    if (ast instanceof HalcSchema.Extension) return "extension";
    return "unknown";
  }

  private static Object schemaAstMap(Object... entries) {
    return hara.lang.data.Map.Standard.from(null, entries);
  }

  private static Object schemaAstVector(java.util.Collection<?> values) {
    return BuiltinStruct.vector(values.toArray());
  }

  private static Object schemaFunctionAst(HalcSchema.Function function) {
    ArrayList<Object> fixed = new ArrayList<>();
    function.fixed().forEach(value -> fixed.add(schemaAst(value)));
    return schemaAstMap(
        Keyword.create("kind"), Keyword.create("fn"),
        Keyword.create("inputs"),
            schemaAstMap(
                Keyword.create("fixed"), schemaAstVector(fixed),
                Keyword.create("rest"),
                    function.rest() == null ? null : schemaAst(function.rest())),
        Keyword.create("output"), schemaAst(function.output()));
  }

  private static Object schemaAst(HalcSchema.Type ast) {
    if (ast instanceof HalcSchema.Properties decorated) {
      Object base = schemaAst(decorated.schema());
      if (!(base instanceof hara.lang.protocol.IMapType<?, ?> map))
        throw new HaraException("canonical schema AST must be a map");
      ArrayList<Object> entries = new ArrayList<>();
      for (Object item : map) {
        Map.Entry<?, ?> entry = (Map.Entry<?, ?>) item;
        entries.add(entry.getKey());
        entries.add(entry.getValue());
      }
      entries.add(Keyword.create("properties"));
      entries.add(decorated.properties());
      return schemaAstMap(entries.toArray());
    }
    if (ast instanceof HalcSchema.Primitive primitive) {
      return schemaAstMap(
          Keyword.create("kind"), Keyword.create("primitive"),
          Keyword.create("name"), Keyword.create(primitive.name()));
    }
    if (ast instanceof HalcSchema.Reference reference) {
      return schemaAstMap(
          Keyword.create("kind"), Keyword.create("reference"),
          Keyword.create("name"), Symbol.create(reference.name()));
    }
    if (ast instanceof HalcSchema.Union union) {
      ArrayList<Object> types = new ArrayList<>();
      union.types().forEach(value -> types.add(schemaAst(value)));
      return schemaAstMap(
          Keyword.create("kind"), Keyword.create("union"),
          Keyword.create("types"), schemaAstVector(types));
    }
    if (ast instanceof HalcSchema.VectorType vector) {
      return schemaAstMap(
          Keyword.create("kind"), Keyword.create("vector"),
          Keyword.create("item"), schemaAst(vector.item()));
    }
    if (ast instanceof HalcSchema.SetType set) {
      return schemaAstMap(
          Keyword.create("kind"), Keyword.create("set"),
          Keyword.create("item"), schemaAst(set.item()));
    }
    if (ast instanceof HalcSchema.Tuple tuple) {
      ArrayList<Object> items = new ArrayList<>();
      tuple.items().forEach(value -> items.add(schemaAst(value)));
      return schemaAstMap(
          Keyword.create("kind"), Keyword.create("tuple"),
          Keyword.create("items"), schemaAstVector(items));
    }
    if (ast instanceof HalcSchema.MapType map) {
      ArrayList<Object> fields = new ArrayList<>();
      map.fields().forEach(
          field -> {
            if (field.properties() == null) {
              fields.add(
                  schemaAstMap(
                      Keyword.create("name"), field.name(),
                      Keyword.create("type"), schemaAst(field.type())));
            } else {
              fields.add(
                  schemaAstMap(
                      Keyword.create("name"), field.name(),
                      Keyword.create("properties"), field.properties(),
                      Keyword.create("type"), schemaAst(field.type())));
            }
          });
      return schemaAstMap(
          Keyword.create("kind"), Keyword.create("map"),
          Keyword.create("fields"), schemaAstVector(fields));
    }
    if (ast instanceof HalcSchema.StructType struct) {
      ArrayList<Object> fields = new ArrayList<>();
      struct.fields().forEach(
          field -> {
            if (field.properties() == null) {
              fields.add(
                  schemaAstMap(
                      Keyword.create("name"), field.name(),
                      Keyword.create("type"), schemaAst(field.type())));
            } else {
              fields.add(
                  schemaAstMap(
                      Keyword.create("name"), field.name(),
                      Keyword.create("properties"), field.properties(),
                      Keyword.create("type"), schemaAst(field.type())));
            }
          });
      return schemaAstMap(
          Keyword.create("kind"), Keyword.create("struct"),
          Keyword.create("name"), Symbol.create(struct.name()),
          Keyword.create("mutable?"), struct.mutable(),
          Keyword.create("fields"), schemaAstVector(fields));
    }
    if (ast instanceof HalcSchema.FunctionType function) {
      if (function.arities().size() == 1) return schemaFunctionAst(function.arities().get(0));
      ArrayList<Object> arities = new ArrayList<>();
      function.arities().forEach(value -> arities.add(schemaFunctionAst(value)));
      return schemaAstMap(
          Keyword.create("kind"), Keyword.create("function"),
          Keyword.create("arities"), schemaAstVector(arities));
    }
    if (ast instanceof HalcSchema.EnumType values) {
      return schemaAstMap(
          Keyword.create("kind"), Keyword.create("enum"),
          Keyword.create("values"), schemaAstVector(values.values()));
    }
    if (ast instanceof HalcSchema.Extension extension) {
      ArrayList<Object> surface = new ArrayList<>();
      surface.add(Keyword.create(extension.head()));
      surface.addAll(extension.arguments());
      return schemaAstMap(
          Keyword.create("kind"), Keyword.create("extension"),
          Keyword.create("head"), Keyword.create(extension.head()),
          Keyword.create("arguments"), schemaAstVector(extension.arguments()),
          Keyword.create("surface"), schemaAstVector(surface));
    }
    Object surface = ((HalcSchema.Unknown) ast).surface();
    return schemaAstMap(
        Keyword.create("kind"), Keyword.create("unknown"),
        Keyword.create("surface"), surface);
  }

  private Object hostCall(Object[] values) {
    requireNativeCapability("Host", "call", "host-call");
    if (values.length != 3
        || !(HaraBox.unwrap(values[0]) instanceof String service)
        || !(HaraBox.unwrap(values[1]) instanceof String method)
        || !(HaraBox.unwrap(values[2]) instanceof hara.lang.protocol.ILinearType<?> arguments)) {
      throw new HaraException(
          "std.native.Host/call expects service, method, and an argument vector");
    }
    if ("host".equals(service)) {
      return switch (method) {
        case "describe" -> hostDescribe(new Object[0]);
        case "capabilities" -> hostCapabilities(new Object[0]);
        case "capability?" ->
            hostCapability(
                java.util.stream.StreamSupport.stream(arguments.spliterator(), false).toArray());
        default ->
            rejectedHostPromise(
                "host/method-unavailable",
                "Host method is unavailable: " + service + "/" + method);
      };
    }
    return rejectedHostPromise(
        "host/method-unavailable",
        "Host method is unavailable: " + service + "/" + method);
  }

  private Object hostDescribe(Object[] values) {
    requireNativeCapability("Host", "describe", "host-call");
    if (values.length != 0) throw new HaraException("std.native.Host/describe expects no arguments");
    Object capabilities = hostCapabilityVector();
    Object descriptor =
        hara.lang.data.Map.Standard.from(
            null,
            Keyword.create("host/version"),
            "hara.host.v1",
            Keyword.create("host/available"),
            capabilities,
            Keyword.create("host/granted"),
            capabilities,
            Keyword.create("host/limits"),
            hara.lang.data.Map.Standard.from(null));
    return new HaraPromise(CompletableFuture.completedFuture(descriptor));
  }

  private Object hostCapabilities(Object[] values) {
    requireNativeCapability("Host", "capabilities", "host-call");
    if (values.length != 0) {
      throw new HaraException("std.native.Host/capabilities expects no arguments");
    }
    return new HaraPromise(CompletableFuture.completedFuture(hostCapabilityVector()));
  }

  private Object hostCapability(Object[] values) {
    requireNativeCapability("Host", "capability?", "host-call");
    if (values.length != 1) {
      throw new HaraException("std.native.Host/capability? expects one capability");
    }
    Object value = HaraBox.unwrap(values[0]);
    String capability =
        value instanceof Keyword keyword
            ? keyword.getName()
            : value instanceof String string ? string.replaceFirst("^:", "") : null;
    if (capability == null) {
      throw new HaraException("std.native.Host/capability? expects a keyword or string");
    }
    boolean granted =
        java.util.Arrays.asList(hostCapabilityNames()).contains(capability);
    return new HaraPromise(CompletableFuture.completedFuture(granted));
  }

  private Object hostCapabilityVector() {
    return BuiltinStruct.vector(hostCapabilityNames());
  }

  private Object[] hostCapabilityNames() {
    java.util.List<Object> capabilities = new ArrayList<>();
    if (filesystemRuntime != null) capabilities.add("filesystem");
    if (environment.isSocketIOAllowed()) capabilities.add("network/socket");
    if (environment.isCreateProcessAllowed()) capabilities.add("process");
    return capabilities.toArray();
  }

  private Object rejectedHostPromise(String code, String message) {
    CompletableFuture<Object> future = new CompletableFuture<>();
    future.completeExceptionally(
        new hara.lang.base.Ex.Info(
            message,
            hara.lang.data.Map.Standard.from(
                null,
                Keyword.create("ex", "code"),
                keyword(code),
                Keyword.create("ex", "class"),
                Keyword.create("ex.class", "host"))));
    return new HaraPromise(future);
  }

  private Object nativeTestEvents(Object[] values) {
    if (values.length != 0) {
      throw new HaraException("std.native.Test/events expects no arguments");
    }
    return BuiltinStruct.vector(
        new Object[] {
          Keyword.create("test", "run-started"),
          Keyword.create("test", "fact-started"),
          Keyword.create("test", "fact-completed"),
          Keyword.create("test", "run-completed")
        });
  }

  private Object nativeTestCatalog(Object[] values) {
    if (values.length != 0) {
      throw new HaraException("std.native.Test/catalog expects no arguments");
    }
    return hara.lang.data.Map.Standard.from(
        null,
        Keyword.create("runners"),
        BuiltinStruct.vector(
            new Object[] {Keyword.create("code.test"), Keyword.create("native")}),
        Keyword.create("default"),
        Keyword.create("code.test"),
        Keyword.create("runner"),
        testRunner,
        Keyword.create("context"),
        Keyword.create("test"),
        Keyword.create("events"),
        nativeTestEvents(new Object[0]));
  }

  private static Keyword runtimeTestRunner(String value) {
    if ("code.test".equals(value) || "native".equals(value)) {
      return Keyword.create(value);
    }
    throw new HaraException("runtime test runner must be code.test or native");
  }

  private Object nativeTestConfig(Object[] values) {
    if (values.length > 1) {
      throw new HaraException("std.native.Test/config expects optional options");
    }
    Object options = values.length == 0 ? hara.lang.data.Map.Standard.EMPTY : HaraBox.unwrap(values[0]);
    if (!(options instanceof IMapType<?, ?>)) {
      throw new HaraException("std.native.Test/config options must be a map");
    }
    if (hasMapKey((IMapType<?, ?>) options, Keyword.create("runner"))) {
      throw new HaraException("std.native.Test/config runner is owned by the runtime");
    }
    return hara.lang.data.Map.Standard.from(
        null,
        Keyword.create("runner"),
        testRunner,
        Keyword.create("options"),
        options);
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  private static boolean hasMapKey(IMapType<?, ?> map, Object key) {
    return ((IMapType) map).find(key) != null;
  }

  private Object nativeTestContext(Object[] values) {
    if (values.length > 1) {
      throw new HaraException("std.native.Test/context expects an optional config");
    }
    Object config = values.length == 0 ? nativeTestConfig(new Object[0]) : HaraBox.unwrap(values[0]);
    if (!(config instanceof IMapType<?, ?> map)) {
      throw new HaraException("std.native.Test/context expects a Test/config map");
    }
    if (!testRunner.equals(lookupValue(map, Keyword.create("runner")))) {
      throw new HaraException(
          "std.native.Test/context config runner does not match the runtime");
    }
    java.util.Map<Object, Object> fields = new LinkedHashMap<>();
    fields.put(Keyword.create("id"), Keyword.create("test"));
    fields.put(Keyword.create("config"), config);
    return new hara.lang.data.Pointer(Keyword.create("test"), fields);
  }

  private Object nativeTestContext(Object name, Object actual, Object expected, Object failures) {
    return hara.lang.data.Map.Standard.from(
        null,
        Keyword.create("test"),
        hara.lang.data.Map.Standard.from(
            null,
            Keyword.create("name"), name,
            Keyword.create("actual"), actual,
            Keyword.create("expected"), expected),
        Keyword.create("failures"), failures);
  }

  private Object nativeTestFailure(Object actual, Object expected) {
    return hara.lang.data.Map.Standard.from(
        null,
        keyword("failure/code"), keyword("test/not-equal"),
        keyword("failure/path"), BuiltinStruct.vector(new Object[0]),
        keyword("failure/in"), BuiltinStruct.vector(new Object[0]),
        keyword("failure/actual"), actual,
        keyword("failure/expected"), expected,
        keyword("failure/message"), "values are not equal",
        keyword("failure/context"), hara.lang.data.Map.Standard.EMPTY,
        keyword("failure/children"), BuiltinStruct.vector(new Object[0]));
  }

  private Object nativeTestCompare(Object[] values) {
    if (values.length != 2) {
      throw new HaraException("std.native.Test/compare expects actual and expected");
    }
    Object actual = HaraBox.unwrap(values[0]);
    Object expected = HaraBox.unwrap(values[1]);
    boolean pass = Eq.eq(actual, expected);
    Object failures = pass
        ? BuiltinStruct.vector(new Object[0])
        : BuiltinStruct.vector(new Object[] {nativeTestFailure(actual, expected)});
    return HaraResult.success(pass, nativeTestContext(null, actual, expected, failures));
  }

  private Object nativeTestResult(Object[] values) {
    if (values.length != 4) {
      throw new HaraException(
          "std.native.Test/result expects name, actual, expected, and comparison Result");
    }
    Object name = HaraBox.unwrap(values[0]);
    Object actual = HaraBox.unwrap(values[1]);
    Object expected = HaraBox.unwrap(values[2]);
    if (!(HaraBox.unwrap(values[3]) instanceof HaraResult comparison)) {
      throw new HaraException("std.native.Test/result expects a comparison Result");
    }
    Object failures = comparison.context().lookup(Keyword.create("failures"), BuiltinStruct.vector(new Object[0]));
    return comparison.withContext(nativeTestContext(name, actual, expected, failures));
  }

  private Object nativeTestError(Object name, Object actual, Object expected, String error) {
    return HaraResult.error(
        new HaraException(error),
        nativeTestContext(name, actual, expected, BuiltinStruct.vector(new Object[0])));
  }

  @SuppressWarnings("unchecked")
  private Object nativeTestCheckedResult(Object name, Object metadata, Object rawResult) {
    Object resultValue = HaraBox.unwrap(rawResult);
    if (!(resultValue instanceof HaraResult result)) {
      return nativeTestError(name, null, null, "Test/run check function must return a Result");
    }
    Object rawTest = result.context().lookup(Keyword.create("test"), hara.lang.data.Map.Standard.EMPTY);
    IMapType<Object, Object> test = rawTest instanceof IMapType<?, ?>
        ? (IMapType<Object, Object>) rawTest
        : hara.lang.data.Map.Standard.EMPTY;
    test = (IMapType<Object, Object>) test.assoc(Keyword.create("name"), name);
    IMapType<Object, Object> context = result.context();
    context = (IMapType<Object, Object>) context.assoc(Keyword.create("test"), test);
    if (metadata != null) {
      context = (IMapType<Object, Object>) context.assoc(Keyword.create("meta"), metadata);
    }

    return result.withContext(context);
  }

  private Object nativeTestRun(Object[] values) {
    if (values.length < 1 || values.length > 3) {
      throw new HaraException(
          "std.native.Test/run expects cases, an optional check function, and an optional lifecycle map");
    }
    Object rawCases = HaraBox.unwrap(values[0]);
    boolean vectorCases = rawCases instanceof hara.lang.data.Vector<?>
        || rawCases.getClass().getName().startsWith("hara.lang.data.Tuple$");
    if (!vectorCases) {
      throw new HaraException("std.native.Test/run expects one vector of test cases");
    }
    ILinearType<?> cases = (ILinearType<?>) rawCases;
    Object second = values.length >= 2 ? HaraBox.unwrap(values[1]) : null;
    Object lifecycle = values.length == 3 ? HaraBox.unwrap(values[2])
        : second instanceof IMapType<?, ?> ? second : null;
    Object checkFunction = lifecycle == second ? null : second;
    if (lifecycle != null && !(lifecycle instanceof IMapType<?, ?>)) {
      throw new HaraException("std.native.Test/run lifecycle must be a map");
    }
    boolean setupOk = nativeTestLifecycle(lifecycle, "setup");
    int index = 0;
    if (setupOk) for (Object rawCase : cases) {
      index += 1;
      Object fallbackName = "invalid case " + index;
      if (!(HaraBox.unwrap(rawCase) instanceof IMapType<?, ?> testCase)) {
        nativeTestResults.add(nativeTestError(fallbackName, null, null, "Test/run case must be a map"));
        continue;
      }
      Object name = hasMapKey(testCase, Keyword.create("name"))
          ? lookupValue(testCase, Keyword.create("name")) : fallbackName;
      boolean hasExpected = hasMapKey(testCase, Keyword.create("expected"));
      Object expected = hasExpected ? lookupValue(testCase, Keyword.create("expected")) : null;
      Object metadata = hasMapKey(testCase, Keyword.create("meta"))
          ? lookupValue(testCase, Keyword.create("meta")) : null;
      boolean hasTest = hasMapKey(testCase, Keyword.create("test"));
      if (!hasTest) {
        nativeTestResults.add(nativeTestError(name, null, expected, "Test/run case requires :test"));
      } else if (!hasExpected) {
        nativeTestResults.add(nativeTestError(name, null, null, "Test/run case requires :expected"));
      } else {
        try {
          Object testFunction = lookupValue(testCase, Keyword.create("test"));
          if (checkFunction == null) {
            Object actual = nativeTestAwait(invokeCallable(testFunction, new Object[0]));
            Object comparison = nativeTestCompare(new Object[] {actual, expected});
            nativeTestResults.add(nativeTestResult(new Object[] {name, actual, expected, comparison}));
          } else {
            Object checked = invokeCallable(checkFunction, new Object[] {testFunction, expected});
            nativeTestResults.add(nativeTestCheckedResult(name, metadata, checked));
          }
        } catch (RuntimeException error) {
          String message = error.getMessage() == null ? error.getClass().getSimpleName() : error.getMessage();
          nativeTestResults.add(nativeTestError(name, null, expected, message));
        }
      }
    }
    nativeTestLifecycle(lifecycle, "teardown");
    return hara.lang.data.Vector.Standard.from(null, nativeTestResults.toArray());
  }

  @SuppressWarnings("unchecked")
  private boolean nativeTestLifecycle(Object lifecycle, String phase) {
    if (!(lifecycle instanceof IMapType<?, ?> lifecycleMap)
        || !hasMapKey(lifecycleMap, Keyword.create(phase))) {
      return true;
    }
    try {
      Object function = lookupValue(lifecycleMap, Keyword.create(phase));
      nativeTestAwait(invokeCallable(function, new Object[0]));
      return true;
    } catch (RuntimeException error) {
      String message = error.getMessage() == null ? error.getClass().getSimpleName() : error.getMessage();
      HaraResult result = (HaraResult) nativeTestError("test " + phase, null, null, message);
      nativeTestResults.add(result.withContext(
          hara.lang.data.Map.Standard.from(null, Keyword.create("phase"), Keyword.create(phase))));
      return false;
    }
  }

  private Object nativeTestAwait(Object value) {
    Object input = HaraBox.unwrap(value);
    return input instanceof HaraPromise promise ? promise.deref() : input;
  }

  private Object nativeTestPassed(Object[] values) {
    if (values.length != 1 || !(HaraBox.unwrap(values[0]) instanceof HaraResult result)) {
      throw new HaraException("std.native.Test/passed? expects a Result");
    }
    return result.isSuccess() && Boolean.TRUE.equals(result.data());
  }

  private HaraResult nativeTestRequireResult(Object value, String operation) {
    if (HaraBox.unwrap(value) instanceof HaraResult result) return result;
    throw new HaraException("std.native.Test/" + operation + " expects a Result");
  }

  private Object nativeTestDetail(HaraResult result, String key) {
    Object test = result.context().lookup(Keyword.create("test"), null);
    return test instanceof IMapType<?, ?> map ? lookupValue(map, Keyword.create(key)) : null;
  }

  private Object nativeTestFailures(HaraResult result) {
    Object failures = result.context().lookup(Keyword.create("failures"), null);
    return failures instanceof ILinearType<?> linear
        ? BuiltinStruct.vector(java.util.stream.StreamSupport.stream(linear.spliterator(), false).toArray())
        : BuiltinStruct.vector(new Object[0]);
  }

  private boolean nativeTestFailureShape(Object rawValue) {
    Object value = HaraBox.unwrap(rawValue);
    if (!(value instanceof IMapType<?, ?> map)) return false;
    Object rawChildren = lookupValue(map, keyword("failure/children"));
    if (!(rawChildren instanceof ILinearType<?> children)) return false;
    for (Object child : children) if (!nativeTestFailureShape(child)) return false;
    return lookupValue(map, keyword("failure/code")) instanceof Keyword
        && lookupValue(map, keyword("failure/path")) instanceof ILinearType<?>
        && lookupValue(map, keyword("failure/in")) instanceof ILinearType<?>
        && hasMapKey(map, keyword("failure/actual"))
        && hasMapKey(map, keyword("failure/expected"))
        && lookupValue(map, keyword("failure/message")) instanceof String
        && lookupValue(map, keyword("failure/context")) instanceof IMapType<?, ?>;
  }

  private void nativeTestFailureLeaves(Object rawFailure, java.util.List<Object> leaves) {
    Object failure = HaraBox.unwrap(rawFailure);
    if (!nativeTestFailureShape(failure)) return;
    IMapType<?, ?> map = (IMapType<?, ?>) failure;
    ILinearType<?> children =
        (ILinearType<?>) lookupValue(map, keyword("failure/children"));
    if (children.count() == 0) {
      leaves.add(failure);
    } else {
      for (Object child : children) nativeTestFailureLeaves(child, leaves);
    }
  }

  private Object nativeTestInspect(Object[] values, String operation) {
    if (values.length != 1) {
      throw new HaraException("std.native.Test/" + operation + " expects one Result");
    }
    HaraResult result = nativeTestRequireResult(values[0], operation);
    if ("actual".equals(operation)) return nativeTestDetail(result, "actual");
    if ("expected".equals(operation)) return nativeTestDetail(result, "expected");
    if ("failures".equals(operation)) return nativeTestFailures(result);
    java.util.List<Object> leaves = new ArrayList<>();
    for (Object failure : (hara.lang.data.Vector<?>) nativeTestFailures(result)) {
      nativeTestFailureLeaves(failure, leaves);
    }
    if ("failure-count".equals(operation)) return (long) leaves.size();
    return BuiltinStruct.vector(leaves.toArray());
  }

  private Object nativeTestFailureAt(Object[] values) {
    if (values.length != 2) {
      throw new HaraException("std.native.Test/failure expects a Result and index");
    }
    HaraResult result = nativeTestRequireResult(values[0], "failure");
    Object rawIndex = HaraBox.unwrap(values[1]);
    if (!(rawIndex instanceof Number number)
        || number.longValue() < 0
        || number.doubleValue() != (double) number.longValue()) {
      throw new HaraException("std.native.Test/failure index must be a non-negative integer");
    }
    Object sequence = nativeTestInspect(new Object[] {result}, "failure-seq");
    hara.lang.data.Vector<?> leaves = (hara.lang.data.Vector<?>) sequence;
    long index = number.longValue();
    return index < leaves.count() ? leaves.nth((int) index) : null;
  }

  private Object nativeTestFailurePredicate(Object[] values) {
    if (values.length != 1) {
      throw new HaraException("std.native.Test/failure? expects one value");
    }
    return nativeTestFailureShape(values[0]);
  }

  private static Keyword keyword(String value) {
    int separator = value.indexOf('/');
    return Keyword.create(
        value.substring(0, separator),
        value.substring(separator + 1));
  }

  private Object createBytes(Object[] values) {
    byte[] result = new byte[values.length];
    for (int i = 0; i < values.length; i++) {
      result[i] = (byte) byteNumber(values[i], "bytes");
    }
    return result;
  }

  private void installNativeLibraries() {
    NativeCrypto.install(this, "std.native.Crypto");
    HaraNativeWork.install(this);
    HaraNamespace document = namespace("std.native.Document");
    for (String method : HaraNativeDeclarations.methods("Document")) {
      document.define(
          method,
          new VariadicBuiltin(
              "std.native.Document/" + method,
              values -> NativeDocument.operation("std.native.Document/" + method, values)));
    }

    HaraNamespace test = namespace("std.native.Test");
    test.define("catalog", new VariadicBuiltin("std.native.Test/catalog", this::nativeTestCatalog));
    test.define("config", new VariadicBuiltin("std.native.Test/config", this::nativeTestConfig));
    test.define("context", new VariadicBuiltin("std.native.Test/context", this::nativeTestContext));
    test.define("events", new VariadicBuiltin("std.native.Test/events", this::nativeTestEvents));
    test.define("compare", new VariadicBuiltin("std.native.Test/compare", this::nativeTestCompare));
    test.define("run", new VariadicBuiltin("std.native.Test/run", this::nativeTestRun));
    test.define("result", new VariadicBuiltin("std.native.Test/result", this::nativeTestResult));
    test.define("passed?", new VariadicBuiltin("std.native.Test/passed?", this::nativeTestPassed));
    test.define("actual", new VariadicBuiltin("std.native.Test/actual", values -> nativeTestInspect(values, "actual")));
    test.define("expected", new VariadicBuiltin("std.native.Test/expected", values -> nativeTestInspect(values, "expected")));
    test.define("failures", new VariadicBuiltin("std.native.Test/failures", values -> nativeTestInspect(values, "failures")));
    test.define("failure-seq", new VariadicBuiltin("std.native.Test/failure-seq", values -> nativeTestInspect(values, "failure-seq")));
    test.define("failure-count", new VariadicBuiltin("std.native.Test/failure-count", values -> nativeTestInspect(values, "failure-count")));
    test.define("failure", new VariadicBuiltin("std.native.Test/failure", this::nativeTestFailureAt));
    test.define("failure?", new VariadicBuiltin("std.native.Test/failure?", this::nativeTestFailurePredicate));
    HaraNamespace algo = namespace("std.native.Algo");
    StdNativeAlgo.install(this, "std.native.Algo");
    algo.define("deque?", typePredicate("std.native.Algo/deque?", hara.lang.data.Deque.class));
    algo.define("ordered-map?", typePredicate("std.native.Algo/ordered-map?", hara.lang.data.OrderedMap.class));
    algo.define("ordered-set?", typePredicate("std.native.Algo/ordered-set?", hara.lang.data.OrderedSet.class));
    algo.define("priority-map?", typePredicate("std.native.Algo/priority-map?", hara.lang.data.PriorityMap.class));
    algo.define("queue?", typePredicate("std.native.Algo/queue?", hara.lang.data.Queue.class));
    algo.define("sorted-map?", typePredicate("std.native.Algo/sorted-map?", hara.lang.data.SortedMap.class));
    algo.define("sorted-set?", typePredicate("std.native.Algo/sorted-set?", hara.lang.data.SortedSet.class));
    algo.define("trie?", typePredicate("std.native.Algo/trie?", hara.lang.data.Trie.class));
    HaraNamespace regex = namespace("std.native.RegExp");
    regex.define(
        "compile",
        new UnaryBuiltin(
            "std.native.RegExp/compile",
            value -> {
              Object raw = HaraBox.unwrap(value);
              if (!(raw instanceof String pattern)) {
                throw new HaraException("std.native.RegExp/compile expects one string");
              }
              try {
                return java.util.regex.Pattern.compile(pattern);
              } catch (java.util.regex.PatternSyntaxException error) {
                throw new HaraException("invalid regexp: " + error.getDescription());
              }
            }));
    regex.define(
        "pattern",
        new UnaryBuiltin(
            "std.native.RegExp/pattern",
            value -> {
              Object raw = HaraBox.unwrap(value);
              if (!(raw instanceof java.util.regex.Pattern pattern)) {
                throw new HaraException("std.native.RegExp/pattern expects one regexp");
              }
              return pattern.pattern();
            }));
    regex.define(
        "find?",
        new VariadicBuiltin(
            "std.native.RegExp/find?",
            values -> {
              if (values.length != 2
                  || !(HaraBox.unwrap(values[0]) instanceof java.util.regex.Pattern pattern)
                  || !(HaraBox.unwrap(values[1]) instanceof String input)) {
                throw new HaraException(
                    "std.native.RegExp/find? expects a regexp and string");
              }
              return pattern.matcher(input).find();
            }));
    regex.define(
        "find",
        new VariadicBuiltin(
            "std.native.RegExp/find",
            values -> {
              if (values.length != 2
                  || !(HaraBox.unwrap(values[0]) instanceof java.util.regex.Pattern pattern)
                  || !(HaraBox.unwrap(values[1]) instanceof String input)) {
                throw new HaraException(
                    "std.native.RegExp/find expects a regexp and string");
              }
              java.util.regex.Matcher matcher = pattern.matcher(input);
              return matcher.find() ? matcher.group() : null;
            }));
    regex.define(
        "matches",
        new VariadicBuiltin(
            "std.native.RegExp/matches",
            values -> {
              if (values.length != 2
                  || !(HaraBox.unwrap(values[0]) instanceof java.util.regex.Pattern pattern)
                  || !(HaraBox.unwrap(values[1]) instanceof String input)) {
                throw new HaraException(
                    "std.native.RegExp/matches expects a regexp and string");
              }
              return pattern.matcher(input).matches();
            }));
    regex.define(
        "replace",
        new VariadicBuiltin(
            "std.native.RegExp/replace",
            values -> {
              if (values.length != 3
                  || !(HaraBox.unwrap(values[0]) instanceof java.util.regex.Pattern pattern)
                  || !(HaraBox.unwrap(values[1]) instanceof String input)
                  || !(HaraBox.unwrap(values[2]) instanceof String replacement)) {
                throw new HaraException(
                    "std.native.RegExp/replace expects a regexp, string, and replacement");
              }
              return pattern.matcher(input).replaceAll(replacement);
            }));
    regex.define(
        "split",
        new VariadicBuiltin(
            "std.native.RegExp/split",
            values -> {
              if (values.length != 2
                  || !(HaraBox.unwrap(values[0]) instanceof java.util.regex.Pattern pattern)
                  || !(HaraBox.unwrap(values[1]) instanceof String input)) {
                throw new HaraException(
                    "std.native.RegExp/split expects a regexp and string");
              }
              if (input.isEmpty()) return null;
              String[] parts = pattern.split(input, -1);
              return hara.lang.data.Vector.Standard.from(null, (Object[]) parts);
            }));
    HaraNamespace kernel = namespace("std.native.Kernel");
    for (String method : HaraNativeDeclarations.methods("Kernel")) {
      kernel.define(
          method,
          new VariadicBuiltin(
              "std.native.Kernel/" + method,
              values -> {
                if (!nativeCapabilityBoundary.granted("kernel")) {
                  throw HaraNativeCapabilityBoundary.denied("Kernel", method, "kernel");
                }
                throw new HaraException(
                    "std.native.Kernel/" + method + " requires a kernel embedding");
              }));
    }
    installNativeSandboxBuiltins();
    HaraNamespace jvm = namespace("hara.native.jvm");
    jvm.define(
        "set!",
        new VariadicBuiltin(
            "hara.native.jvm/set!",
            values -> {
              if (values.length != 3)
                throw new HaraException("hara.native.jvm/set! expects 3 arguments");
              String member;
              if (values[1] instanceof Symbol) member = ((Symbol) values[1]).getName();
              else if (values[1] instanceof Keyword) member = ((Keyword) values[1]).getName();
              else if (values[1] instanceof String) member = (String) values[1];
              else throw new HaraException("JVM member must be a symbol, keyword, or string");
              NativeFlavorProvider provider = nativeProvider();
              if (provider == null) {
                throw new HaraException("hara.native.jvm/set! requires an ns :flavor declaration");
              }
              return provider.writeMember(
                  HaraBox.unwrap(values[0]), member, HaraBox.unwrap(values[2]), nativeAccess());
            }));
    namespace("hara.native.jvm.reflect");
    namespace("hara.native.jvm.classpath");
    HaraNamespace edn = namespace("std.native.Edn");
    edn.define(
        "read-forms",
        new VariadicBuiltin("std.native.Edn/read-forms", this::readForms));
    edn.define(
        "read",
        new UnaryBuiltin(
            "std.native.Edn/read",
            value -> {
              Object unwrapped = HaraBox.unwrap(value);
              if (!(unwrapped instanceof String source)) {
                throw new HaraException("edn/read expects a string");
              }
              try {
                Object[] forms = HaraLanguage.readAll(source, "<edn>");
                if (forms.length != 1) {
                  throw new HaraException("edn/read expects exactly one value");
                }
                return forms[0];
              } catch (HaraException error) {
                throw error;
              } catch (RuntimeException error) {
                throw new HaraException("edn/read: " + error.getMessage());
              }
            }));
    edn.define(
        "write",
        new UnaryBuiltin(
            "std.native.Edn/write",
            value -> hara.kernel.builtin.BuiltinUtil.prStr(HaraBox.unwrap(value))));
    edn.define(
        "pretty",
        new VariadicBuiltin(
            "std.native.Edn/pretty",
            values -> {
              if (values.length != 2 || !(HaraBox.unwrap(values[1]) instanceof IMapType<?, ?>)) {
                throw new HaraException("edn/pretty expects a value and options map");
              }
              return hara.kernel.builtin.BuiltinUtil.prStr(HaraBox.unwrap(values[0]));
            }));
    installJvmNativeLibraries();
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  private void installNativeSandboxBuiltins() {
    HaraNamespace sandbox = namespace("std.native.Sandbox");
    if (sessionKernel == null) {
      for (String method : HaraNativeDeclarations.methods("Sandbox")) {
        sandbox.define(
            method,
            new VariadicBuiltin(
                "std.native.Sandbox/" + method,
                values -> {
                  if (!nativeCapabilityBoundary.granted("sandbox")) {
                    throw HaraNativeCapabilityBoundary.denied("Sandbox", method, "sandbox");
                  }
                  throw new HaraException(
                      "std.native.Sandbox/" + method + " requires a kernel embedding");
                }));
      }
      return;
    }
    sandbox.define(
        "open",
        new UnaryBuiltin(
            "std.native.Sandbox/open",
            value -> sandboxPromise(() -> sessionKernel.openSandbox(sandboxSpec(value)).value())));
    sandbox.define(
        "eval",
        new VariadicBuiltin(
            "std.native.Sandbox/eval",
            values -> {
              if (values.length != 2 || !(HaraBox.unwrap(values[1]) instanceof String text))
                throw new HaraException("std.native.Sandbox/eval expects source text");
              try {
                SandboxProvider.Pending<Object> pending =
                    sessionKernel.sandboxEval(sandboxId(values[0]), text);
                return sandboxPromise(pending);
              } catch (SandboxModel.SandboxException error) {
                return sandboxRejected(error);
              }
            }));
    sandbox.define(
        "call",
        new VariadicBuiltin(
            "std.native.Sandbox/call",
            values -> {
              Object callableValue = values.length > 1 ? HaraBox.unwrap(values[1]) : null;
              if (values.length != 3
                  || !(callableValue instanceof Symbol callableSymbol)
                  || callableSymbol.getNamespace() == null)
                throw new HaraException(
                    "std.native.Sandbox/call expects an id, qualified symbol, and argument vector");
              String callable = callableSymbol.display();
              Object rawArguments = HaraBox.unwrap(values[2]);
              if (!(rawArguments instanceof hara.lang.protocol.ILinearType<?> arguments))
                throw new HaraException("std.native.Sandbox/call expects an argument vector");
              ArrayList<Object> transferred = new ArrayList<>();
              for (int index = 0; index < arguments.count(); index++) {
                Object argument = arguments.nth(index);
                if (!sandboxPortable(argument)) {
                  throw new HaraException(
                      "std.native.Sandbox/call arguments must be immutable portable values");
                }
                transferred.add(argument);
              }
              try {
                SandboxProvider.Pending<Object> pending =
                    sessionKernel.sandboxCall(sandboxId(values[0]), callable, transferred);
                return sandboxPromise(pending);
              } catch (SandboxModel.SandboxException error) {
                return sandboxRejected(error);
              }
            }));
    sandbox.define(
        "cancel",
        new UnaryBuiltin(
            "std.native.Sandbox/cancel",
            id -> sandboxPromise(() -> sessionKernel.cancelSandbox(sandboxId(id)))));
    sandbox.define(
        "status",
        new UnaryBuiltin(
            "std.native.Sandbox/status",
            id -> sandboxStatusValue(sessionKernel.sandboxStatus(sandboxId(id)))));
    sandbox.define(
        "close",
        new UnaryBuiltin(
            "std.native.Sandbox/close",
            id ->
                sandboxPromise(
                    () -> {
                      sessionKernel.closeSandbox(sandboxId(id));
                      return null;
                    })));
  }

  private Object sandboxPromise(java.util.function.Supplier<Object> operation) {
    CompletableFuture<Object> future = new CompletableFuture<>();
    try {
      future.complete(operation.get());
    } catch (SandboxModel.SandboxException error) {
      future.completeExceptionally(sandboxException(error));
    }
    return promiseValue(future);
  }

  private Object sandboxRejected(SandboxModel.SandboxException error) {
    CompletableFuture<Object> future = new CompletableFuture<>();
    future.completeExceptionally(sandboxException(error));
    return promiseValue(future);
  }

  private Object sandboxPromise(SandboxProvider.Pending<Object> pending) {
    CompletableFuture<Object> structured =
        pending
            .future()
            .handle(
                (value, failure) -> {
                  if (failure == null) return value;
                  Throwable cause =
                      failure instanceof CompletionException && failure.getCause() != null
                          ? failure.getCause()
                          : failure;
                  if (cause instanceof SandboxModel.SandboxException sandboxError) {
                    throw new CompletionException(sandboxException(sandboxError));
                  }
                  if (cause instanceof RuntimeException runtime) throw runtime;
                  throw new CompletionException(cause);
                });
    return cancellablePromise(structured, pending::cancel);
  }

  private static hara.lang.base.Ex.Info sandboxException(
      SandboxModel.SandboxException error) {
    Object data =
        HaraPersistentValues.normalize(
            java.util.Map.of(
                Keyword.create("ex/code"),
                Keyword.create(sandboxErrorCode(error.code())),
                Keyword.create("ex/class"),
                Keyword.create(sandboxErrorClass(error.code()))));
    return new hara.lang.base.Ex.Info(
        error.getMessage(), (hara.lang.protocol.IMetadata) data, error);
  }

  private static String sandboxErrorCode(SandboxModel.ErrorCode code) {
    return switch (code) {
      case INVALID_SPEC -> "sandbox/invalid-spec";
      case PROVIDER_NOT_FOUND -> "sandbox/provider-not-found";
      case PROVIDER_UNAVAILABLE, UNSUPPORTED -> "sandbox/provider-unavailable";
      case BUNDLE_NOT_FOUND -> "sandbox/bundle-not-found";
      case BUNDLE_DIGEST_MISMATCH -> "sandbox/bundle-digest-mismatch";
      case MOUNT_NOT_FOUND -> "sandbox/mount-not-found";
      case NOT_FOUND, CLOSED -> "sandbox/not-found";
      case BUSY -> "sandbox/busy";
      case CANCELLED -> "sandbox/cancelled";
      case TIMEOUT -> "sandbox/timeout";
      case LIMIT_EXCEEDED -> "sandbox/limit-exceeded";
      case EVALUATION_FAILED -> "sandbox/evaluation-failed";
      case RESULT_NOT_TRANSFERABLE -> "sandbox/result-not-transferable";
      case TRANSPORT_FAILED -> "sandbox/transport-failed";
      case PROVIDER_FAILED -> "sandbox/provider-failed";
    };
  }

  private static String sandboxErrorClass(SandboxModel.ErrorCode code) {
    return switch (code) {
      case INVALID_SPEC -> "ex.class/argument";
      case PROVIDER_NOT_FOUND, BUNDLE_NOT_FOUND, MOUNT_NOT_FOUND, NOT_FOUND, CLOSED ->
          "ex.class/not-found";
      case PROVIDER_UNAVAILABLE, UNSUPPORTED -> "ex.class/dependency";
      case BUNDLE_DIGEST_MISMATCH, RESULT_NOT_TRANSFERABLE -> "ex.class/serialization";
      case BUSY -> "ex.class/conflict";
      case CANCELLED, EVALUATION_FAILED -> "ex.class/state";
      case TIMEOUT -> "ex.class/timeout";
      case LIMIT_EXCEEDED -> "ex.class/limit";
      case TRANSPORT_FAILED -> "ex.class/io";
      case PROVIDER_FAILED -> "ex.class/host";
    };
  }

  private static SandboxModel.SandboxId sandboxId(Object value) {
    Object input = HaraBox.unwrap(value);
    if (!(input instanceof Number number))
      throw new HaraException("sandbox id must be a positive integer");
    return new SandboxModel.SandboxId(number.longValue());
  }

  @SuppressWarnings("rawtypes")
  private static SandboxModel.SandboxSpec sandboxSpec(Object value) {
    Object input = HaraBox.unwrap(value);
    if (!(input instanceof hara.lang.protocol.IMapType map))
      throw new HaraException("std.native.Sandbox/open expects a SandboxSpec map");
    java.util.Set<String> allowed =
        java.util.Set.of(
            "protocol", "provider", "runtime", "entry-namespace", "bundles", "mount",
            "provider-options", "limits");
    for (Object item : map) {
      java.util.Map.Entry entry = (java.util.Map.Entry) item;
      Object key = entry.getKey();
      if (!(key instanceof Keyword keyword) || !allowed.contains(keyword.getName()))
        throw new SandboxModel.SandboxException(
            SandboxModel.ErrorCode.INVALID_SPEC, "unknown sandbox spec key " + key);
    }
    if (map.count() != allowed.size()) {
      throw new SandboxModel.SandboxException(
          SandboxModel.ErrorCode.INVALID_SPEC, "SandboxSpec requires exactly eight keys");
    }
    Object protocolValue = HaraBox.unwrap(map.lookup(Keyword.create("protocol")));
    if (!(protocolValue instanceof String protocol)) {
      throw new SandboxModel.SandboxException(
          SandboxModel.ErrorCode.INVALID_SPEC, "protocol must be a string");
    }
    String provider = sandboxString(map, "provider", null);
    String runtime = sandboxString(map, "runtime", null);
    if (provider == null || runtime == null) {
      throw new SandboxModel.SandboxException(
          SandboxModel.ErrorCode.INVALID_SPEC, "provider and runtime are required");
    }
    String entry = sandboxEntryNamespace(map);
    java.util.List<SandboxModel.BundleReference> bundles = sandboxBundles(map);
    SessionModel.SessionMountId mount = sandboxMount(map);
    Object providerOptions = map.lookup(Keyword.create("provider-options"));
    if (!(HaraBox.unwrap(providerOptions) instanceof hara.lang.protocol.IMapType)
        || !sandboxPortable(providerOptions)) {
      throw new SandboxModel.SandboxException(
          SandboxModel.ErrorCode.INVALID_SPEC, "provider-options must be an immutable map");
    }
    return new SandboxModel.SandboxSpec(
        protocol,
        provider,
        runtime,
        entry,
        bundles,
        mount,
        providerOptions,
        sandboxLimits(map));
  }

  @SuppressWarnings("rawtypes")
  private static String sandboxEntryNamespace(hara.lang.protocol.IMapType map) {
    Object value = HaraBox.unwrap(map.lookup(Keyword.create("entry-namespace")));
    if (value instanceof Symbol symbol && symbol.getNamespace() == null) return symbol.display();
    throw new SandboxModel.SandboxException(
        SandboxModel.ErrorCode.INVALID_SPEC, "entry-namespace must be an unqualified symbol");
  }

  @SuppressWarnings("rawtypes")
  private static boolean sandboxPortable(Object value) {
    Object input = HaraBox.unwrap(value);
    if (input == null
        || input instanceof Boolean
        || input instanceof String
        || input instanceof Number
        || input instanceof HaraCharacter
        || input instanceof Character
        || input instanceof Keyword
        || input instanceof Symbol) return true;
    if (input instanceof hara.lang.protocol.IMapType map) {
      for (Object item : map) {
        java.util.Map.Entry entry = (java.util.Map.Entry) item;
        if (!sandboxPortable(entry.getKey()) || !sandboxPortable(entry.getValue())) return false;
      }
      return true;
    }
    if (input instanceof hara.lang.protocol.ILinearType<?> values) {
      for (int index = 0; index < values.count(); index++) {
        if (!sandboxPortable(values.nth(index))) return false;
      }
      return true;
    }
    if (input instanceof hara.lang.protocol.ISetType<?> values) {
      for (Object item : values) if (!sandboxPortable(item)) return false;
      return true;
    }
    return false;
  }

  @SuppressWarnings("rawtypes")
  private static java.util.List<SandboxModel.BundleReference> sandboxBundles(
      hara.lang.protocol.IMapType spec) {
    Object raw = HaraBox.unwrap(spec.lookup(Keyword.create("bundles")));
    if (raw == null) {
      throw new SandboxModel.SandboxException(
          SandboxModel.ErrorCode.INVALID_SPEC, "bundles must be a vector");
    }
    if (!(raw instanceof hara.lang.protocol.ILinearType<?> bundles)) {
      throw new SandboxModel.SandboxException(
          SandboxModel.ErrorCode.INVALID_SPEC, "bundles must be a vector");
    }
    ArrayList<SandboxModel.BundleReference> resolved = new ArrayList<>();
    for (int index = 0; index < bundles.count(); index++) {
      Object item = HaraBox.unwrap(bundles.nth(index));
      if (!(item instanceof hara.lang.protocol.IMapType bundle) || bundle.count() != 2) {
        throw new SandboxModel.SandboxException(
            SandboxModel.ErrorCode.INVALID_SPEC, "bundle references require digest and format");
      }
      resolved.add(
          new SandboxModel.BundleReference(
              sandboxString(bundle, "digest", null), sandboxString(bundle, "format", null)));
    }
    return resolved;
  }

  @SuppressWarnings("rawtypes")
  private static SessionModel.SessionMountId sandboxMount(
      hara.lang.protocol.IMapType spec) {
    Object raw = HaraBox.unwrap(spec.lookup(Keyword.create("mount")));
    if (raw == null) return null;
    if (!(raw instanceof Number number) || number.longValue() <= 0) {
      throw new SandboxModel.SandboxException(
          SandboxModel.ErrorCode.INVALID_SPEC, "mount must be an opaque positive mount id");
    }
    return SessionModel.SessionMountId.of(number.longValue());
  }

  @SuppressWarnings("rawtypes")
  private static SandboxModel.SandboxLimits sandboxLimits(
      hara.lang.protocol.IMapType spec) {
    Object raw = HaraBox.unwrap(spec.lookup(Keyword.create("limits")));
    if (raw == null) {
      throw new SandboxModel.SandboxException(
          SandboxModel.ErrorCode.INVALID_SPEC, "limits must be a map");
    }
    if (!(raw instanceof hara.lang.protocol.IMapType limits)) {
      throw new SandboxModel.SandboxException(
          SandboxModel.ErrorCode.INVALID_SPEC, "limits must be a map");
    }
    java.util.Set<String> allowed =
        java.util.Set.of(
            "source-bytes", "result-bytes", "output-bytes", "evaluation-ms", "memory-bytes",
            "active-evaluations");
    for (Object item : limits) {
      java.util.Map.Entry entry = (java.util.Map.Entry) item;
      if (!(entry.getKey() instanceof Keyword keyword) || !allowed.contains(keyword.getName())) {
        throw new SandboxModel.SandboxException(
            SandboxModel.ErrorCode.INVALID_SPEC, "unknown sandbox limit " + entry.getKey());
      }
    }
    if (limits.count() != allowed.size()) {
      throw new SandboxModel.SandboxException(
          SandboxModel.ErrorCode.INVALID_SPEC, "limits require exactly six keys");
    }
    SandboxModel.SandboxLimits defaults = SandboxModel.SandboxLimits.defaults();
    return new SandboxModel.SandboxLimits(
        Math.toIntExact(sandboxPositive(limits, "source-bytes", defaults.sourceBytes())),
        Math.toIntExact(sandboxPositive(limits, "result-bytes", defaults.resultBytes())),
        Math.toIntExact(sandboxPositive(limits, "output-bytes", defaults.outputBytes())),
        sandboxPositive(limits, "evaluation-ms", defaults.evaluationMillis()),
        sandboxPositive(limits, "memory-bytes", defaults.memoryBytes()),
        Math.toIntExact(
            sandboxPositive(limits, "active-evaluations", defaults.activeEvaluations())));
  }

  @SuppressWarnings("rawtypes")
  private static long sandboxPositive(
      hara.lang.protocol.IMapType map, String name, long fallback) {
    Object raw = HaraBox.unwrap(map.lookup(Keyword.create(name)));
    if (raw == null) return fallback;
    if (!(raw instanceof Number number) || number.longValue() <= 0) {
      throw new SandboxModel.SandboxException(
          SandboxModel.ErrorCode.INVALID_SPEC, name + " must be a positive integer");
    }
    return number.longValue();
  }

  @SuppressWarnings("rawtypes")
  private static String sandboxString(
      hara.lang.protocol.IMapType map, String name, String fallback) {
    Object value = HaraBox.unwrap(map.lookup(Keyword.create(name)));
    if (value == null) return fallback;
    if (value instanceof String text) return text;
    if (value instanceof Keyword keyword) return keyword.getName();
    if (value instanceof Symbol symbol) return symbol.display();
    throw new SandboxModel.SandboxException(
        SandboxModel.ErrorCode.INVALID_SPEC, name + " must be a string, keyword, or symbol");
  }

  private static Object sandboxStatusValue(SandboxModel.SandboxStatus status) {
    java.util.LinkedHashMap<Object, Object> value = new java.util.LinkedHashMap<>();
    value.put(Keyword.create("sandbox/id"), status.id().value());
    value.put(Keyword.create("sandbox/provider"), status.provider());
    value.put(
        Keyword.create("sandbox/state"), Keyword.create(status.state().name().toLowerCase()));
    value.put(Keyword.create("sandbox/secure"), status.secure());
    value.put(Keyword.create("sandbox/evaluation-active"), status.evaluationActive());
    SandboxModel.SandboxError error = status.error();
    value.put(
        Keyword.create("sandbox/error"),
        error == null
            ? null
            : HaraPersistentValues.normalize(
                java.util.Map.of(
                    Keyword.create("code"),
                    Keyword.create(error.code().name().toLowerCase().replace('_', '-')),
                    Keyword.create("message"),
                    error.message())));
    return HaraPersistentValues.normalize(value);
  }

  private void installJvmNativeLibraries() {
    HaraNamespace reflect = namespace("hara.native.jvm.reflect");
    reflect.define(
        "type",
        new UnaryBuiltin(
            "reflect/type", value -> jvmProvider().type(HaraBox.unwrap(value), nativeAccess())));
    reflect.define(
        "name",
        new UnaryBuiltin(
            "reflect/name",
            value -> jvmProvider().typeName(HaraBox.unwrap(value), nativeAccess())));
    reflect.define(
        "instance?",
        new VariadicBuiltin(
            "reflect/instance?",
            values -> {
              requireMethodArity("reflect/instance?", values, 2);
              return jvmProvider()
                  .isInstance(HaraBox.unwrap(values[0]), HaraBox.unwrap(values[1]), nativeAccess());
            }));
    reflect.define(
        "fields",
        new UnaryBuiltin(
            "reflect/fields",
            value -> jvmProvider().fields(HaraBox.unwrap(value), nativeAccess())));
    reflect.define(
        "methods",
        new UnaryBuiltin(
            "reflect/methods",
            value -> jvmProvider().methods(HaraBox.unwrap(value), nativeAccess())));

    HaraNamespace classpath = namespace("hara.native.jvm.classpath");
    classpath.define(
        "paths",
        new VariadicBuiltin(
            "classpath/paths",
            values -> {
              requireMethodArity("classpath/paths", values, 0);
              return jvmProvider().classPath(nativeAccess());
            }));
    classpath.define(
        "add",
        new UnaryBuiltin(
            "classpath/add",
            value ->
                jvmProvider().addClassPath(String.valueOf(HaraBox.unwrap(value)), nativeAccess())));

  }

  void installNativeLibrary(String namespace) {
    NativeLibraryInstaller installer = NATIVE_LIBRARY_INSTALLERS.get(namespace);
    if (installer == null) {
      throw new HaraException("No registered native library installer: " + namespace);
    }
    String nativeType = namespace.startsWith("std.native.")
        ? namespace.substring("std.native.".length())
        : namespace;
    if (!HaraNativeDeclarations.namespace(nativeType).equals(namespace)) {
      throw new HaraException("Native library is not annotated: " + namespace);
    }
    withDefinitionOrigin(installer.origin(), () -> installer.install().accept(this));
  }

  private void defineStringLibrary() {
    HaraNamespace string = namespace("std.native.String");

    // Spec-named symbols.
    string.define(
        "length",
        new UnaryBuiltin("str/length", value -> (long) codePointLength(stringValue(value, "str/length"))));
    string.define(
        "blank?",
        new UnaryBuiltin(
            "str/blank?", value -> stringValue(value, "str/blank?").trim().isEmpty()));
    string.define(
        "includes?",
        new VariadicBuiltin(
            "str/includes?",
            values -> {
              String[] pair = stringPair(values, "str/includes?");
              return pair[0].contains(pair[1]);
            }));
    string.define(
        "starts-with?",
        new VariadicBuiltin(
            "str/starts-with?",
            values -> {
              String[] pair = stringPair(values, "str/starts-with?");
              return pair[0].startsWith(pair[1]);
            }));
    string.define(
        "ends-with?",
        new VariadicBuiltin(
            "str/ends-with?",
            values -> {
              String[] pair = stringPair(values, "str/ends-with?");
              return pair[0].endsWith(pair[1]);
            }));
    string.define("char-at", new VariadicBuiltin("str/char-at", this::stringCharAt));
    string.define("slice", new VariadicBuiltin("str/slice", this::stringSlice));
    string.define("index-of", new VariadicBuiltin("str/index-of", this::stringIndexOf));
    string.define(
        "last-index-of", new VariadicBuiltin("str/last-index-of", this::stringLastIndexOf));
    string.define("split", new VariadicBuiltin("str/split", this::stringSplit));
    string.define("split-lines", new VariadicBuiltin("str/split-lines", this::stringSplitLines));
    string.define("join", new VariadicBuiltin("str/join", this::stringJoin));
    string.define("repeat", new VariadicBuiltin("str/repeat", this::stringRepeat));
    string.define("replace", new VariadicBuiltin("str/replace", this::stringReplace));
    string.define(
        "replace-first", new VariadicBuiltin("str/replace-first", this::stringReplaceFirst));
    string.define(
        "trim", new UnaryBuiltin("str/trim", value -> stringValue(value, "str/trim").trim()));
    string.define(
        "trim-left",
        new UnaryBuiltin(
            "str/trim-left", value -> stringValue(value, "str/trim-left").stripLeading()));
    string.define(
        "trim-right",
        new UnaryBuiltin(
            "str/trim-right", value -> stringValue(value, "str/trim-right").stripTrailing()));
    string.define(
        "upper",
        new UnaryBuiltin(
            "str/upper", value -> stringValue(value, "str/upper").toUpperCase(java.util.Locale.ROOT)));
    string.define(
        "lower",
        new UnaryBuiltin(
            "str/lower", value -> stringValue(value, "str/lower").toLowerCase(java.util.Locale.ROOT)));
    string.define(
        "capitalize",
        new UnaryBuiltin("str/capitalize", value -> stringCapitalize(stringValue(value, "str/capitalize"))));
    string.define(
        "decapitalize",
        new UnaryBuiltin(
            "str/decapitalize", value -> stringDecapitalize(stringValue(value, "str/decapitalize"))));
    string.define(
        "pad-left", new VariadicBuiltin("str/pad-left", values -> padString(values, true)));
    string.define(
        "pad-right", new VariadicBuiltin("str/pad-right", values -> padString(values, false)));
    string.define(
        "reverse",
        new UnaryBuiltin(
            "str/reverse", value -> new StringBuilder(stringValue(value, "str/reverse")).reverse().toString()));
    string.define(
        "encode-utf8",
        new UnaryBuiltin(
            "str/encode-utf8",
            value -> stringValue(value, "str/encode-utf8").getBytes(StandardCharsets.UTF_8)));
    string.define(
        "decode-utf8",
        new UnaryBuiltin(
            "str/decode-utf8",
            value -> new String(bytesValue(value, "str/decode-utf8"), StandardCharsets.UTF_8)));

    string.define("to-fixed", new VariadicBuiltin("str/to-fixed", this::stringToFixed));
  }

  private void defineBytesLibrary() {
    HaraNamespace bytes = namespace("std.native.Bytes");
    bytes.define("new", new VariadicBuiltin("std.native.Bytes/new", this::createBytes));
    bytes.define(
        "count",
        new UnaryBuiltin("bytes/count", value -> (long) bytesValue(value, "bytes/count").length));
    bytes.define("get", new VariadicBuiltin("bytes/get", this::bytesGet));
    bytes.define("set", new VariadicBuiltin("bytes/set", this::bytesSet));
    bytes.define(
        "copy", new UnaryBuiltin("bytes/copy", value -> bytesValue(value, "bytes/copy").clone()));
    bytes.define("slice", new VariadicBuiltin("bytes/slice", this::bytesSlice));
    bytes.define(
        "u8", new UnaryBuiltin("bytes/u8", value -> (long) (byteNumber(value, "bytes/u8") & 0xff)));
    bytes.define(
        "s8", new UnaryBuiltin("bytes/s8", value -> (long) (byte) byteNumber(value, "bytes/s8")));
  }

  private void definePromiseLibrary() {
    HaraNamespace promise = namespace("std.native.Promise");
    promise.define("run", new UnaryBuiltin("std.native.Promise/run", this::promiseRun));
    promise.define("new", new UnaryBuiltin("promise/new", this::promiseNew));
    promise.define("from", new UnaryBuiltin("promise/from", this::promiseFrom));
    promise.define("all", new UnaryBuiltin("promise/all", this::promiseAll));
    promise.define(
        "state",
        new UnaryBuiltin("promise/state", value -> requirePromise(value, "promise/state").state()));
    promise.define(
        "value",
        new UnaryBuiltin("promise/value", value -> requirePromise(value, "promise/value").value()));
    promise.define(
        "then", new VariadicBuiltin("promise/then", values -> promiseThen(values, false)));
    promise.define(
        "catch", new VariadicBuiltin("promise/catch", values -> promiseThen(values, true)));
    promise.define("finally", new VariadicBuiltin("promise/finally", this::promiseFinally));
    promise.define(
        "cancel",
        new UnaryBuiltin(
            "promise/cancel", value -> requirePromise(value, "promise/cancel").cancel()));
    promise.define("delay", new VariadicBuiltin("promise/delay", this::promiseDelay));
  }

  private void defineJsonLibrary() {
    HaraNamespace json = namespace("std.native.Json");
    json.define(
        "read",
        new UnaryBuiltin(
            "std.native.Json/read",
            value -> {
              if (!(HaraBox.unwrap(value) instanceof String source)) {
                throw new HaraException("json/read expects a string");
              }
              try {
                return StdJson.read(source);
              } catch (IllegalArgumentException error) {
                throw new HaraException("json/read: " + error.getMessage());
              }
            }));
    json.define(
        "write",
        new UnaryBuiltin(
            "std.native.Json/write",
            value -> {
              try {
                return StdJson.write(HaraBox.unwrap(value));
              } catch (IllegalArgumentException error) {
                throw new HaraException("json/write: " + error.getMessage());
              }
            }));
    json.define(
        "pretty",
        new VariadicBuiltin(
            "std.native.Json/pretty",
            values -> {
              requireMethodArity("json/pretty", values, 2);
              if (!(HaraBox.unwrap(values[1]) instanceof IMapType<?, ?>)) {
                throw new HaraException("json/pretty expects an options map");
              }
              try {
                return StdJson.writePretty(HaraBox.unwrap(values[0]));
              } catch (IllegalArgumentException error) {
                throw new HaraException("json/pretty: " + error.getMessage());
              }
            }));
  }

  private void defineFileLibrary() {
    HaraNamespace file = namespace("std.native.File");
    file.define("parent", new UnaryBuiltin("std.native.File/parent", this::fileParent));
    file.define("join", new VariadicBuiltin("std.native.File/join", this::fileJoin));
    file.define("resolve", new VariadicBuiltin("std.native.File/resolve", this::fileResolve));
    file.define("read", new UnaryBuiltin("std.native.File/read", this::fileRead));
    file.define("write", new VariadicBuiltin("std.native.File/write", this::fileWrite));
    file.define("exists?", new UnaryBuiltin("std.native.File/exists?", this::fileExists));
    file.define("stat", new UnaryBuiltin("std.native.File/stat", this::fileStat));
    file.define("entries", new UnaryBuiltin("std.native.File/entries", this::fileEntries));
    file.define("list", new UnaryBuiltin("std.native.File/list", this::fileList));
    file.define("walk", new UnaryBuiltin("std.native.File/walk", this::fileWalk));
    file.define("mkdir", new VariadicBuiltin("std.native.File/mkdir", this::fileMkdir));
    file.define("delete", new VariadicBuiltin("std.native.File/delete", this::fileDelete));
    file.define("copy", new VariadicBuiltin("std.native.File/copy", this::fileCopy));
    file.define("move", new VariadicBuiltin("std.native.File/move", this::fileMove));
    file.define("temp-file", new VariadicBuiltin("std.native.File/temp-file", this::fileTempFile));
    file.define(
        "temp-directory",
        new VariadicBuiltin("std.native.File/temp-directory", this::fileTempDirectory));
  }

  private void defineOsLibrary() {
    HaraNamespace os = namespace("std.native.OS");
    os.define("platform", new VariadicBuiltin("std.native.OS/platform", this::osPlatform));
    os.define("arch", new VariadicBuiltin("std.native.OS/arch", this::osArch));
    os.define("cwd", new VariadicBuiltin("std.native.OS/cwd", this::osCwd));
    os.define("env", new VariadicBuiltin("std.native.OS/env", this::osEnv));
    os.define("getenv", new UnaryBuiltin("std.native.OS/getenv", this::osGetenv));
    os.define("time-ms", new VariadicBuiltin("std.native.OS/time-ms", this::osTimeMs));
    os.define("time-ns", new VariadicBuiltin("std.native.OS/time-ns", this::osTimeNs));
    HaraNamespace process = namespace("std.native.Process");
    process.define("spawn", new VariadicBuiltin("std.native.Process/spawn", this::osSpawn));
    process.define("alive?", new UnaryBuiltin("std.native.Process/alive?", value -> requireProcess(value, "std.native.Process/alive?").process.isAlive()));
    process.define("write", new VariadicBuiltin("std.native.Process/write", this::osProcessWrite));
    process.define("close-input", new UnaryBuiltin("std.native.Process/close-input", this::osProcessCloseInput));
    process.define("stdout", new UnaryBuiltin("std.native.Process/stdout", value -> new HaraPromise(requireProcess(value, "std.native.Process/stdout").stdout)));
    process.define("stderr", new UnaryBuiltin("std.native.Process/stderr", value -> new HaraPromise(requireProcess(value, "std.native.Process/stderr").stderr)));
    process.define("stdout-stream", new UnaryBuiltin("std.native.Process/stdout-stream", value -> requireProcess(value, "std.native.Process/stdout-stream").stdoutStream));
    process.define("stderr-stream", new UnaryBuiltin("std.native.Process/stderr-stream", value -> requireProcess(value, "std.native.Process/stderr-stream").stderrStream));
    process.define(
        "wait",
        new UnaryBuiltin(
            "std.native.Process/wait",
            value -> {
              HaraProcess handle = requireProcess(value, "std.native.Process/wait");
              return new HaraPromise(
                  handle.exit,
                  () -> {
                    if (handle.process.isAlive()) handle.process.destroyForcibly();
                  });
            }));
    process.define("kill", new UnaryBuiltin("std.native.Process/kill", this::osProcessKill));
  }

  private void requireProcessIO(String operation) {
    if (!environment.isCreateProcessAllowed()) {
      throw HaraNativeCapabilityBoundary.denied(
          "Process", HaraNativeCapabilityBoundary.method(operation), "native-runtime");
    }
  }

  private Object osPlatform(Object[] values) {
    requireMethodArity("os/platform", values, 0);
    String name = System.getProperty("os.name", "").toLowerCase(java.util.Locale.ROOT);
    return Keyword.create(name.contains("win") ? "windows" : name.contains("mac") ? "macos" : name.contains("nux") ? "linux" : "unknown");
  }

  private Object osTimeMs(Object[] values) {
    requireMethodArity("os/time-ms", values, 0);
    return System.currentTimeMillis();
  }

  private Object osTimeNs(Object[] values) {
    requireMethodArity("os/time-ns", values, 0);
    return System.nanoTime();
  }

  private Object osArch(Object[] values) {
    requireMethodArity("os/arch", values, 0);
    String arch = System.getProperty("os.arch", "unknown").toLowerCase(java.util.Locale.ROOT);
    if (arch.equals("amd64") || arch.equals("x86_64")) arch = "x86-64";
    else if (arch.equals("aarch64") || arch.equals("arm64")) arch = "aarch64";
    return Keyword.create(arch.replace('_', '-'));
  }

  private Object osCwd(Object[] values) {
    requireMethodArity("os/cwd", values, 0);
    return Path.of("").toAbsolutePath().normalize().toString();
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  private Object osEnv(Object[] values) {
    requireMethodArity("os/env", values, 0);
    IMapType result = hara.lang.data.Map.Standard.EMPTY;
    for (Map.Entry<String, String> entry : System.getenv().entrySet()) result = (IMapType) result.assoc(entry.getKey(), entry.getValue());
    return result;
  }

  private Object osGetenv(Object value) {
    String found = System.getenv(stringValue(value, "os/getenv"));
    return found == null ? null : found;
  }

  private java.util.List<String> processArgv(Object value) {
    Object raw = HaraBox.unwrap(value);
    if (!(raw instanceof ILinearType<?> argv) || argv.count() == 0) throw new HaraException("os/spawn expects a non-empty vector of strings");
    java.util.List<String> result = new ArrayList<>();
    for (int index = 0; index < argv.count(); index++) result.add(stringValue(argv.nth(index), "os/spawn"));
    return result;
  }

  @SuppressWarnings({"rawtypes"})
  private Object osSpawn(Object[] values) {
    requireProcessIO("os/spawn");
    if (values.length < 1 || values.length > 2) throw new HaraException("os/spawn expects argv and optional options");
    ProcessBuilder builder = new ProcessBuilder(processArgv(values[0]));
    if (values.length == 2) {
      Object raw = HaraBox.unwrap(values[1]);
      if (!(raw instanceof IMapType options)) throw new HaraException("os/spawn options must be a map");
      Object cwd = options.lookup(Keyword.create("cwd"));
      if (cwd != null) builder.directory(new File(stringValue(cwd, "os/spawn :cwd")));
      Object env = options.lookup(Keyword.create("env"));
      if (env != null) {
        if (!(HaraBox.unwrap(env) instanceof IMapType envMap)) throw new HaraException("os/spawn :env must be a map");
        for (Object item : envMap) {
          Map.Entry entry = (Map.Entry) item;
          builder.environment().put(stringValue(entry.getKey(), "os/spawn :env"), stringValue(entry.getValue(), "os/spawn :env"));
        }
      }
    }
    try { return new HaraProcess(builder.start()); }
    catch (IOException error) { throw new HaraException("os/spawn failed: " + error.getMessage()); }
  }

  private HaraProcess requireProcess(Object value, String operation) {
    requireProcessIO(operation);
    Object raw = HaraBox.unwrap(value);
    if (!(raw instanceof HaraProcess process)) throw new HaraException(operation + " expects a process");
    return process;
  }

  private Object osProcessWrite(Object[] values) {
    if (values.length != 2) throw new HaraException("os/process-write expects a process and bytes");
    HaraProcess process = requireProcess(values[0], "os/process-write");
    byte[] bytes = bytesValue(values[1], "os/process-write");
    synchronized (process.stdin) {
      try { process.stdin.write(bytes); process.stdin.flush(); return (long) bytes.length; }
      catch (IOException error) { throw new HaraException("os/process-write failed: " + error.getMessage()); }
    }
  }

  private Object osProcessCloseInput(Object value) {
    HaraProcess process = requireProcess(value, "os/process-close-input");
    try { process.stdin.close(); } catch (IOException error) { throw new HaraException("os/process-close-input failed: " + error.getMessage()); }
    return null;
  }

  private Object osProcessKill(Object value) {
    HaraProcess process = requireProcess(value, "os/process-kill");
    if (process.process.isAlive()) process.process.destroyForcibly();
    return process;
  }

  private void defineSocketLibrary() {
    HaraNamespace socket = namespace("std.native.Socket");
    socket.define("connect", new VariadicBuiltin("std.native.Socket/connect", this::socketConnect));
    socket.define("listen", new VariadicBuiltin("std.native.Socket/listen", this::socketListen));
    socket.define("endpoint", new UnaryBuiltin("std.native.Socket/endpoint", this::socketEndpoint));
    socket.define("events", new VariadicBuiltin("std.native.Socket/events", this::socketEvents));
    socket.define("next", new UnaryBuiltin("std.native.Socket/next", this::socketNext));
    socket.define("receive-stream", new UnaryBuiltin("std.native.Socket/receive-stream", this::socketReceiveStream));
    socket.define("send", new VariadicBuiltin("std.native.Socket/send", this::socketSend));
    socket.define("close", new UnaryBuiltin("std.native.Socket/close", this::socketClose));
  }

  private static Object bitOperation(String operation, Object[] values) {
    if (values.length != 2) throw new HaraException("bit-" + operation + " expects two integers");
    Object left = HaraNumericConversions.toInteger(values[0], "bit-" + operation);
    Object right = HaraNumericConversions.toInteger(values[1], "bit-" + operation);
    if ("and".equals(operation)) return Num.and(left, right);
    if ("or".equals(operation)) return Num.or(left, right);
    return Num.xor(left, right);
  }

  private static Object bitShift(Object[] values, boolean left) {
    String name = left ? "bit-shift-left" : "bit-shift-right";
    if (values.length != 2) throw new HaraException(name + " expects two integers");
    Object value = HaraNumericConversions.toInteger(values[0], name);
    int distance = HaraNumericConversions.toShiftDistance(values[1], name);
    return left ? Num.shiftLeft(value, distance) : Num.shiftRight(value, distance);
  }

  private static Number numericValue(Object value, String operation) {
    return HaraNumericConversions.toNumber(value, operation);
  }

  private static Object numericAbs(Object value) {
    Number input = numericValue(value, "abs");
    return Num.isNeg(input) ? Num.minusP(input) : input;
  }

  private static UnaryBuiltin mathUnary(String operation, DoubleUnaryOperator implementation) {
    return new UnaryBuiltin(
        operation,
        value ->
            HaraNumericConversions.requireFinite(
                implementation.applyAsDouble(HaraNumericConversions.toDouble(value))));
  }

  private static Object mathBinary(String operation, Object[] values) {
    requireMethodArity(operation, values, 2);
    double first = HaraNumericConversions.toDouble(values[0]);
    double second = HaraNumericConversions.toDouble(values[1]);
    return HaraNumericConversions.requireFinite(
        "atan2".equals(operation) ? Math.atan2(first, second) : Math.pow(first, second));
  }

  private static double asinh(double value) {
    double magnitude = Math.abs(value);
    if (magnitude > 1.0e154) {
      return Math.copySign(Math.log(magnitude) + Math.log(2.0), value);
    }
    return Math.copySign(Math.log(magnitude + Math.hypot(magnitude, 1.0)), value);
  }

  private static double acosh(double value) {
    if (value > 1.0e154) return Math.log(value) + Math.log(2.0);
    return Math.log(value + Math.sqrt(value - 1.0) * Math.sqrt(value + 1.0));
  }

  private static double atanh(double value) {
    return 0.5 * (Math.log1p(value) - Math.log1p(-value));
  }

  private static String stringValue(Object value, String operation) {
    Object input = HaraBox.unwrap(value);
    if (!(input instanceof String)) throw new HaraException(operation + " expects a string");
    return (String) input;
  }

  private static String[] stringPair(Object[] values, String operation) {
    if (values.length != 2) throw new HaraException(operation + " expects two strings");
    return new String[] {stringValue(values[0], operation), stringValue(values[1], operation)};
  }

  private static int codePointLength(String input) {
    return input.codePointCount(0, input.length());
  }

  private Object padString(Object[] values, boolean left) {
    String operation = left ? "str/pad-left" : "str/pad-right";
    if (values.length != 3) {
      throw new HaraException(operation + " expects a string, length, and padding string");
    }
    String input = stringValue(values[0], operation);
    int length = HaraNumericConversions.toInt(values[1], operation);
    String padding = stringValue(values[2], operation);
    int inputLength = codePointLength(input);
    if (padding.isEmpty() || inputLength >= length) return input;
    int[] paddingCodePoints = padding.codePoints().toArray();
    StringBuilder fill = new StringBuilder();
    for (int index = 0; index < length - inputLength; index++) {
      fill.appendCodePoint(paddingCodePoints[index % paddingCodePoints.length]);
    }
    return left ? fill + input : input + fill;
  }

  private Object stringCharAt(Object[] values) {
    if (values.length != 2) {
      throw new HaraException("str/char-at expects a string and index");
    }
    String input = stringValue(values[0], "str/char-at");
    int index = HaraNumericConversions.toInt(values[1], "str/char-at");
    int length = codePointLength(input);
    if (index < 0 || index >= length) {
      throw new HaraException("str/char-at index out of bounds");
    }
    int charIndex = input.offsetByCodePoints(0, index);
    int codePoint = input.codePointAt(charIndex);
    return HaraCharacter.of(codePoint);
  }

  private Object stringSplit(Object[] values) {
    if (values.length != 2) {
      throw new HaraException("str/split expects a string and string or regexp separator");
    }
    String input = stringValue(values[0], "str/split");
    if (input.isEmpty()) return null;
    Object separator = HaraBox.unwrap(values[1]);
    String[] parts;
    if (separator instanceof java.util.regex.Pattern pattern) {
      parts = pattern.split(input);
    } else if (separator instanceof String text) {
      parts = input.split(java.util.regex.Pattern.quote(text), -1);
    } else {
      throw new HaraException("str/split expects a string and string or regexp separator");
    }
    return hara.lang.data.Vector.Standard.from(null, (Object[]) parts);
  }

  private Object parseLong(Object value) {
    String input = stringValue(value, "parse-long");
    if (input.isEmpty() || !input.equals(input.trim())) return null;
    try {
      return Long.parseLong(input);
    } catch (NumberFormatException ignored) {
      return null;
    }
  }

  private Object parseDouble(Object value) {
    String input = stringValue(value, "parse-double");
    if (input.isEmpty() || !input.equals(input.trim())) return null;
    if (input.equals("NaN")
        || input.equals("Infinity")
        || input.equals("+Infinity")
        || input.equals("-Infinity")) {
      throw new HaraException("non-finite number");
    }
    if (!input.matches("[+-]?(?:(?:[0-9]+(?:\\.[0-9]*)?)|(?:\\.[0-9]+))(?:[eE][+-]?[0-9]+)?")) {
      return null;
    }
    try {
      return HaraNumericConversions.requireFinite(Double.parseDouble(input));
    } catch (NumberFormatException ignored) {
      return null;
    }
  }

  private Object stringSplitLines(Object[] values) {
    if (values.length != 1) throw new HaraException("str/split-lines expects one string");
    String input = stringValue(values[0], "str/split-lines");
    String[] parts = input.split("\n", -1);
    return hara.lang.data.Vector.Standard.from(null, (Object[]) parts);
  }

  private Object stringJoin(Object[] values) {
    if (values.length != 2) throw new HaraException("str/join expects a separator and collection");
    String separator = stringValue(values[0], "str/join");
    Iterator<?> iterator = (Iterator<?>) iterValue(values[1]);
    StringBuilder output = new StringBuilder();
    while (iterator.hasNext()) {
      if (output.length() > 0) output.append(separator);
      Object item = HaraBox.unwrap(iterator.next());
      if (item instanceof String text) {
        output.append(text);
      } else if (item instanceof HaraCharacter character) {
        output.append(character.text());
      } else if (item instanceof Character character) {
        output.append(character);
      } else {
        throw new HaraException("str/join expects a collection of strings or characters");
      }
    }
    return output.toString();
  }

  private Object stringIndexOf(Object[] values) {
    if (values.length < 2 || values.length > 3) {
      throw new HaraException("str/index-of expects a string, substring, and optional offset");
    }
    String input = stringValue(values[0], "str/index-of");
    String part = stringValue(values[1], "str/index-of");
    int offset = values.length == 2
        ? 0
        : HaraNumericConversions.toInt(values[2], "str/index-of");
    int length = codePointLength(input);
    if (offset < 0 || offset > length) return -1L;
    int charOffset = input.offsetByCodePoints(0, offset);
    int result = input.indexOf(part, charOffset);
    return result < 0 ? -1L : (long) input.codePointCount(0, result);
  }

  private Object stringSlice(Object[] values) {
    if (values.length < 2 || values.length > 3) {
      throw new HaraException("str/slice expects a string, start, and optional end");
    }
    String input = stringValue(values[0], "str/slice");
    int start = HaraNumericConversions.toInt(values[1], "str/slice");
    int end = values.length == 3
        ? HaraNumericConversions.toInt(values[2], "str/slice")
        : codePointLength(input);
    int length = codePointLength(input);
    if (start < 0 || start > end || end > length) {
      throw new HaraException("str/slice range is out of bounds");
    }
    return input.substring(
        input.offsetByCodePoints(0, start), input.offsetByCodePoints(0, end));
  }

  private Object stringToFixed(Object[] values) {
    if (values.length != 2 || !HaraNumericConversions.isNumeric(values[0])) {
      throw new HaraException("str/to-fixed expects a number and precision");
    }
    int precision = HaraNumericConversions.toInt(values[1], "str/to-fixed");
    if (precision < 0 || precision > 100) {
      throw new HaraException("str/to-fixed precision must be in the range 0..100");
    }
    return String.format(
        java.util.Locale.ROOT,
        "%." + precision + "f",
        HaraNumericConversions.toDouble(values[0]));
  }

  private Object stringReplace(Object[] values) {
    if (values.length != 3) {
      throw new HaraException("str/replace expects a string, match, and replacement");
    }
    return stringValue(values[0], "str/replace")
        .replace(stringValue(values[1], "str/replace"), stringValue(values[2], "str/replace"));
  }

  private Object stringReplaceFirst(Object[] values) {
    if (values.length != 3) {
      throw new HaraException("str/replace-first expects a string, match, and replacement");
    }
    String input = stringValue(values[0], "str/replace-first");
    String match = stringValue(values[1], "str/replace-first");
    String replacement = stringValue(values[2], "str/replace-first");
    int index = input.indexOf(match);
    if (index < 0) return input;
    return input.substring(0, index) + replacement + input.substring(index + match.length());
  }

  private Object stringRepeat(Object[] values) {
    if (values.length != 2) {
      throw new HaraException("str/repeat expects a string and count");
    }
    String input = stringValue(values[0], "str/repeat");
    int count = HaraNumericConversions.toInt(values[1], "str/repeat");
    if (count < 0) throw new HaraException("str/repeat count must be non-negative");
    return input.repeat(count);
  }

  private static String stringCapitalize(String input) {
    if (input.isEmpty()) return input;
    int first = input.codePointAt(0);
    int rest = Character.charCount(first);
    return new String(Character.toChars(Character.toUpperCase(first))) + input.substring(rest);
  }

  private static String stringDecapitalize(String input) {
    if (input.isEmpty()) return input;
    int first = input.codePointAt(0);
    int rest = Character.charCount(first);
    return new String(Character.toChars(Character.toLowerCase(first))) + input.substring(rest);
  }

  private Object stringLastIndexOf(Object[] values) {
    if (values.length < 2 || values.length > 3) {
      throw new HaraException("str/last-index-of expects a string, substring, and optional offset");
    }
    String input = stringValue(values[0], "str/last-index-of");
    String part = stringValue(values[1], "str/last-index-of");
    int length = codePointLength(input);
    int offset = values.length == 2
        ? length
        : HaraNumericConversions.toInt(values[2], "str/last-index-of");
    if (offset < 0) return -1L;
    int charOffset = input.offsetByCodePoints(0, Math.min(offset, length));
    int result = input.lastIndexOf(part, charOffset);
    return result < 0 ? -1L : (long) input.codePointCount(0, result);
  }

  private static byte[] bytesValue(Object value, String operation) {
    Object input = HaraBox.unwrap(value);
    if (!(input instanceof byte[])) throw new HaraException(operation + " expects bytes");
    return (byte[]) input;
  }

  private static int byteNumber(Object value, String operation) {
    long number = HaraNumericConversions.toLong(value, operation);
    if (number < -128 || number > 255) {
      throw new HaraException(operation + " expects a value in the range -128..255");
    }
    return (int) number;
  }

  private Object bytesGet(Object[] values) {
    if (values.length < 2 || values.length > 3) {
      throw new HaraException("bytes/get expects bytes, index, and optional fallback");
    }
    byte[] input = bytesValue(values[0], "bytes/get");
    int index = HaraNumericConversions.toInt(values[1], "bytes/get");
    if (index < 0 || index >= input.length) {
      if (values.length == 3) return values[2];
      throw new HaraException("bytes/get index out of bounds: " + index);
    }
    return (long) Byte.toUnsignedInt(input[index]);
  }

  private Object bytesSet(Object[] values) {
    if (values.length != 3) throw new HaraException("bytes/set expects bytes, index, and value");
    byte[] input = bytesValue(values[0], "bytes/set");
    int index = HaraNumericConversions.toInt(values[1], "bytes/set");
    if (index < 0 || index >= input.length) {
      throw new HaraException("bytes/set index out of bounds: " + index);
    }
    int value = byteNumber(values[2], "bytes/set");
    input[index] = (byte) value;
    return input;
  }

  private Object bytesSlice(Object[] values) {
    if (values.length < 2 || values.length > 3) {
      throw new HaraException("bytes/slice expects bytes, start, and optional end");
    }
    byte[] input = bytesValue(values[0], "bytes/slice");
    int start = HaraNumericConversions.toInt(values[1], "bytes/slice");
    int end = values.length == 3
        ? HaraNumericConversions.toInt(values[2], "bytes/slice")
        : input.length;
    if (start < 0 || end < start || end > input.length) {
      throw new HaraException("bytes/slice range is out of bounds");
    }
    return java.util.Arrays.copyOfRange(input, start, end);
  }

  private Object promiseRun(Object thunk) {
    CompletableFuture<Object> future =
        CompletableFuture.supplyAsync(
                () -> {
                  try {
                    return invokeInContext(() -> invokeCallable(thunk, new Object[0]));
                  } catch (RuntimeException error) {
                    throw new CompletionException(error);
                  }
                })
            .thenCompose(this::flatten);
    return new HaraPromise(future);
  }

  private Object promiseNew(Object thunk) {
    CompletableFuture<Object> future = new CompletableFuture<>();
    Object resolve =
        new UnaryBuiltin(
            "promise-resolve",
            value -> {
              flatten(value)
                  .whenComplete(
                      (resolved, error) -> {
                        if (error == null) future.complete(resolved);
                        else future.completeExceptionally(error);
                      });
              return value;
            });
    Object reject =
        new UnaryBuiltin(
            "promise-reject",
            value -> {
              future.completeExceptionally(new HaraPromiseRejection(value));
              return value;
            });
    try {
      invokeCallable(thunk, new Object[] {resolve, reject});
    } catch (RuntimeException error) {
      future.completeExceptionally(error);
    }
    return new HaraPromise(future);
  }

  private Object promiseFrom(Object value) {
    Object input = HaraBox.unwrap(value);
    return input instanceof HaraPromise ? input : new HaraPromise(flatten(input));
  }

  private HaraPromise requirePromise(Object value, String operation) {
    Object input = HaraBox.unwrap(value);
    if (!(input instanceof HaraPromise)) throw new HaraException(operation + " expects a promise");
    return (HaraPromise) input;
  }

  private CompletableFuture<Object> flatten(Object value) {
    Object input = HaraBox.unwrap(value);
    return input instanceof HaraPromise
        ? ((HaraPromise) input).future
        : CompletableFuture.completedFuture(input);
  }

  private Object promiseAll(Object value) {
    ArrayList<CompletableFuture<Object>> promises = new ArrayList<>();
    ArrayList<HaraPromise> cancellable = new ArrayList<>();
    Iterator<?> iterator = (Iterator<?>) iterValue(value);
    while (iterator.hasNext()) {
      Object item = HaraBox.unwrap(iterator.next());
      if (item instanceof HaraPromise promise) cancellable.add(promise);
      promises.add(flatten(item));
    }
    CompletableFuture<?>[] futures = promises.toArray(new CompletableFuture[0]);
    CompletableFuture<Object> result =
        CompletableFuture.allOf(futures)
            .thenApply(
                ignored ->
                    hara.lang.data.Vector.Standard.from(
                        null,
                        promises.stream()
                            .map(promise -> HaraPersistentValues.normalize(promise.join()))
                            .toArray()));
    return new HaraPromise(result, () -> cancellable.forEach(HaraPromise::cancel));
  }

  private Object promiseThen(Object[] values, boolean failure) {
    String operation = failure ? "promise/catch" : "promise/then";
    if (values.length != 2) throw new HaraException(operation + " expects a promise and function");
    HaraPromise promise = requirePromise(values[0], operation);
    CompletableFuture<Object> result;
    if (failure) {
      result =
          promise
              .future
              .handle(
                  (value, error) ->
                      error == null
                          ? CompletableFuture.completedFuture(value)
                          : flatten(
                              invokeInContext(
                                  () ->
                                      invokeCallable(
                                          values[1],
                                          new Object[] {
                                            promiseRejectionValue(error)
                                          }))))
              .thenCompose(Function.identity());
    } else {
      result =
          promise
              .future
              .thenApply(
                  value ->
                      flatten(
                          invokeInContext(() -> invokeCallable(values[1], new Object[] {value}))))
              .thenCompose(Function.identity());
    }
    return new HaraPromise(result, () -> promise.cancel());
  }

  private Object promiseRejectionValue(Throwable error) {
    Throwable cause = error.getCause() == null ? error : error.getCause();
    return cause instanceof HaraPromiseRejection rejection ? rejection.value : cause;
  }

  private Object promiseFinally(Object[] values) {
    if (values.length != 2) {
      throw new HaraException("promise/finally expects a promise and function");
    }
    HaraPromise promise = requirePromise(values[0], "promise/finally");
    CompletableFuture<Object> result =
        promise
            .future
            .handle(
                (value, error) ->
                    flatten(invokeInContext(() -> invokeCallable(values[1], new Object[0])))
                        .thenApply(
                            ignored -> {
                              if (error != null) throw new CompletionException(error);
                              return value;
                            }))
            .thenCompose(Function.identity());
    return new HaraPromise(result, () -> promise.cancel());
  }

  private Object promiseDelay(Object[] values) {
    if (values.length != 2) {
      throw new HaraException("promise/delay expects milliseconds and a function");
    }
    long millis = HaraNumericConversions.toLong(values[0], "promise/delay");
    if (millis < 0) throw new HaraException("promise/delay expects non-negative milliseconds");
    CompletableFuture<Object> future = new CompletableFuture<>();
    AtomicReference<CompletableFuture<Object>> active = new AtomicReference<>();
    CompletableFuture.delayedExecutor(millis, TimeUnit.MILLISECONDS)
        .execute(
            () -> {
              if (future.isDone()) return;
              CompletableFuture<Object> source;
              try {
                source =
                    flatten(invokeInContext(() -> invokeCallable(values[1], new Object[0])));
              } catch (Throwable error) {
                future.completeExceptionally(error);
                return;
              }
              active.set(source);
              if (future.isCancelled()) {
                source.cancel(false);
                return;
              }
              source.whenComplete(
                  (value, error) -> {
                    if (error == null) future.complete(value);
                    else future.completeExceptionally(error);
                  });
            });
    return new HaraPromise(
        future,
        () -> {
          CompletableFuture<Object> source = active.get();
          if (source != null) source.cancel(false);
        });
  }

  <T> T invokeInContext(Supplier<T> operation) {
    try {
      if (HaraLanguage.currentContext() == this) return operation.get();
    } catch (IllegalStateException ignored) {
      // No Hara context is entered on this thread; enter it below.
    }
    Object previous = environment.getContext().enter(null);
    try {
      return operation.get();
    } finally {
      environment.getContext().leave(null, previous);
    }
  }

  private void requireFileIO(String operation) {
    if (!environment.isFileIOAllowed()) {
      throw new HaraException(operation + " is unsupported or file access is denied");
    }
  }

  private void requireSocketIO(String operation) {
    if (!environment.isSocketIOAllowed()) {
      throw HaraNativeCapabilityBoundary.denied(
          "Socket", HaraNativeCapabilityBoundary.method(operation), "network");
    }
  }

  private Object socketConnect(Object[] values) {
    requireSocketIO("socket/connect");
    if (values.length != 4) {
      throw new HaraException("socket/connect expects host, port, options, and callback");
    }
    String host = stringValue(values[0], "socket/connect");
    int port = HaraNumericConversions.toInt(values[1], "socket/connect");
    if (port < 1 || port > 65535) throw new HaraException("socket/connect expects a valid port");
    Object callback = values[3];
    CompletableFuture.runAsync(
        () -> {
          try {
            HaraSocket connection = new HaraSocket(new Socket());
            connection.socket.connect(new InetSocketAddress(host, port));
            connection.startDrainer();
            invokeInContext(() -> invokeCallable(callback, new Object[] {null, connection}));
          } catch (Exception error) {
            String message = error.getMessage() == null ? error.getClass().getSimpleName() : error.getMessage();
            invokeInContext(() -> invokeCallable(callback, new Object[] {message, null}));
          }
        });
    return null;
  }

  private Object socketSend(Object[] values) {
    requireSocketIO("socket/send");
    if (values.length != 2) throw new HaraException("socket/send expects a connection and bytes");
    HaraSocket connection = requireSocket(values[0], "socket/send");
    byte[] bytes = bytesValue(values[1], "socket/send");
    synchronized (connection) {
      try {
        connection.socket.getOutputStream().write(bytes);
        connection.socket.getOutputStream().flush();
        return (long) bytes.length;
      } catch (IOException error) {
        throw new HaraException("socket/send failed: " + error.getMessage());
      }
    }
  }

  private Object socketListen(Object[] values) {
    requireSocketIO("socket/listen");
    if (values.length != 4) {
      throw new HaraException("socket/listen expects host, port, options, and callback");
    }
    String host = stringValue(values[0], "socket/listen");
    int port = HaraNumericConversions.toInt(values[1], "socket/listen");
    if (port < 0 || port > 65535) throw new HaraException("socket/listen expects a valid port");
    try {
      ServerSocket listener = new ServerSocket();
      listener.bind(new InetSocketAddress(host, port));
      HaraSocketServer server = new HaraSocketServer(listener, values[3]);
      server.start();
      return server;
    } catch (IOException error) {
      throw new HaraException("socket/listen failed: " + error.getMessage());
    }
  }

  private Object socketEndpoint(Object value) {
    requireSocketIO("socket/endpoint");
    Object input = HaraBox.unwrap(value);
    if (!(input instanceof HaraSocketServer server)) {
      throw new HaraException("socket/endpoint expects a socket server");
    }
    return hara.lang.data.Map.Standard.from(
        null,
        new Object[] {Keyword.create("host"), server.host(), Keyword.create("port"), (long) server.port()});
  }

  private Object socketEvents(Object[] values) {
    requireSocketIO("socket/events");
    if (values.length != 2) throw new HaraException("socket/events expects a handle and options");
    Object input = HaraBox.unwrap(values[0]);
    if (input instanceof HaraSocket socket) return socket.events();
    if (input instanceof HaraSocketServer server) return server.events();
    throw new HaraException("socket/events expects a socket connection or server");
  }

  private Object socketNext(Object value) {
    requireSocketIO("socket/next");
    Object input = HaraBox.unwrap(value);
    if (!(input instanceof HaraSocketStream stream)) {
      throw new HaraException("socket/next expects a socket stream");
    }
    return stream.next();
  }

  private Object socketReceiveStream(Object value) {
    requireSocketIO("socket/receive-stream");
    return requireSocket(value, "socket/receive-stream").bytes();
  }

  private Object socketClose(Object value) {
    requireSocketIO("socket/close");
    Object input = HaraBox.unwrap(value);
    if (input instanceof HaraSocketServer server) {
      server.close();
      return null;
    }
    HaraSocket connection = requireSocket(value, "socket/close");
    try {
      connection.socket.close();
      return null;
    } catch (IOException error) {
      throw new HaraException("socket/close failed: " + error.getMessage());
    }
  }

  private static HaraSocket requireSocket(Object value, String operation) {
    Object input = HaraBox.unwrap(value);
    if (!(input instanceof HaraSocket)) throw new HaraException(operation + " expects a socket connection");
    return (HaraSocket) input;
  }

  @FunctionalInterface
  private interface FileEffect {
    Object invoke(HaraFileProvider provider) throws Exception;
  }

  private Object fileResolve(Object[] values) {
    requireNativeCapability("File", "resolve", "file");
    if (values.length != 2) throw new HaraException("file/resolve expects a base and path");
    String base = stringValue(values[0], "file/resolve");
    String path = stringValue(values[1], "file/resolve");
    try {
      return HaraLogicalPath.resolve(base, path);
    } catch (Throwable error) {
      throw fileFailure("resolve", base, path, error);
    }
  }

  private Object fileParent(Object value) {
    requireNativeCapability("File", "parent", "file");
    String path = stringValue(value, "file/parent");
    try {
      return HaraLogicalPath.parent(path);
    } catch (Throwable error) {
      throw fileFailure("parent", path, null, error);
    }
  }

  private Object fileJoin(Object[] values) {
    requireNativeCapability("File", "join", "file");
    if (values.length != 2) throw new HaraException("file/join expects a base and path");
    String base = stringValue(values[0], "file/join");
    String path = stringValue(values[1], "file/join");
    try {
      return HaraLogicalPath.join(base, path);
    } catch (Throwable error) {
      throw fileFailure("join", base, path, error);
    }
  }

  private Object fileRead(Object value) {
    String path = stringValue(value, "file/read");
    return fileEffect("read", path, null, binding -> binding.read(path), result -> result);
  }

  private Object fileWrite(Object[] values) {
    if (values.length < 2 || values.length > 3) {
      throw new HaraException("file/write expects a path, bytes, and optional options");
    }
    String path = stringValue(values[0], "file/write");
    byte[] contents = bytesValue(values[1], "file/write").clone();
    IMapType<?, ?> options =
        values.length == 3 ? fileOptions(values[2], "file/write") : emptyFileOptions();
    IFilesystem.WriteMode mode =
        switch (fileKeywordOption(options, "mode", "create", "file/write")) {
          case "create" -> IFilesystem.WriteMode.CREATE;
          case "replace" -> IFilesystem.WriteMode.REPLACE;
          case "append" -> IFilesystem.WriteMode.APPEND;
          default ->
              throw new HaraException(
                  "file/write :mode must be :create, :replace, or :append");
        };
    IFilesystem.WriteOptions writeOptions =
        new IFilesystem.WriteOptions(
            mode, fileBooleanOption(options, "parents?", false, "file/write"));
    IFilesystem.MutationContext mutation = fileMutationContext(options, "file/write");
    return fileEffect(
        "write",
        path,
        null,
        binding -> binding.write(path, contents, writeOptions, mutation),
        IFilesystem.Mutation::path);
  }

  private Object fileExists(Object value) {
    String path = stringValue(value, "file/exists?");
    return fileEffect("exists?", path, null, binding -> binding.exists(path), result -> result);
  }

  private Object fileStat(Object value) {
    String path = stringValue(value, "file/stat");
    return fileEffect(
        "stat", path, null, binding -> binding.stat(path), FilesystemHaraValues::entry);
  }

  private Object fileEntries(Object value) {
    String path = stringValue(value, "file/entries");
    return fileEffect(
        "entries",
        path,
        null,
        binding -> binding.entries(path),
        FilesystemHaraValues::entries);
  }

  private Object fileList(Object value) {
    String path = stringValue(value, "file/list");
    return fileEffect(
        "list", path, null, binding -> binding.list(path), FilesystemHaraValues::paths);
  }

  private Object fileWalk(Object value) {
    String path = stringValue(value, "file/walk");
    return fileEffect(
        "walk", path, null, binding -> binding.walk(path), FilesystemHaraValues::paths);
  }

  private Object fileMkdir(Object[] values) {
    if (values.length < 1 || values.length > 2) {
      throw new HaraException("file/mkdir expects a path and optional options");
    }
    String path = stringValue(values[0], "file/mkdir");
    IMapType<?, ?> options =
        values.length == 2 ? fileOptions(values[1], "file/mkdir") : emptyFileOptions();
    IFilesystem.MkdirOptions mkdirOptions =
        new IFilesystem.MkdirOptions(
            fileBooleanOption(options, "parents?", true, "file/mkdir"),
            fileBooleanOption(options, "exists-ok?", true, "file/mkdir"));
    IFilesystem.MutationContext mutation = fileMutationContext(options, "file/mkdir");
    return fileEffect(
        "mkdir",
        path,
        null,
        binding -> binding.mkdir(path, mkdirOptions, mutation),
        IFilesystem.Mutation::path);
  }

  private Object fileDelete(Object[] values) {
    if (values.length < 1 || values.length > 2) {
      throw new HaraException("file/delete expects a path and optional options");
    }
    String path = stringValue(values[0], "file/delete");
    IMapType<?, ?> options =
        values.length == 2 ? fileOptions(values[1], "file/delete") : emptyFileOptions();
    IFilesystem.DeleteOptions deleteOptions =
        new IFilesystem.DeleteOptions(
            fileBooleanOption(options, "missing-ok?", false, "file/delete"));
    IFilesystem.MutationContext mutation = fileMutationContext(options, "file/delete");
    return fileEffect(
        "delete",
        path,
        null,
        binding -> binding.delete(path, deleteOptions, mutation),
        IFilesystem.Mutation::path);
  }

  private Object fileCopy(Object[] values) {
    if (values.length < 2 || values.length > 3) {
      throw new HaraException("file/copy expects source, target, and optional options");
    }
    String source = stringValue(values[0], "file/copy");
    String target = stringValue(values[1], "file/copy");
    IMapType<?, ?> options =
        values.length == 3 ? fileOptions(values[2], "file/copy") : emptyFileOptions();
    IFilesystem.CopyOptions copyOptions =
        new IFilesystem.CopyOptions(
            fileBooleanOption(options, "replace?", false, "file/copy"),
            fileBooleanOption(options, "parents?", false, "file/copy"),
            fileBooleanOption(options, "preserve-modified?", false, "file/copy"));
    IFilesystem.MutationContext mutation = fileMutationContext(options, "file/copy");
    return fileEffect(
        "copy",
        source,
        target,
        binding -> binding.copy(source, target, copyOptions, mutation),
        IFilesystem.Mutation::path);
  }

  private Object fileMove(Object[] values) {
    if (values.length < 2 || values.length > 3) {
      throw new HaraException("file/move expects source, target, and optional options");
    }
    String source = stringValue(values[0], "file/move");
    String target = stringValue(values[1], "file/move");
    IMapType<?, ?> options =
        values.length == 3 ? fileOptions(values[2], "file/move") : emptyFileOptions();
    IFilesystem.MoveOptions moveOptions =
        new IFilesystem.MoveOptions(
            fileBooleanOption(options, "replace?", false, "file/move"),
            fileBooleanOption(options, "parents?", false, "file/move"),
            fileBooleanOption(options, "atomic?", false, "file/move"));
    IFilesystem.MutationContext mutation = fileMutationContext(options, "file/move");
    return fileEffect(
        "move",
        source,
        target,
        binding -> binding.move(source, target, moveOptions, mutation),
        IFilesystem.Mutation::path);
  }

  private Object fileTempFile(Object[] values) {
    if (values.length < 1 || values.length > 2) {
      throw new HaraException("file/temp-file expects a parent and optional options");
    }
    String parent = stringValue(values[0], "file/temp-file");
    IMapType<?, ?> options =
        values.length == 2 ? fileOptions(values[1], "file/temp-file") : emptyFileOptions();
    String prefix = fileStringOption(options, "prefix", "tmp", "file/temp-file");
    String suffix = fileStringOption(options, "suffix", "", "file/temp-file");
    return fileEffect(
        "temp-file",
        parent,
        null,
        binding -> binding.tempFile(parent, prefix, suffix),
        result -> result);
  }

  private Object fileTempDirectory(Object[] values) {
    if (values.length < 1 || values.length > 2) {
      throw new HaraException("file/temp-directory expects a parent and optional options");
    }
    String parent = stringValue(values[0], "file/temp-directory");
    IMapType<?, ?> options =
        values.length == 2
            ? fileOptions(values[1], "file/temp-directory")
            : emptyFileOptions();
    String prefix = fileStringOption(options, "prefix", "tmp", "file/temp-directory");
    return fileEffect(
        "temp-directory",
        parent,
        null,
        binding -> binding.tempDirectory(parent, prefix),
        result -> result);
  }

  private <T> Object fileEffect(
      String operation,
      String path,
      String target,
      Function<FilesystemRuntimeBinding, FilesystemRuntimeBinding.Pending<T>> effect,
      Function<? super T, ?> transform) {
    if (filesystemRuntime == null) {
      return rejectedNativeCapabilityPromise("File", operation, "file");
    }
    try {
      return FilesystemPromiseBridge.bind(
          this,
          effect.apply(filesystemRuntime),
          transform,
          operation,
          path,
          target);
    } catch (Throwable error) {
      return rejectedFilePromise(operation, path, target, error);
    }
  }

  private Object rejectedFilePromise(
      String operation, String path, String target, Throwable error) {
    CompletableFuture<Object> future = new CompletableFuture<>();
    future.completeExceptionally(fileFailure(operation, path, target, error));
    return cancellablePromise(future, () -> {});
  }

  @SuppressWarnings("unchecked")
  private static IMapType<?, ?> emptyFileOptions() {
    return hara.lang.data.Map.Standard.EMPTY;
  }

  private static IMapType<?, ?> fileOptions(Object value, String operation) {
    Object raw = HaraBox.unwrap(value);
    if (raw == null) return emptyFileOptions();
    if (!(raw instanceof IMapType<?, ?> map)) {
      throw new HaraException(operation + " options must be a map");
    }
    return map;
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  private static Object fileOption(IMapType<?, ?> options, String name, Object defaultValue) {
    Object found = ((IMapType) options).find(Keyword.create(name));
    java.util.Map.Entry entry = (java.util.Map.Entry) found;
    return entry == null ? defaultValue : HaraBox.unwrap(entry.getValue());
  }

  private static boolean fileBooleanOption(
      IMapType<?, ?> options, String name, boolean defaultValue, String operation) {
    Object value = fileOption(options, name, defaultValue);
    if (value instanceof Boolean bool) return bool;
    throw new HaraException(operation + " :" + name + " must be boolean");
  }

  private static String fileStringOption(
      IMapType<?, ?> options, String name, String defaultValue, String operation) {
    Object value = fileOption(options, name, defaultValue);
    if (value instanceof String string) return string;
    throw new HaraException(operation + " :" + name + " must be a string");
  }

  private static String fileKeywordOption(
      IMapType<?, ?> options, String name, String defaultValue, String operation) {
    Object value = fileOption(options, name, Keyword.create(defaultValue));
    if (value instanceof Keyword keyword) return keyword.getName();
    throw new HaraException(operation + " :" + name + " must be a keyword");
  }

  private static IFilesystem.MutationContext fileMutationContext(
      IMapType<?, ?> options, String operation) {
    return new IFilesystem.MutationContext(
        fileOptionalStringOption(options, "expected-revision", operation),
        fileOptionalStringOption(options, "expected-target-revision", operation));
  }

  private static String fileOptionalStringOption(
      IMapType<?, ?> options, String name, String operation) {
    Object value = fileOption(options, name, null);
    if (value == null || value instanceof String) return (String) value;
    throw new HaraException(operation + " :" + name + " must be a string");
  }

  static hara.lang.base.Ex.Info fileFailure(
      String operation, String path, String target, Throwable error) {
    Throwable cause = HaraFileProvider.unwrap(error);
    if (cause instanceof hara.lang.base.Ex.Info info) return info;
    FilesystemException filesystem =
        cause instanceof FilesystemException failure ? failure : null;
    String code = filesystem == null ? HaraFileProvider.code(cause) : filesystem.code();
    String effectiveOperation =
        filesystem != null && filesystem.operation() != null
            ? filesystem.operation()
            : operation;
    String effectivePath =
        filesystem != null && filesystem.path() != null ? filesystem.path() : path;
    String effectiveTarget =
        filesystem != null && filesystem.target() != null ? filesystem.target() : target;
    ArrayList<Object> fields = new ArrayList<>();
    Collections.addAll(
        fields,
        Keyword.create("ex", "code"),
        Keyword.create("file", code),
        Keyword.create("ex", "class"),
        fileErrorClass(code),
        Keyword.create("file", "operation"),
        Keyword.create(effectiveOperation),
        Keyword.create("file", "path"),
        canonicalFilePath(effectivePath),
        Keyword.create("file", "target"),
        canonicalFilePath(effectiveTarget));
    if (filesystem != null) {
      Collections.addAll(
          fields,
          Keyword.create("file", "provider"),
          filesystem.provider(),
          Keyword.create("file", "provider-code"),
          filesystem.providerCode(),
          Keyword.create("file", "retryable?"),
          filesystem.retryable());
    }
    IMetadata data = hara.lang.data.Map.Standard.from(null, fields.toArray());
    String message = cause.getMessage();
    if (message == null || message.isBlank()) message = cause.getClass().getSimpleName();
    return new hara.lang.base.Ex.Info(
        "file/" + effectiveOperation + " failed: " + message, data, cause);
  }

  private static Keyword fileErrorClass(String code) {
    return switch (code) {
      case "not-found" -> Keyword.create("ex.class", "not-found");
      case "already-exists", "directory-not-empty", "conflict", "ambiguous-path" ->
          Keyword.create("ex.class", "conflict");
      case "denied", "permission-denied", "outside-root", "auth-required",
          "authentication-failed" -> Keyword.create("ex.class", "security");
      case "invalid-path" -> Keyword.create("ex.class", "argument");
      case "timeout" -> Keyword.create("ex.class", "timeout");
      default -> Keyword.create("ex.class", "io");
    };
  }

  private static Object canonicalFilePath(String value) {
    if (value == null) return null;
    try {
      return HaraLogicalPath.normalise(value);
    } catch (Throwable ignored) {
      return value;
    }
  }

  @TruffleBoundary
  public Object invokeMarkerMethod(Object receiverValue, String method, Object[] arguments) {
    Object receiver = HaraBox.unwrap(receiverValue);
    if (receiver instanceof HaraArray || receiver instanceof HaraObject) {
      throw new HaraException(
          "Dot calls do not support arrays or objects; use Arr/ or Obj/ functions");
    }
    NativeFlavorProvider provider = nativeProvider();
    if (provider == null) {
      throw new HaraException(
          "Dot calls are only supported on values created by array or object unless the namespace selects a native flavor");
    }
    return provider.invokeMember(receiver, method, arguments, nativeAccess());
  }

  @TruffleBoundary
  Object executeBytecodeDeclaration(String expectedOperator, Object form) {
    Object raw = HaraBox.unwrap(form);
    if (!(raw instanceof hara.lang.data.List<?> list)
        || list.count() == 0
        || !(list.nth(0) instanceof Symbol operator)
        || operator.getNamespace() != null
        || !expectedOperator.equals(operator.getName())) {
      throw new HaraException(
          expectedOperator + " instruction contains the wrong declaration");
    }
    Source source = Source.newBuilder(HaraLanguage.ID, "", "<hbc-" + expectedOperator + ">")
        .build();
    return HaraAnalyzer.compile(
            HaraLanguage.currentLanguage(),
            new Object[] {raw},
            source.createUnavailableSection(),
            this)
        .call();
  }

  private Object nativeMutableCall(String type, String method, Object[] values) {
    if ("new".equals(method)) {
      Object[] unwrapped =
          java.util.Arrays.stream(values).map(HaraBox::unwrap).toArray(Object[]::new);
      return "Arr".equals(type) ? new HaraArray(unwrapped) : new HaraObject(unwrapped);
    }
    if (values.length == 0) {
      throw new HaraException("std.native." + type + "/" + method + " expects a receiver");
    }
    Object receiver = HaraBox.unwrap(values[0]);
    Object[] arguments = new Object[values.length - 1];
    for (int index = 1; index < values.length; index++) {
      arguments[index - 1] = HaraBox.unwrap(values[index]);
    }
    if ("Arr".equals(type) && receiver instanceof HaraArray array) {
      return invokeArrayMethod(array, method, arguments);
    }
    if ("Obj".equals(type) && receiver instanceof HaraObject object) {
      return invokeObjectMethod(object, method, arguments);
    }
    throw new HaraException("std.native." + type + "/" + method + " receiver type mismatch");
  }

  private Object invokeArrayMethod(HaraArray array, String method, Object[] arguments) {
    switch (method) {
      case "get":
        requireMethodArity(method, arguments, 1);
        return array.get(arrayIndex(arguments[0], array.size(), false, method));
      case "set":
        requireMethodArity(method, arguments, 2);
        array.set(arrayIndex(arguments[0], array.size(), false, method), arguments[1]);
        return array;
      case "push-last":
        requireMethodArity(method, arguments, 1);
        array.add(arguments[0]);
        return array;
      case "pop-last":
        requireMethodArity(method, arguments, 0);
        if (array.isEmpty()) return null;
        return array.remove(array.size() - 1);
      case "push-first":
        requireMethodArity(method, arguments, 1);
        array.add(0, arguments[0]);
        return array;
      case "pop-first":
        requireMethodArity(method, arguments, 0);
        if (array.isEmpty()) return null;
        return array.remove(0);
      case "insert":
        requireMethodArity(method, arguments, 2);
        array.add(arrayIndex(arguments[0], array.size(), true, method), arguments[1]);
        return array;
      case "remove":
        requireMethodArity(method, arguments, 1);
        return array.remove(arrayIndex(arguments[0], array.size(), false, method));
      case "clone":
        requireMethodArity(method, arguments, 0);
        return new HaraArray(array.toArray());
      case "slice":
        {
          if (arguments.length < 1 || arguments.length > 2) {
            throw new HaraException("array.slice expects a start and optional end");
          }
          int start = arrayIndex(arguments[0], array.size(), true, method);
          int end =
              arguments.length == 2
                  ? arrayIndex(arguments[1], array.size(), true, method)
                  : array.size();
          if (end < start) throw new HaraException("array.slice range is out of bounds");
          return new HaraArray(array.subList(start, end).toArray());
        }
      case "map":
        {
          requireMethodArity(method, arguments, 1);
          HaraArray output = new HaraArray();
          for (Object value : array) output.add(invokeCallable(arguments[0], new Object[] {value}));
          return output;
        }
      case "filter":
        {
          requireMethodArity(method, arguments, 1);
          HaraArray output = new HaraArray();
          for (Object value : array) {
            if (truthy(invokeCallable(arguments[0], new Object[] {value}))) output.add(value);
          }
          return output;
        }
      case "fold-left":
      case "fold-right":
        requireMethodArity(method, arguments, 2);
        Object result = arguments[1];
        if ("fold-left".equals(method)) {
          for (Object value : array) {
            result = invokeCallable(arguments[0], new Object[] {result, value});
          }
        } else {
          for (int i = array.size() - 1; i >= 0; i--) {
            result = invokeCallable(arguments[0], new Object[] {array.get(i), result});
          }
        }
        return result;
      default:
        throw new HaraException("Unsupported array method: " + method);
    }
  }

  private Object invokeObjectMethod(HaraObject object, String method, Object[] arguments) {
    switch (method) {
      case "has?":
        requireMethodArity(method, arguments, 1);
        return object.containsKey(objectKey(arguments[0], method));
      case "get":
        if (arguments.length < 1 || arguments.length > 2) {
          throw new HaraException("object.get expects a key and optional fallback");
        }
        String key = objectKey(arguments[0], method);
        return object.containsKey(key)
            ? object.get(key)
            : arguments.length == 2 ? arguments[1] : null;
      case "set":
        requireMethodArity(method, arguments, 2);
        object.put(objectKey(arguments[0], method), arguments[1]);
        return object;
      case "delete":
        requireMethodArity(method, arguments, 1);
        return object.remove(objectKey(arguments[0], method));
      case "clone":
        requireMethodArity(method, arguments, 0);
        return new HaraObject(object);
      case "assign":
        requireMethodArity(method, arguments, 1);
        Object source = HaraBox.unwrap(arguments[0]);
        if (!(source instanceof HaraObject)) {
          throw new HaraException("object.assign expects an object marker");
        }
        object.putAll((HaraObject) source);
        return object;
      case "keys":
        requireMethodArity(method, arguments, 0);
        return new HaraArray(object.keySet().toArray());
      case "vals":
        requireMethodArity(method, arguments, 0);
        return new HaraArray(object.values().toArray());
      case "pairs":
        {
          requireMethodArity(method, arguments, 0);
          HaraArray pairs = new HaraArray();
          for (Map.Entry<String, Object> entry : object.entrySet()) {
            pairs.add(new HaraArray(new Object[] {entry.getKey(), entry.getValue()}));
          }
          return pairs;
        }
      default:
        throw new HaraException("Unsupported object method: " + method);
    }
  }

  static void requireMethodArity(String method, Object[] arguments, int expected) {
    if (arguments.length != expected) {
      throw new HaraException(method + " expects " + expected + " arguments");
    }
  }

  private static int arrayIndex(Object value, int size, boolean allowEnd, String operation) {
    int index = HaraNumericConversions.toInt(value, operation);
    if (index < 0 || index > size || (!allowEnd && index == size)) {
      throw new HaraException(operation + " index out of bounds: " + index);
    }
    return index;
  }

  private static String objectKey(Object value, String operation) {
    Object input = HaraBox.unwrap(value);
    if (!(input instanceof String)) {
      throw new HaraException("object." + operation + " expects a string key");
    }
    return (String) input;
  }

  @TruffleBoundary
  private Object arithmetic(String operator, Object[] values) {
    if (operator.equals("+") && values.length == 0) return 0L;
    if (operator.equals("*") && values.length == 0) return 1L;
    if (values.length == 0) {
      throw new HaraException(operator + " expects at least one number");
    }
    if ((operator.equals("quot") || operator.equals("rem") || operator.equals("mod"))
        && values.length != 2) {
      throw new HaraException(operator + " expects two numbers");
    }
    for (Object value : values) {
      if (!(HaraBox.unwrap(value) instanceof Number)) {
        throw new HaraException(operator + " expects two numbers");
      }
    }
    if (operator.equals("-") && values.length == 1) {
      return Num.minusP(values[0]);
    }
    if (operator.equals("/") && values.length == 1) {
      return Num.divide(1L, values[0]);
    }
    Object result = values[0];
    for (int i = 1; i < values.length; i++) {
      Object value = values[i];
      if (!(HaraBox.unwrap(result) instanceof Number)
          || !(HaraBox.unwrap(value) instanceof Number)) {
        throw new HaraException(operator + " expects two numbers");
      }
      if (operator.equals("+")) {
        result = Num.addP(result, value);
      } else if (operator.equals("-")) {
        result = Num.minusP(result, value);
      } else if (operator.equals("*")) {
        result = Num.multiplyP(result, value);
      } else if (operator.equals("/")) {
        result = Num.divide(result, value);
      } else if (operator.equals("quot")) {
        result = Num.quotient(result, value);
      } else if (operator.equals("rem")) {
        result = Num.remainder(result, value);
      } else if (operator.equals("mod")) {
        result = Num.mod(result, value);
      } else {
        throw new HaraException("Unknown arithmetic operator: " + operator);
      }
    }
    return result;
  }

  @TruffleBoundary
  private Object compare(String operator, Object[] values) {
    if (values.length < 2) {
      throw new HaraException(operator + " expects at least two arguments");
    }
    Object previous = HaraBox.unwrap(values[0]);
    for (int i = 1; i < values.length; i++) {
      Object current = HaraBox.unwrap(values[i]);
      boolean matches;
      if (operator.equals("=") || operator.equals("not=")) {
        boolean equal = Eq.eq(previous, current);
        if (operator.equals("not=") && !equal) return true;
        matches = equal;
      } else {
        int comparison = compareValue(previous, current);
        if (operator.equals("<")) matches = comparison < 0;
        else if (operator.equals("<=")) matches = comparison <= 0;
        else if (operator.equals(">")) matches = comparison > 0;
        else if (operator.equals(">=")) matches = comparison >= 0;
        else throw new HaraException("Unknown comparison operator: " + operator);
      }
      if (!matches) return false;
      previous = current;
    }
    return !operator.equals("not=");
  }

  private int compareValue(Object left, Object right) {
    if (Eq.eq(left, right)) return 0;
    if (left instanceof Number a && right instanceof Number b) return Num.compare(a, b);
    if (left instanceof String a && right instanceof String b) return a.compareTo(b);
    if (left instanceof HaraCharacter a && right instanceof HaraCharacter b)
      return a.compareTo(b);
    if (left instanceof HaraCharacter a && right instanceof Character b)
      return Integer.compare(a.codePoint(), b.charValue());
    if (left instanceof Character a && right instanceof HaraCharacter b)
      return Integer.compare(a.charValue(), b.codePoint());
    if (left instanceof Character a && right instanceof Character b) return a.compareTo(b);
    if (left instanceof Keyword a && right instanceof Keyword b) return a.compareTo(b);
    if (left instanceof Symbol a && right instanceof Symbol b) {
      return a.display().compareTo(b.display());
    }
    if (left instanceof Boolean a && right instanceof Boolean b) return a.compareTo(b);
    if (left instanceof ILinearType<?> a && right instanceof ILinearType<?> b) {
      Iterator<?> ai = a.iterator();
      Iterator<?> bi = b.iterator();
      while (ai.hasNext() && bi.hasNext()) {
        int result = compareValue(ai.next(), bi.next());
        if (result != 0) return result;
      }
      return Boolean.compare(ai.hasNext(), bi.hasNext());
    }
    throw new HaraException("compare expects two mutually orderable Hara values");
  }

  private Object protocolCall(String protocolName, String methodName, Object[] values) {
    if (values.length == 0) {
      throw new HaraException(methodName + " expects a collection value");
    }
    HaraVar variable = resolve(Symbol.create(protocolName));
    if (variable == null || !(variable.get() instanceof HaraProtocol)) {
      throw new HaraException("Missing protocol: " + protocolName);
    }
    Object receiver = HaraBox.unwrap(values[0]);
    if (isHostObject(receiver)) {
      receiver = asHostObject(receiver);
    }
    Object[] arguments = new Object[values.length - 1];
    System.arraycopy(values, 1, arguments, 0, arguments.length);
    if (receiver instanceof hara.lang.data.Pointer pointer) {
      if ("IDeref".equals(protocolName) && "deref".equals(methodName)) {
        requireMethodArity("IDeref/deref", arguments, 0);
        return pointerDeref(pointer);
      }
      if ("IApplicable".equals(protocolName)) {
        if ("apply-default".equals(methodName)) {
          requireMethodArity("IApplicable/apply-default", arguments, 0);
          return pointerDefault(pointer);
        }
        if ("apply-in".equals(methodName)) {
          requireMethodArity("IApplicable/apply-in", arguments, 2);
          return pointerContextCall(
              pointer,
              HaraBox.unwrap(arguments[0]),
              "pointer/invoke",
              sequentialValues(arguments[1], "IApplicable/apply-in"));
        }
        if ("transform-in".equals(methodName)) {
          requireMethodArity("IApplicable/transform-in", arguments, 2);
          return arguments[1];
        }
        if ("transform-out".equals(methodName)) {
          requireMethodArity("IApplicable/transform-out", arguments, 3);
          return arguments[2];
        }
      }
      if ("IInvokeIn".equals(protocolName) && "invoke-in".equals(methodName)) {
        if (arguments.length < 1) {
          throw new HaraException("IInvokeIn/invoke-in expects a pointer and runtime");
        }
        Object[] invokeArguments = new Object[arguments.length - 1];
        System.arraycopy(arguments, 1, invokeArguments, 0, invokeArguments.length);
        return pointerContextCall(
            pointer, HaraBox.unwrap(arguments[0]), "pointer/invoke", invokeArguments);
      }
    }
    return ((HaraProtocol) variable.get()).invoke(methodName, receiver, arguments);
  }

  private hara.lang.data.Pointer requirePointer(Object value, String operation) {
    if (value instanceof hara.lang.data.Pointer pointer) return pointer;
    throw new HaraException(operation + " expects one pointer");
  }

  private Object[] sequentialValues(Object value, String operation) {
    Object unwrapped = HaraBox.unwrap(value);
    if (!(unwrapped instanceof ILinearType<?> sequence)) {
      throw new HaraException(operation + " expects sequential arguments");
    }
    Object[] result = new Object[(int) sequence.count()];
    for (int index = 0; index < result.length; index++) result[index] = sequence.nth(index);
    return result;
  }

  private Object pointerDefault(hara.lang.data.Pointer pointer) {
    try {
      HaraNamespace space = requiredNamespace("std.lib.context.space");
      HaraVar resolver = space == null ? null : space.lookup("space:rt-current");
      if (resolver == null) throw new HaraException("std.lib.context.space is unavailable");
      return invokeCallable(resolver, new Object[] {pointer.context()});
    } catch (RuntimeException error) {
      String message = error.getMessage() == null ? error.getClass().getSimpleName() : error.getMessage();
      if (message.startsWith("pointer/runtime-unavailable:")) throw error;
      throw HaraException.withCause("pointer/runtime-unavailable: " + message, error);
    }
  }

  private Object pointerDeref(hara.lang.data.Pointer pointer) {
    return pointerContextCall(pointer, pointerDefault(pointer), "pointer/deref", new Object[0]);
  }

  private Object pointerContextCall(
      hara.lang.data.Pointer pointer, Object runtime, String operation, Object[] arguments) {
    Object[] call = new Object[arguments.length + 3];
    call[0] = runtime;
    call[1] = Keyword.create(operation);
    call[2] = pointer;
    System.arraycopy(arguments, 0, call, 3, arguments.length);
    return protocolCall("IContext", "call", call);
  }

  public Object invokeProtocol(String protocolName, String methodName, Object... values) {
    return protocolCall(protocolName, methodName, values);
  }

  void publishWholeWasmProtocolCall(
      String target,
      int arity,
      HaraTargetRuntime.ResultMode resultMode,
      String status) {
    instrumentationRuntime.publishWholeWasmProtocolCall(target, arity, resultMode, status);
  }

  /**
   * The canonical builtin instance currently specializing simple-name collection operations
   * (get/nth), or null when the operation has no specialized node support. Specialized
   * nodes compare the operator var's current value against this instance on every call and fall
   * back to a generic invocation when it differs (e.g. after redefinition).
   */
  public Object intrinsicCollectionBuiltin(String name) {
    return intrinsicCollectionBuiltins.get(name);
  }

  /**
   * The canonical std.foundation first/rest function captured when the namespace source was
   * (re)loaded, or null when unavailable. Specialized nodes compare the operator var's current
   * value against this instance on every call and fall back to a generic invocation when it
   * differs (e.g. after redefinition).
   */
  public Object intrinsicSequenceFunction(String name) {
    return "first".equals(name) ? intrinsicFirstFunction : intrinsicRestFunction;
  }

  /**
   * The exact (seq (iter-drop 1 source)) expansion for a receiver already coerced to an
   * iterator: a lazy seq over the tail, or null when the tail is empty.
   */
  public Object restSequence(Iterator<?> source) {
    return hara.lang.data.Seq.create(closeable(Iter.drop(source, 1), source));
  }

  private void captureSequenceIntrinsics() {
    HaraNamespace foundation = namespaces.get(FOUNDATION_NAMESPACE);
    HaraVar firstVariable = foundation == null ? null : foundation.lookup("first");
    HaraVar restVariable = foundation == null ? null : foundation.lookup("rest");
    intrinsicFirstFunction = firstVariable == null ? null : firstVariable.deref();
    intrinsicRestFunction = restVariable == null ? null : restVariable.deref();
  }

  private void requireHalPath(String path, String operation) {
    String sourcePath = path.startsWith("classpath:") ? path.substring(10) : path;
    if (!sourcePath.endsWith(".hal")
        && !sourcePath.endsWith(".hrl")
        && !sourcePath.endsWith(".hbx")) {
      throw new HaraException(
          operation + " accepts only .hal, .hrl, or .hbx executable resources");
    }
  }

  private Object loadString(Object value) {
    if (!(value instanceof String)) {
      throw new HaraException("load-string expects a string");
    }
    ContextSnapshot snapshot = snapshot();
    try {
      return parseAndExecute((String) value, "<string>");
    } catch (RuntimeException error) {
      restore(snapshot);
      throw HaraException.withCause(
          "Unable to evaluate Hara source: " + error.getMessage(), error);
    }
  }

  private Object readString(Object value) {
    if (!(value instanceof String source)) {
      throw new HaraException("read-string expects a string");
    }
    try {
      return Parser.LispReader.readString(source, null);
    } catch (RuntimeException error) {
      throw HaraException.withCause("read-string failed: " + error.getMessage(), error);
    }
  }

  private Object evalForm(Object value) {
    return evaluationRuntime.evalForm(value, "<eval>");
  }

  Object executeToolVmHalc(HalcArtifact.Module module) {
    ContextSnapshot snapshot = snapshot();
    try {
      return HaraLanguage.compileHalc(module, "tool.vm/execute").call();
    } catch (RuntimeException error) {
      restore(snapshot);
      throw error;
    }
  }

  Object executeToolVmHbc(hara.truffle.bytecode.HbcProgram program) {
    ContextSnapshot snapshot = snapshot();
    try {
      return HbcMachine.execute(program, this);
    } catch (RuntimeException error) {
      restore(snapshot);
      throw error;
    }
  }

  private Object loadFile(Object value) {
    if (!(value instanceof String)) {
      throw new HaraException("load-file expects a path string");
    }
    requireHalPath((String) value, "load-file");
    requireFileIO("load-file");
    ContextSnapshot snapshot = snapshot();
    try {
      String path = canonicalPath((String) value);
      Object result =
          parseAndExecute(
              new String(
                  environment.getPublicTruffleFile(path).readAllBytes(), StandardCharsets.UTF_8),
              path);
      registerModule(path);
      return result;
    } catch (IOException | RuntimeException error) {
      restore(snapshot);
      throw HaraException.withCause(
          "Unable to load Hara file: " + value + " (" + error.getMessage() + ")", error);
    }
  }

  @TruffleBoundary
  private Object readForms(Object[] values) {
    requireMethodArity("read-forms", values, 1);
    Object value = HaraBox.unwrap(values[0]);
    if (!(value instanceof String)) {
      throw new HaraException("read-forms expects a path string");
    }
    requireHalPath((String) value, "read-forms");
    requireFileIO("read-forms");
    String path = canonicalPath((String) value);
    try {
      String source =
          new String(
              environment.getPublicTruffleFile(path).readAllBytes(), StandardCharsets.UTF_8);
      return hara.lang.data.Vector.Standard.from(null, HaraLanguage.readAll(source, path));
    } catch (IOException | RuntimeException error) {
      if (error instanceof HaraException) throw (HaraException) error;
      throw new HaraException(
          "Unable to read Hara forms: " + value + " (" + error.getMessage() + ")");
    }
  }

  private String namespaceIdentifier(Object value, String operation) {
    Object unwrapped = unwrapQuoted(HaraBox.unwrap(value));
    if (unwrapped instanceof HaraNamespace namespace) return namespace.name();
    if (unwrapped instanceof Symbol symbol) return symbol.display();
    if (unwrapped instanceof String name) return name;
    throw new HaraException(operation + " expects a namespace symbol or string");
  }

  private Object namespaceFind(Object value) {
    String name = namespaceIdentifier(value, "ns-find");
    if (sandboxRestricted && sandboxForbiddenNamespace(name)) return null;
    return namespaces.get(name);
  }

  private Object namespaceCreate(Object value) {
    Object unwrapped = unwrapQuoted(HaraBox.unwrap(value));
    if (!(unwrapped instanceof Symbol symbol) || symbol.getNamespace() != null) {
      throw new HaraException("ns-create expects an unqualified symbol");
    }
    if (sandboxRestricted && sandboxForbiddenNamespace(symbol.display())) return null;
    return namespace(symbol.display());
  }

  private Object namespaceName(Object value) {
    String name = namespaceIdentifier(value, "ns-name");
    if (sandboxRestricted && sandboxForbiddenNamespace(name)) {
      throw new HaraException("No such namespace: " + name);
    }
    if (!namespaces.containsKey(name)) {
      throw new HaraException("No such namespace: " + name);
    }
    return Symbol.create(name);
  }

  private Object internVar(Object[] values) {
    if (values.length != 3 && values.length != 4) {
      throw new HaraException("intern-var expects namespace, symbol, var, and optional metadata");
    }
    String namespaceName = namespaceIdentifier(values[0], "intern-var");
    Object rawSymbol = HaraBox.unwrap(values[1]);
    if (!(rawSymbol instanceof Symbol symbol) || symbol.getNamespace() != null) {
      throw new HaraException("intern-var expects an unqualified target symbol");
    }
    Object rawVar = HaraBox.unwrap(values[2]);
    if (!(rawVar instanceof HaraVar source)) {
      throw new HaraException("intern-var expects a source Var");
    }
    IMetadata metadata = source.meta();
    if (values.length == 4 && values[3] != null) {
      Object extension = HaraBox.unwrap(values[3]);
      if (!(extension instanceof IMapType<?, ?> extra)) {
        throw new HaraException("intern-var metadata extension must be a map");
      }
      java.util.ArrayList<Object> entries = new java.util.ArrayList<>();
      if (metadata instanceof IMapType<?, ?> sourceMetadata) {
        for (Object entry : sourceMetadata) {
          java.util.Map.Entry<?, ?> pair = (java.util.Map.Entry<?, ?>) entry;
          entries.add(pair.getKey());
          entries.add(pair.getValue());
        }
      }
      for (Object entry : extra) {
        java.util.Map.Entry<?, ?> pair = (java.util.Map.Entry<?, ?>) entry;
        entries.add(pair.getKey());
        entries.add(pair.getValue());
      }
      metadata = hara.lang.data.Map.Standard.from(null, entries.toArray());
    }
    Object value = source.get();
    HaraVar imported =
        namespace(namespaceName)
            .define(symbol.getName(), value, metadata, HaraVar.Origin.SOURCE);
    if (value instanceof HaraMacro macro) {
      macros
          .computeIfAbsent(namespaceName, ignored -> new ConcurrentHashMap<>())
          .put(symbol.getName(), macro);
    }
    return imported;
  }

  private Object namespaceState(Object value) {
    String name = namespaceIdentifier(value, "ns-state");
    if (sandboxRestricted && sandboxForbiddenNamespace(name)) return Keyword.create("unknown");
    NamespaceLoadState state = namespaceStates.get(name);
    return Keyword.create(state == null ? "unknown" : state.keyword);
  }

  private Object namespaceLoaded(Object value) {
    String name = namespaceIdentifier(value, "ns-loaded?");
    if (sandboxRestricted && sandboxForbiddenNamespace(name)) return false;
    return namespaceStates.get(name)
        == NamespaceLoadState.LOADED;
  }

  private Object namespaceAliasState(Object[] values) {
    if (values.length != 1 && values.length != 2) {
      throw new HaraException("ns-alias-state expects alias or namespace and alias");
    }
    String owner = currentNamespace.name();
    Object aliasValue = values[0];
    if (values.length == 2) {
      owner = namespaceIdentifier(values[0], "ns-alias-state");
      aliasValue = values[1];
    }
    Object rawAlias = HaraBox.unwrap(aliasValue);
    if (!(rawAlias instanceof Symbol alias) || alias.getNamespace() != null) {
      throw new HaraException("ns-alias-state expects an unqualified alias symbol");
    }
    String target = aliases.getOrDefault(owner, Map.of()).get(alias.getName());
    if (target == null) return null;
    return hara.lang.data.Map.Standard.from(
        null,
        new Object[] {
          Keyword.create("alias"), Symbol.create(alias.getName()),
          Keyword.create("target"), Symbol.create(target),
          Keyword.create("state"), namespaceState(Symbol.create(target))
        });
  }

  private Object evalInNamespace(Object[] values) {
    if (values.length != 2) throw new HaraException("eval-in-ns expects namespace and forms");
    String target = namespaceIdentifier(values[0], "eval-in-ns");
    if (!namespaces.containsKey(target)) {
      throw new HaraException("eval-in-ns requires an existing namespace: " + target);
    }
    Object forms = HaraBox.unwrap(values[1]);
    if (!(forms instanceof ILinearType<?>)) {
      throw new HaraException("eval-in-ns expects a vector or list of forms");
    }
    HaraNamespace previous = currentNamespace;
    try {
      currentNamespace = namespaces.get(target);
      Object result = null;
      for (Object form : (ILinearType<?>) forms) {
        result = evaluationRuntime.evalForm(form, "<with-ns>");
      }
      return result;
    } finally {
      currentNamespace = previous;
    }
  }

  @TruffleBoundary
  private Object namespacePublics(Object value) {
    String name = namespaceIdentifier(value, "ns-publics");
    if (sandboxRestricted && sandboxForbiddenNamespace(name)) {
      throw new HaraException("No such namespace: " + name);
    }
    HaraNamespace target = namespaces.get(name);
    if (target == null) throw new HaraException("No such namespace: " + name);
    ArrayList<Object> entries = new ArrayList<>();
    for (String symbolName : target.sortedSymbolNames()) {
      entries.add(Symbol.create(symbolName));
      entries.add(target.lookup(symbolName));
    }
    return hara.lang.data.OrderedMap.Standard.from(null, entries.toArray());
  }

  @TruffleBoundary
  private Object namespaceAliases(Object value) {
    String name = namespaceIdentifier(value, "ns-aliases");
    if (sandboxRestricted && sandboxForbiddenNamespace(name)) {
      throw new HaraException("No such namespace: " + name);
    }
    if (!namespaces.containsKey(name)) throw new HaraException("No such namespace: " + name);
    ArrayList<Object> entries = new ArrayList<>();
    aliases.getOrDefault(name, Map.of()).entrySet().stream()
        .sorted(Map.Entry.comparingByKey())
        .forEach(
            entry -> {
              entries.add(Symbol.create(entry.getKey()));
              entries.add(Symbol.create(entry.getValue()));
            });
    return hara.lang.data.OrderedMap.Standard.from(null, entries.toArray());
  }

  @TruffleBoundary
  private Object loadResource(Object value) {
    if (!(value instanceof String) || ((String) value).isEmpty()) {
      throw new HaraException("load-resource expects a non-empty resource name");
    }
    String resourceName = (String) value;
    requireHalPath(resourceName, "load-resource");
    ContextSnapshot snapshot = snapshot();
    try {
      ensureFoundationResource(resourceName);
      String foundationNamespace = foundationResourceNamespace(resourceName);
      if (resourceName.endsWith(".hbx") && foundationNamespace != null) {
        if (FOUNDATION_NAMESPACE.equals(foundationNamespace)) ensureEagerFallbacks();
        else requiredNamespace(foundationNamespace);
        return null;
      }
      FoundationHalcLoader.Attempt hir = FoundationHalcLoader.load(resourceName);
      if (hir.loaded) return hir.value;
      try (InputStream input =
          HaraContext.class.getClassLoader().getResourceAsStream(resourceName)) {
        if (input == null) {
          throw new HaraException("Unable to find Hara resource: " + value);
        }
        return parseAndExecute(
            new String(input.readAllBytes(), StandardCharsets.UTF_8), "classpath:" + resourceName);
      } catch (IOException error) {
        throw HaraException.withCause(
            "Unable to load Hara resource: " + value + " (" + error.getMessage() + ")", error);
      }
    } catch (RuntimeException error) {
      restore(snapshot);
      throw error;
    }
  }

  @TruffleBoundary
  public Object requireModule(Object[] arguments) {
    if (arguments.length == 1 || arguments.length == 2) {
      Object requestedNamespace = unwrapQuoted(arguments[0]);
      if (requestedNamespace instanceof Symbol) {
        return requireNamespace((Symbol) requestedNamespace, arguments.length == 2 ? arguments[1] : null);
      }
    }
    if (arguments.length < 1 || arguments.length > 2 || !(arguments[0] instanceof String)) {
      throw new HaraException("require expects a path string or namespace symbol");
    }
    String callerNamespace = currentNamespace.name();
    try {
      String requested = (String) arguments[0];
      requireHalPath(requested, "require");
      boolean classpath = requested.startsWith("classpath:") || getResource(requested) != null;
      String resourceName =
          requested.startsWith("classpath:") ? requested.substring(10) : requested;
      String key = classpath ? "classpath:" + resourceName : canonicalPath(requested);
      boolean reload =
          arguments.length == 2 && requireOption(arguments[1], "reload") == Boolean.TRUE;
      if (!loadingStack.isEmpty()) {
        moduleDependencies
            .computeIfAbsent(loadingStack.peekLast(), ignored -> ConcurrentHashMap.newKeySet())
            .add(key);
      }
      if (reload || !modules.containsKey(key)) {
        if (!loadingModules.add(key)) {
          throw new HaraException("Cyclic module require: " + key);
        }
        Map<String, HaraMacro> callerMacrosBefore =
            new LinkedHashMap<>(macros.getOrDefault(callerNamespace, Map.of()));
        try {
          loadingStack.addLast(key);
          if (classpath) {
            loadResource(resourceName);
            registerResource(resourceName);
          } else {
            loadFile(key);
          }
        } finally {
          loadingStack.removeLastOccurrence(key);
          loadingModules.remove(key);
        }
        ModuleRecord loaded = modules.get(key);
        relocateLoadedMacros(callerNamespace, callerMacrosBefore, loaded);
      }
      currentNamespace = namespace(callerNamespace);
      if (arguments.length == 2) {
        applyRequireOptions(arguments[1], modules.get(key));
      }
      return null;
    } finally {
      currentNamespace = namespace(callerNamespace);
    }
  }

  @TruffleBoundary
  private Object requireNamespace(Symbol symbol, Object options) {
    if (symbol.getNamespace() != null) {
      throw new HaraException("require expects an unqualified namespace symbol");
    }
    String target = symbol.display();
    boolean reload = options != null && Boolean.TRUE.equals(requireOption(options, "reload"));
    HaraNamespace required;
    if (reload) {
      NamespaceLoadState previousState = namespaceStates.get(target);
      ContextSnapshot snapshot = snapshot();
      namespaceStates.put(target, NamespaceLoadState.LOADING);
      try {
        libraryLoader.ensure(this, target);
        required =
            libraryLoader.provides(target)
                ? loadLibraryResource(target, true)
                : requireSourceNamespace(target, true);
        if (required == null) required = namespaces.get(target);
        if (required == null) {
          restore(snapshot);
          if (previousState != NamespaceLoadState.LOADED) {
            namespaceStates.put(target, NamespaceLoadState.FAILED);
            namespaceFailures.put(
                target, "no library, source, or extension provided this namespace");
          }
        } else {
          namespaceStates.put(target, NamespaceLoadState.LOADED);
          namespaceFailures.remove(target);
        }
      } catch (RuntimeException failure) {
        restore(snapshot);
        if (previousState != NamespaceLoadState.LOADED) {
          namespaceStates.put(target, NamespaceLoadState.FAILED);
          namespaceFailures.put(
              target,
              failure.getMessage() == null
                  ? failure.getClass().getSimpleName()
                  : failure.getMessage());
        }
        throw failure;
      }
    } else {
      required = requiredNamespace(target);
    }
    if (required == null) {
      throw new HaraException("Cannot require missing namespace: " + target);
    }
    if (options != null) applyNamespaceOptions(target, required, options);
    return null;
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  private void applyNamespaceOptions(String target, HaraNamespace required, Object options) {
    if (!(options instanceof IMapType<?, ?> map)) throw new HaraException("require options expect a map");
    Object alias = unwrapQuoted(((IMapType) map).lookup(Keyword.create("as")));
    if (alias != null) {
      if (!(alias instanceof Symbol) || ((Symbol) alias).getNamespace() != null) {
        throw new HaraException("require :as expects an unqualified symbol");
      }
      defineAlias((Symbol) alias, Symbol.create(target));
    }
    Object refer = unwrapQuoted(((IMapType) map).lookup(Keyword.create("refer")));
    if (refer != null) {
      java.util.List<Object> symbols = new ArrayList<>();
      if (refer instanceof Keyword keyword && "all".equals(keyword.getName())) {
        symbols.addAll(required.symbolNames());
      } else if (refer instanceof ILinearType<?>) {
        for (Object value : (ILinearType<?>) refer) symbols.add(value);
      } else {
        throw new HaraException("require :refer expects a sequential collection of symbols or :all");
      }
      for (Object value : symbols) {
        if (value instanceof String) value = Symbol.create((String) value);
        if (!(value instanceof Symbol) || ((Symbol) value).getNamespace() != null) {
          throw new HaraException("require :refer expects unqualified symbols");
        }
        String name = ((Symbol) value).getName();
        HaraVar variable = required.lookup(name);
        if (variable == null) throw new HaraException("Cannot refer missing var " + name + " from " + target);
        currentNamespace.refer(name, variable);
        HaraMacro macro = macros.getOrDefault(target, Map.of()).get(name);
        if (macro != null) macros.computeIfAbsent(currentNamespace.name(), ignored -> new ConcurrentHashMap<>()).put(name, macro);
      }
    }
  }

  private void applyRequireOptions(Object options, ModuleRecord module) {
    if (!(options instanceof IMapType<?, ?>) || module == null) {
      throw new HaraException("require options expect a map");
    }
    @SuppressWarnings("rawtypes")
    Object alias = unwrapQuoted(((IMapType) options).lookup(Keyword.create("as")));
    if (alias != null && (!(alias instanceof Symbol) || ((Symbol) alias).getNamespace() != null)) {
      throw new HaraException("require :as expects an unqualified symbol");
    }
    if (alias != null) defineAlias((Symbol) alias, Symbol.create(module.namespace));

    @SuppressWarnings("rawtypes")
    Object refer = unwrapQuoted(((IMapType) options).lookup(Keyword.create("refer")));
    if (refer != null) {
      if (!(refer instanceof ILinearType<?>)) {
        throw new HaraException("require :refer expects a sequential collection of symbols");
      }
      HaraNamespace target = namespaces.get(module.namespace);
      for (Object value : (ILinearType<?>) refer) {
        if (!(value instanceof Symbol) || ((Symbol) value).getNamespace() != null) {
          throw new HaraException("require :refer expects unqualified symbols");
        }
        Symbol symbol = (Symbol) value;
        HaraVar variable = target == null ? null : target.lookup(symbol.getName());
        if (variable == null) {
          throw new HaraException(
              "Cannot refer missing var " + symbol.getName() + " from " + module.namespace);
        }
        currentNamespace.refer(symbol.getName(), variable);
        Map<String, HaraMacro> targetMacros = macros.get(module.namespace);
        if (targetMacros != null && targetMacros.containsKey(symbol.getName())) {
          macros
              .computeIfAbsent(currentNamespace.name(), ignored -> new ConcurrentHashMap<>())
              .put(symbol.getName(), targetMacros.get(symbol.getName()));
        }
      }
    }

    Object referMacros = unwrapQuoted(((IMapType) options).lookup(Keyword.create("refer-macros")));
    if (referMacros == null) return;
    if (!(referMacros instanceof ILinearType<?>)) {
      throw new HaraException("require :refer-macros expects a sequential collection of symbols");
    }
    Map<String, HaraMacro> targetMacros = macros.get(module.namespace);
    for (Object value : (ILinearType<?>) referMacros) {
      if (!(value instanceof Symbol) || ((Symbol) value).getNamespace() != null) {
        throw new HaraException("require :refer-macros expects unqualified symbols");
      }
      String name = ((Symbol) value).getName();
      HaraMacro macro = targetMacros == null ? null : targetMacros.get(name);
      if (macro == null) {
        throw new HaraException("Cannot refer missing macro " + name + " from " + module.namespace);
      }
      macros
          .computeIfAbsent(currentNamespace.name(), ignored -> new ConcurrentHashMap<>())
          .put(name, macro);
    }
  }

  private Object unwrapQuoted(Object value) {
    if (value instanceof List<?>
        && ((List<?>) value).count() == 2
        && Symbol.create("quote").equals(((List<?>) value).nth(0))) {
      return ((List<?>) value).nth(1);
    }
    return value;
  }

  @TruffleBoundary
  private void relocateLoadedMacros(
      String callerNamespace, Map<String, HaraMacro> callerMacrosBefore, ModuleRecord module) {
    if (module == null || callerNamespace.equals(module.namespace)) return;
    Map<String, HaraMacro> callerMacros = macros.get(callerNamespace);
    if (callerMacros == null) return;
    Map<String, HaraMacro> moduleMacros =
        macros.computeIfAbsent(module.namespace, ignored -> new ConcurrentHashMap<>());
    for (Map.Entry<String, HaraMacro> entry : new LinkedHashMap<>(callerMacros).entrySet()) {
      HaraMacro previous = callerMacrosBefore.get(entry.getKey());
      if (previous != entry.getValue()) {
        moduleMacros.put(entry.getKey(), entry.getValue());
        callerMacros.remove(entry.getKey(), entry.getValue());
      }
    }
  }

  @SuppressWarnings("rawtypes")
  private Object requireOption(Object options, String name) {
    if (!(options instanceof IMapType)) {
      throw new HaraException("require options expect a map");
    }
    return ((IMapType) options).lookup(Keyword.create(name));
  }

  private Object moduleRevision(Object value) {
    if (!(value instanceof String)) {
      throw new HaraException("module-revision expects a path string");
    }
    String requested = (String) value;
    String key =
        requested.startsWith("classpath:")
            ? requested
            : (getResource(requested) == null
                ? canonicalPath(requested)
                : "classpath:" + requested);
    ModuleRecord module = modules.get(key);
    return module == null ? 0L : module.revision;
  }

  @TruffleBoundary
  private Object moduleDependencies(Object value) {
    if (!(value instanceof String)) {
      throw new HaraException("module-dependencies expects a path string");
    }
    String requested = (String) value;
    String key =
        requested.startsWith("classpath:")
            ? requested
            : (getResource(requested) == null
                ? canonicalPath(requested)
                : "classpath:" + requested);
    Set<String> dependencies = moduleDependencies.getOrDefault(key, Set.of());
    return BuiltinStruct.vector(new LinkedHashSet<>(dependencies).toArray());
  }

  @TruffleBoundary
  private Object referNamespace(Object value) {
    if (!(value instanceof String)) {
      throw new HaraException("refer expects a namespace string");
    }
    HaraNamespace target = namespaces.get((String) value);
    if (target == null) {
      throw new HaraException("Cannot refer missing namespace: " + value);
    }
    for (Map.Entry<String, HaraVar> entry : target.vars.entrySet()) {
      currentNamespace.refer(entry.getKey(), entry.getValue());
    }
    Map<String, HaraMacro> targetMacros = macros.get(target.name());
    if (targetMacros != null) {
      macros
          .computeIfAbsent(currentNamespace.name(), ignored -> new ConcurrentHashMap<>())
          .putAll(targetMacros);
    }
    return null;
  }

  private Object inNamespace(Object value) {
    if (!(value instanceof Symbol) || ((Symbol) value).getNamespace() != null) {
      throw new HaraException("in-ns expects an unqualified namespace symbol");
    }
    setCurrentNamespace((Symbol) value);
    return value;
  }

  private Object useNamespace(Object value) {
    if (value instanceof Symbol && ((Symbol) value).getNamespace() == null) {
      return referNamespace(((Symbol) value).getName());
    }
    return referNamespace(value);
  }

  @TruffleBoundary
  Object iterValue(Object value) {
    Object target = HaraBox.unwrap(value);
    if (target == null || target == HaraNull.SINGLETON) return Iter.emptyIterator();
    if (target instanceof Iterator<?>) return target;
    if (target instanceof String) return Iter.codePoints((String) target);
    try {
      Iterator<?> source = Iter.iter(target);
      return new CloseableIterator<Object>() {
        private boolean closed;

        @Override
        public boolean hasNext() {
          return !closed && source.hasNext();
        }

        @Override
        public Object next() {
          if (closed) throw new java.util.NoSuchElementException();
          return source.next();
        }

        @Override
        public void close() {
          if (closed) return;
          closed = true;
          Iter.close(source);
        }
      };
    } catch (RuntimeException error) {
      throw new HaraException("iter does not support value: " + target);
    }
  }

  @TruffleBoundary
  Object seqValue(Object[] values) {
    if (values.length != 1 && values.length != 2) {
      throw new HaraException("seq expects a source, or a transform and source");
    }
    Object source = values.length == 1 ? values[0] : values[1];
    Object unwrappedSource = HaraBox.unwrap(source);
    Object lazySource =
        unwrappedSource instanceof hara.lang.data.Seq
            ? unwrappedSource
            : hara.lang.data.Seq.create((Iterator<?>) snapshotOrIterator(source));
    if (values.length == 1) {
      return lazySource;
    }
    Object result = invokeCallable(values[0], new Object[] {lazySource});
    Object unwrapped = HaraBox.unwrap(result);
    hara.lang.data.Seq<?> sequence =
        unwrapped instanceof hara.lang.data.Seq
            ? (hara.lang.data.Seq<?>) unwrapped
            : hara.lang.data.Seq.create((Iterator<?>) iterValue(unwrapped));
    return sequence;
  }

  @TruffleBoundary
  private Object isIteratorFinite(Object value) {
    return !(HaraBox.unwrap(value) instanceof Iterator<?>);
  }

  @TruffleBoundary
  private Object iterMaterialize(Object value) {
    Object target = HaraBox.unwrap(value);
    if (target instanceof Iterator<?> && !((Boolean) isIteratorFinite(target))) {
      throw new HaraException("cannot materialize an infinite or unknown iterator");
    }
    Iterator<?> iterator = (Iterator<?>) iterValue(target);
    java.util.List<Object> output = new ArrayList<>();
    iterator.forEachRemaining(output::add);
    return hara.lang.data.Vector.Standard.from(null, output.toArray());
  }

  private Object snapshotOrIterator(Object value) {
    Object target = HaraBox.unwrap(value);
    if (target instanceof Iterator<?>) return target;
    if (target instanceof HaraArray) return Iter.objects(((HaraArray) target).toArray());
    return iterValue(target);
  }

  @TruffleBoundary
  private Object iterHasNext(Object value) {
    Iterator<?> iterator = requireIterator(value, "iter-next?");
    return iterator.hasNext();
  }

  @TruffleBoundary
  private Object iterNext(Object value) {
    Iterator<?> iterator = requireIterator(value, "iter-next");
    if (!iterator.hasNext()) throw new HaraException("iter-next reached the end of the iterator");
    return iterator.next();
  }

  private Object iterClose(Object value) {
    Iterator<?> iterator = requireIterator(value, "iter-close");
    Iter.close(iterator);
    return null;
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  private Object concatIterators(Object[] values) {
    return Iter.concat(Iter.map((Iterator) Iter.objects(values), value -> Iter.iter(value)));
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  private Object iterMap(Object[] values) {
    if (values.length < 2) {
      throw new HaraException("iter-map expects a function and at least one source");
    }
    Object function = values[0];
    Object[] sourceValues = java.util.Arrays.copyOfRange(values, 1, values.length);
    Iterator zipped = iterZipArrays(sourceValues);
    return closeable(
        Iter.map(
            zipped,
            value -> HaraBox.unwrap(invokeCallable(function, (Object[]) value))),
        zipped);
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  Object iterFilter(Object[] values) {
    requireIteratorArity(values, 2, "iter-filter");
    Iterator source = (Iterator) iterValue(values[1]);
    Object function = values[0];
    return closeable(
        Iter.filter(source, value -> truthy(invokeCallable(function, new Object[] {value}))),
        source);
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  private Object iterTakeWhile(Object[] values) {
    requireIteratorArity(values, 2, "iter-take-while");
    Iterator source = (Iterator) iterValue(values[1]);
    Object function = values[0];
    return closeable(
        new CloseableIterator<Object>() {
          private boolean finished;
          private boolean ready;
          private Object next;

          private void prime() {
            if (finished || ready) return;
            if (!source.hasNext()) {
              finished = true;
              Iter.close(source);
              return;
            }
            Object candidate = source.next();
            if (!truthy(invokeCallable(function, new Object[] {candidate}))) {
              finished = true;
              Iter.close(source);
              return;
            }
            next = candidate;
            ready = true;
          }

          @Override
          public boolean hasNext() {
            prime();
            return ready;
          }

          @Override
          public Object next() {
            prime();
            if (!ready) throw new NoSuchElementException();
            Object result = next;
            next = null;
            ready = false;
            return result;
          }

          @Override
          public void close() {
            finished = true;
            ready = false;
            next = null;
            Iter.close(source);
          }
        },
        source);
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  private Object iterDropWhile(Object[] values) {
    requireIteratorArity(values, 2, "iter-drop-while");
    Iterator source = (Iterator) iterValue(values[1]);
    Object function = values[0];
    return closeable(
        new CloseableIterator<Object>() {
          private boolean dropped;
          private boolean finished;
          private boolean ready;
          private Object next;

          private void prime() {
            if (finished || ready) return;
            while (!dropped && source.hasNext()) {
              Object candidate = source.next();
              if (!truthy(invokeCallable(function, new Object[] {candidate}))) {
                next = candidate;
                ready = true;
                dropped = true;
              }
            }
            if (!dropped) {
              finished = true;
              Iter.close(source);
              return;
            }
            if (!ready && source.hasNext()) {
              next = source.next();
              ready = true;
            } else if (!ready) {
              finished = true;
              Iter.close(source);
            }
          }

          @Override
          public boolean hasNext() {
            prime();
            return ready;
          }

          @Override
          public Object next() {
            prime();
            if (!ready) throw new NoSuchElementException();
            Object result = next;
            next = null;
            ready = false;
            return result;
          }

          @Override
          public void close() {
            finished = true;
            ready = false;
            next = null;
            Iter.close(source);
          }
        },
        source);
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  private Object iterMapcat(Object[] values) {
    requireIteratorArity(values, 2, "iter-mapcat");
    Iterator source = (Iterator) iterValue(values[1]);
    Object function = values[0];
    Iterator result =
        Iter.mapcat(
            source, value -> (Iterator) iterValue(invokeCallable(function, new Object[] {value})));
    return closeable(result, source);
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  private Object iterKeep(Object[] values) {
    requireIteratorArity(values, 2, "iter-keep");
    Iterator source = (Iterator) iterValue(values[1]);
    Object function = values[0];
    return closeable(
        Iter.keep(
            source,
            value -> {
              Object result = HaraBox.unwrap(invokeCallable(function, new Object[] {value}));
              return result == HaraNull.SINGLETON ? null : result;
            }),
        source);
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  private Object iterInterpose(Object[] values) {
    requireIteratorArity(values, 2, "iter-interpose");
    Object separator = values[0];
    Iterator source = (Iterator) iterValue(values[1]);
    return closeable(
        new CloseableIterator<Object>() {
          private boolean first = true;
          private boolean ready;
          private boolean emitSeparator;
          private boolean done;
          private Object next;

          private void prime() {
            if (done || ready) return;
            if (emitSeparator) {
              if (!source.hasNext()) {
                done = true;
                Iter.close(source);
                return;
              }
              next = separator;
              emitSeparator = false;
              ready = true;
              return;
            }
            if (!source.hasNext()) {
              done = true;
              Iter.close(source);
              return;
            }
            next = source.next();
            emitSeparator = source.hasNext();
            first = false;
            ready = true;
          }

          @Override
          public boolean hasNext() {
            prime();
            return ready;
          }

          @Override
          public Object next() {
            prime();
            if (!ready) throw new NoSuchElementException();
            Object result = next;
            next = null;
            ready = false;
            return result;
          }

          @Override
          public void close() {
            done = true;
            ready = false;
            next = null;
            Iter.close(source);
          }
        },
        source);
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  private Object iterInterleave(Object[] values) {
    if (values.length == 0) {
      throw new HaraException("iter-interleave expects at least one source");
    }
    Iterator[] sources = new Iterator[values.length];
    for (int i = 0; i < values.length; i++) sources[i] = (Iterator) iterValue(values[i]);
    return new CloseableIterator<Object>() {
      private int index;
      private boolean closed;

      @Override
      public boolean hasNext() {
        if (closed) return false;
        if (index == 0) {
          for (Iterator source : sources) {
            if (!source.hasNext()) {
              close();
              return false;
            }
          }
        }
        if (sources[index].hasNext()) return true;
        close();
        return false;
      }

      @Override
      public Object next() {
        if (!hasNext()) throw new NoSuchElementException();
        Object result = sources[index].next();
        index = (index + 1) % sources.length;
        return result;
      }

      @Override
      public void close() {
        if (closed) return;
        closed = true;
        for (Iterator source : sources) Iter.close(source);
      }
    };
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  private Object iterEvery(Object[] values) {
    requireIteratorArity(values, 2, "iter-every?");
    Iterator source = (Iterator) iterValue(values[1]);
    Object function = values[0];
    return Iter.every(source, value -> truthy(invokeCallable(function, new Object[] {value})));
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  private Object iterAny(Object[] values) {
    requireIteratorArity(values, 2, "iter-any?");
    Iterator source = (Iterator) iterValue(values[1]);
    Object function = values[0];
    return Iter.any(source, value -> truthy(invokeCallable(function, new Object[] {value})));
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  Object iterTake(Object[] values) {
    requireIteratorArity(values, 2, "iter-take");
    Iterator source = (Iterator) iterValue(values[1]);
    return closeable(Iter.take(source, iterationCount(values[0], "iter-take")), source);
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  Object iterDrop(Object[] values) {
    requireIteratorArity(values, 2, "iter-drop");
    Iterator source = (Iterator) iterValue(values[1]);
    return closeable(Iter.drop(source, iterationCount(values[0], "iter-drop")), source);
  }

  private Object iterZip(Object[] values) {
    CloseableIterator<Object[]> zipped = iterZipArrays(values);
    return closeable(Iter.map(zipped, BuiltinStruct::vector), zipped);
  }

  private CloseableIterator<Object[]> iterZipArrays(Object[] values) {
    if (values.length == 0) {
      throw new HaraException("iter-zip expects at least one source");
    }
    return new CloseableIterator<Object[]>() {
      private Iterator<?>[] sources;
      private boolean closed;

      private void initialize() {
        if (sources != null) return;
        sources = new Iterator<?>[values.length];
        for (int i = 0; i < values.length; i++) {
          sources[i] = (Iterator<?>) iterValue(values[i]);
        }
      }

      @Override
      public boolean hasNext() {
        if (closed) return false;
        initialize();
        for (Iterator<?> source : sources) {
          if (!source.hasNext()) return false;
        }
        return true;
      }

      @Override
      public Object[] next() {
        if (!hasNext()) throw new NoSuchElementException();
        Object[] result = new Object[sources.length];
        for (int i = 0; i < sources.length; i++) {
          result[i] = sources[i].next();
        }
        return result;
      }

      @Override
      public void close() {
        if (closed) return;
        closed = true;
        if (sources != null) {
          for (Iterator<?> source : sources) Iter.close(source);
        }
      }
    };
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  Object iterCycle(Object value) {
    Iterator cycle = Iter.cycle(() -> (Iterator) iterValue(value));
    if (!cycle.hasNext()) {
      Iter.close(cycle);
      throw new HaraException("cycle expects a non-empty source");
    }
    return cycle;
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  private Object iterPartitionPair(Object value) {
    Iterator<Object> source = (Iterator<Object>) iterValue(value);
    Iterator<Map.Entry<Object, Object>> pairs = Iter.partitionPair(source);
    return Iter.map(
        pairs, pair -> BuiltinStruct.vector(new Object[] {pair.getKey(), pair.getValue()}));
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  private Object iterPartition(Object[] values, boolean includePartial) {
    requireIteratorArity(values, 2, includePartial ? "iter-partition-all" : "iter-partition");
    int size = iterationCount(values[0], includePartial ? "iter-partition-all" : "iter-partition");
    if (size == 0) {
      throw new HaraException(
          (includePartial ? "iter-partition-all" : "iter-partition") + " expects a positive count");
    }
    Iterator source = (Iterator) iterValue(values[1]);
    return closeable(
        new CloseableIterator<Object>() {
          private boolean done;
          private boolean ready;
          private Object next;

          private void prime() {
            if (done || ready) return;
            if (!source.hasNext()) {
              done = true;
              Iter.close(source);
              return;
            }
            ArrayList<Object> chunk = new ArrayList<>(size);
            while (chunk.size() < size && source.hasNext()) {
              chunk.add(source.next());
            }
            if (!includePartial && chunk.size() < size) {
              done = true;
              Iter.close(source);
              return;
            }
            next = BuiltinStruct.vector(chunk.toArray());
            ready = true;
          }

          @Override
          public boolean hasNext() {
            prime();
            return ready;
          }

          @Override
          public Object next() {
            prime();
            if (!ready) throw new NoSuchElementException();
            Object result = next;
            next = null;
            ready = false;
            return result;
          }

          @Override
          public void close() {
            done = true;
            ready = false;
            next = null;
            Iter.close(source);
          }
        },
        source);
  }

  @TruffleBoundary
  private Object iterRange(Object[] values) {
    if (values.length < 1 || values.length > 2) {
      throw new HaraException("iter-range expects an end or start and end");
    }
    long start = values.length == 1
        ? 0L
        : HaraNumericConversions.toLong(values[0], "iter-range");
    long end = HaraNumericConversions.toLong(values[values.length - 1], "iter-range");
    return Iter.range(start, end);
  }

  private Object iterRepeatedly(Object function) {
    return Iter.repeatedly(() -> invokeCallable(function, new Object[0]));
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  private Object iterIterate(Object[] values) {
    requireIteratorArity(values, 2, "iter-iterate");
    Object function = values[0];
    return Iter.iterate(values[1], value -> invokeCallable(function, new Object[] {value}));
  }

  private static void requireIteratorArity(Object[] values, int expected, String name) {
    if (values.length != expected) {
      throw new HaraException(name + " expects " + (expected - 1) + " arguments");
    }
  }

  @TruffleBoundary
  private static int iterationCount(Object value, String name) {
    long count = HaraNumericConversions.toLong(value, name);
    if (count < 0 || count > Integer.MAX_VALUE) {
      throw new HaraException(name + " count is out of bounds: " + count);
    }
    return (int) count;
  }

  private static boolean truthy(Object value) {
    return value != null && value != HaraNull.SINGLETON && !Boolean.FALSE.equals(value);
  }

  @SuppressWarnings("unchecked")
  private static Iterator<?> closeable(Iterator<?> iterator, Iterator<?>... sources) {
    return new CloseableIterator<Object>() {
      private boolean closed;

      @Override
      public boolean hasNext() {
        return !closed && iterator.hasNext();
      }

      @Override
      public Object next() {
        if (!hasNext()) throw new NoSuchElementException();
        return iterator.next();
      }

      @Override
      public void close() {
        if (closed) return;
        closed = true;
        for (Iterator<?> source : sources) Iter.close(source);
        Iter.close(iterator);
      }
    };
  }

  private Object alterVarRoot(Object[] values) {
    if (values.length < 2 || !(HaraBox.unwrap(values[0]) instanceof HaraVar)) {
      throw new HaraException("alter-var-root expects a Var, function, and optional arguments");
    }
    HaraVar var = (HaraVar) HaraBox.unwrap(values[0]);
    Object function = HaraBox.unwrap(values[1]);
    Object[] arguments = new Object[values.length - 1];
    arguments[0] = var.deref();
    System.arraycopy(values, 2, arguments, 1, values.length - 2);
    Object updated = invokeCallable(function, arguments);
    return var.reset(HaraBox.unwrap(updated));
  }

  private Object applyFunction(Object[] values) {
    if (values.length < 2) {
      throw new HaraException("apply expects a function and a final sequential value");
    }
    ArrayList<Object> arguments = new ArrayList<>();
    for (int i = 1; i < values.length - 1; i++) {
      arguments.add(values[i]);
    }
    Iterator<?> tail = (Iterator<?>) iterValue(values[values.length - 1]);
    while (tail.hasNext()) {
      arguments.add(tail.next());
    }
    return invokeCallable(values[0], arguments.toArray());
  }

  @TruffleBoundary
  public Object invokeCallable(Object value, Object[] arguments) {
    return invokeCallable(value, arguments, null);
  }

  @TruffleBoundary
  public Object invokeCallable(
      Object value, Object[] arguments, hara.lang.base.Ex.Info.Site creationSite) {
    Object function = HaraBox.unwrap(value);
    if (function instanceof HaraVar variable) {
      Object dereferenced = variable.deref();
      if (dereferenced == variable) {
        throw new HaraException("Var refers to itself: " + variable);
      }
      return invokeCallable(dereferenced, arguments, creationSite);
    }
    if (function instanceof HaraBuiltinFunction builtin) {
      Object result = builtin.apply(arguments);
      if (creationSite != null
          && builtin.recordsExceptionCreation()
          && result instanceof hara.lang.base.Ex.Info info) {
        info.recordCreation(creationSite);
      }
      return HaraBox.unwrap(result);
    }
    if (function instanceof HaraFunction) {
      HaraFunction haraFunction = (HaraFunction) function;
      HaraFunction selected = haraFunction.resolveArity(arguments.length);
      if (selected == null) {
        throw new HaraException("function has no matching arity: " + arguments.length);
      }
      return HaraBox.unwrap(selected.callTarget().call(selected.callArguments(arguments)));
    }
    if (function instanceof HbcMachine.HbcClosure) {
      return HaraBox.unwrap(((HbcMachine.HbcClosure) function).invoke(arguments));
    }
    if (function instanceof HbcMachine.HbcMultiArity) {
      return HaraBox.unwrap(((HbcMachine.HbcMultiArity) function).invoke(arguments));
    }
    if (function instanceof HbcMachine.HbcNativeCallable) {
      return HaraBox.unwrap(((HbcMachine.HbcNativeCallable) function).invoke(arguments));
    }
    if (function instanceof hara.lang.data.Pointer pointer) {
      return pointerContextCall(pointer, pointerDefault(pointer), "pointer/invoke", arguments);
    }
    if (function instanceof HaraMultiFunction) {
      return HaraBox.unwrap(((HaraMultiFunction) function).invoke(arguments));
    }
    if (function instanceof HaraStruct || function instanceof HaraMutable || function instanceof IFn) {
      return HaraBox.unwrap(ifnProtocol.invoke("invoke", function, arguments));
    }
    if (function instanceof HaraType) {
      HaraType type = (HaraType) function;
      if (arguments.length != type.arity()) {
        throw new HaraException("constructor has no matching arity: " + arguments.length);
      }
      try {
        return type.construct(arguments);
      } catch (com.oracle.truffle.api.interop.ArityException impossible) {
        throw new IllegalStateException("constructor arity was checked", impossible);
      }
    }
    throw new HaraException("value is not callable: " + function);
  }

  Object hbcAsync(java.util.function.Supplier<Object> operation) {
    CompletableFuture<Object> future =
        CompletableFuture.supplyAsync(
                () -> {
                  try {
                    return invokeInContext(operation);
                  } catch (RuntimeException failure) {
                    throw new CompletionException(failure);
                  }
                })
            .thenCompose(this::flatten);
    return new HaraPromise(future);
  }

  public boolean isFunctionValue(Object value) {
    Object function = HaraBox.unwrap(value);
    if (function instanceof HaraVar variable) {
      Object dereferenced = variable.deref();
      return dereferenced != variable && isFunctionValue(dereferenced);
    }
    return function instanceof HaraFunction
        || function instanceof HaraMultiFunction
        || function instanceof HaraType
        || function instanceof hara.lang.data.Pointer
        || function instanceof HaraStruct
        || function instanceof HaraMutable
        || function instanceof IFn
        || function instanceof HbcMachine.HbcClosure
        || function instanceof HbcMachine.HbcMultiArity
        || function instanceof HbcMachine.HbcNativeCallable
        || function instanceof UnaryBuiltin
        || function instanceof VariadicBuiltin;
  }

  private boolean isNativeFunctionValue(Object value) {
    Object function = HaraBox.unwrap(value);
    return function instanceof HaraFunction
        || function instanceof HaraMultiFunction
        || function instanceof ObjFn
        || function instanceof HaraBuiltinFunction
        || function instanceof HbcMachine.HbcClosure
        || function instanceof HbcMachine.HbcMultiArity
        || function instanceof HbcMachine.HbcNativeCallable;
  }

  private boolean protocolSatisfies(HaraProtocol protocol, Object value) {
    Object receiver = HaraBox.unwrap(value);
    Boolean policy = protocolSatisfactionPolicy(protocol.name(), receiver);
    if (policy != null) return policy;
    return protocol.satisfies(receiver);
  }

  /**
   * Keeps the two compatibility rules that are not expressible as Java-interface dispatch:
   * callable guest values include runtime wrappers that cannot implement IFn, while sets expose
   * get-like membership without being key/value lookupables.
   */
  private Boolean protocolSatisfactionPolicy(String protocolName, Object receiver) {
    String simpleName = protocolName.substring(protocolName.lastIndexOf('.') + 1);
    if ("IFn".equals(simpleName)) return isFunctionValue(receiver);
    if ("ISequential".equals(simpleName)) {
      return receiver instanceof hara.lang.protocol.ISequential<?>;
    }
    if ("ILookup".equals(simpleName)
        && receiver instanceof hara.lang.protocol.ISetType<?>) {
      return false;
    }
    return null;
  }

  private UnaryBuiltin typePredicate(String name, Class<?> type) {
    return new UnaryBuiltin(name, value -> type.isInstance(HaraBox.unwrap(value)));
  }

  private Iterator<?> requireIterator(Object value, String operation) {
    Object target = HaraBox.unwrap(value);
    if (!(target instanceof Iterator<?>)) {
      throw new HaraException(operation + " expects an iterator");
    }
    return (Iterator<?>) target;
  }

  private String canonicalPath(String value) {
    return environment.getPublicTruffleFile(value).getAbsoluteFile().normalize().getPath();
  }

  private void registerModule(String path) {
    String key = path;
    ModuleRecord previous = modules.get(key);
    String namespaceName = currentNamespace.name();
    modules.put(
        key, new ModuleRecord(key, namespaceName, previous == null ? 1L : previous.revision + 1L));
    moduleDependencies.computeIfAbsent(key, ignored -> ConcurrentHashMap.newKeySet());
  }

  private void registerResource(String resourceName) {
    String key = "classpath:" + resourceName;
    ModuleRecord previous = modules.get(key);
    String namespaceName = currentNamespace.name();
    modules.put(
        key, new ModuleRecord(key, namespaceName, previous == null ? 1L : previous.revision + 1L));
    moduleDependencies.computeIfAbsent(key, ignored -> ConcurrentHashMap.newKeySet());
  }

  private java.net.URL getResource(String resourceName) {
    ClassLoader definingLoader = HaraContext.class.getClassLoader();
    java.net.URL resource =
        definingLoader == null
            ? ClassLoader.getSystemResource(resourceName)
            : definingLoader.getResource(resourceName);
    if (resource != null) {
      return resource;
    }
    ClassLoader contextLoader = Thread.currentThread().getContextClassLoader();
    return contextLoader != null && contextLoader != definingLoader
        ? contextLoader.getResource(resourceName)
        : null;
  }

  @TruffleBoundary
  private ContextSnapshot snapshot() {
    Map<String, Map<String, Object>> values = new LinkedHashMap<>();
    Map<String, Map<String, HaraVar>> bindings = new LinkedHashMap<>();
    Map<String, Map<String, IMetadata>> metadata = new LinkedHashMap<>();
    Map<String, Map<String, HaraVar.Origin>> origins = new LinkedHashMap<>();
    Map<String, String> roles = new LinkedHashMap<>();
    for (Map.Entry<String, HaraNamespace> namespace : namespaces.entrySet()) {
      Map<String, Object> namespaceValues = new LinkedHashMap<>();
      Map<String, HaraVar> namespaceBindings = new LinkedHashMap<>();
      Map<String, IMetadata> namespaceMetadata = new LinkedHashMap<>();
      Map<String, HaraVar.Origin> namespaceOrigins = new LinkedHashMap<>();
      for (Map.Entry<String, HaraVar> var : namespace.getValue().vars.entrySet()) {
        namespaceValues.put(var.getKey(), var.getValue().get());
        namespaceBindings.put(var.getKey(), var.getValue());
        namespaceMetadata.put(var.getKey(), var.getValue().meta());
        namespaceOrigins.put(var.getKey(), var.getValue().origin());
      }
      values.put(namespace.getKey(), namespaceValues);
      bindings.put(namespace.getKey(), namespaceBindings);
      metadata.put(namespace.getKey(), namespaceMetadata);
      origins.put(namespace.getKey(), namespaceOrigins);
      roles.put(namespace.getKey(), namespace.getValue().role);
    }
    Map<String, Map<String, HaraMacro>> macroValues = new LinkedHashMap<>();
    for (Map.Entry<String, Map<String, HaraMacro>> entry : macros.entrySet()) {
      macroValues.put(entry.getKey(), new LinkedHashMap<>(entry.getValue()));
    }
    Map<String, Map<String, String>> aliasValues = new LinkedHashMap<>();
    for (Map.Entry<String, Map<String, String>> entry : aliases.entrySet()) {
      aliasValues.put(entry.getKey(), new LinkedHashMap<>(entry.getValue()));
    }
    Map<String, String> globalAliasValues = new LinkedHashMap<>(globalAliases);
    Map<String, String> globalImportValues = new LinkedHashMap<>(globalImports);
    Map<String, String> nativeFlavorValues = new LinkedHashMap<>(nativeFlavors);
    Map<String, Map<String, Object>> nativeImportValues = new LinkedHashMap<>();
    for (Map.Entry<String, Map<String, Object>> entry : nativeImports.entrySet()) {
      nativeImportValues.put(entry.getKey(), new LinkedHashMap<>(entry.getValue()));
    }
    Map<String, Set<String>> dependencyValues = new LinkedHashMap<>();
    for (Map.Entry<String, Set<String>> entry : moduleDependencies.entrySet()) {
      dependencyValues.put(entry.getKey(), new LinkedHashSet<>(entry.getValue()));
    }
    return new ContextSnapshot(
        currentNamespace.name(),
        values,
        bindings,
        metadata,
        origins,
        roles,
        macroValues,
        aliasValues,
        globalAliasValues,
        globalImportValues,
        nativeFlavorValues,
        nativeImportValues,
        new LinkedHashMap<>(modules),
        dependencyValues,
        new LinkedHashMap<>(namespaceStates),
        new LinkedHashSet<>(blankNamespaces));
  }

  @TruffleBoundary
  private void restore(ContextSnapshot snapshot) {
    namespaces.clear();
    for (Map.Entry<String, Map<String, Object>> entry : snapshot.values.entrySet()) {
      HaraNamespace namespace = namespace(entry.getKey());
      namespace.role = snapshot.roles.getOrDefault(entry.getKey(), "standard");
      for (Map.Entry<String, Object> value : entry.getValue().entrySet()) {
        HaraVar binding = snapshot.bindings.get(entry.getKey()).get(value.getKey());
        if (binding == null) {
          namespace.define(value.getKey(), value.getValue());
        } else {
          binding.set(value.getValue());
          binding.setMetadata(snapshot.metadata.get(entry.getKey()).get(value.getKey()));
          binding.setOrigin(snapshot.origins.get(entry.getKey()).get(value.getKey()));
          namespace.refer(value.getKey(), binding);
        }
      }
    }
    currentNamespace = namespace(snapshot.currentNamespace);
    macros.clear();
    for (Map.Entry<String, Map<String, HaraMacro>> entry : snapshot.macros.entrySet()) {
      macros.put(entry.getKey(), new ConcurrentHashMap<>(entry.getValue()));
    }
    aliases.clear();
    for (Map.Entry<String, Map<String, String>> entry : snapshot.aliases.entrySet()) {
      aliases.put(entry.getKey(), new ConcurrentHashMap<>(entry.getValue()));
    }
    globalAliases.clear();
    globalAliases.putAll(snapshot.globalAliases);
    globalImports.clear();
    globalImports.putAll(snapshot.globalImports);
    nativeFlavors.clear();
    nativeFlavors.putAll(snapshot.nativeFlavors);
    nativeImports.clear();
    for (Map.Entry<String, Map<String, Object>> entry : snapshot.nativeImports.entrySet()) {
      nativeImports.put(entry.getKey(), new ConcurrentHashMap<>(entry.getValue()));
    }
    modules.clear();
    modules.putAll(snapshot.modules);
    moduleDependencies.clear();
    for (Map.Entry<String, Set<String>> entry : snapshot.moduleDependencies.entrySet()) {
      moduleDependencies.put(entry.getKey(), ConcurrentHashMap.newKeySet());
      moduleDependencies.get(entry.getKey()).addAll(entry.getValue());
    }
    namespaceStates.clear();
    namespaceStates.putAll(snapshot.namespaceStates);
    blankNamespaces.clear();
    blankNamespaces.addAll(snapshot.blankNamespaces);
  }

  private static final class BuiltinExport {
    private final String namespace;
    private final String name;
    private final Object value;
    private final IMetadata metadata;
    private final HaraVar.Origin origin;

    private BuiltinExport(
        String namespace,
        String name,
        Object value,
        IMetadata metadata,
        HaraVar.Origin origin) {
      this.namespace = namespace;
      this.name = name;
      this.value = value;
      this.metadata = metadata == null ? hara.lang.data.Map.Standard.EMPTY : metadata;
      this.origin = origin;
    }
  }

  private static final class ContextSnapshot {
    private final String currentNamespace;
    private final Map<String, Map<String, Object>> values;
    private final Map<String, Map<String, HaraVar>> bindings;
    private final Map<String, Map<String, IMetadata>> metadata;
    private final Map<String, Map<String, HaraVar.Origin>> origins;
    private final Map<String, String> roles;
    private final Map<String, Map<String, HaraMacro>> macros;
    private final Map<String, Map<String, String>> aliases;
    private final Map<String, String> globalAliases;
    private final Map<String, String> globalImports;
    private final Map<String, String> nativeFlavors;
    private final Map<String, Map<String, Object>> nativeImports;
    private final Map<String, ModuleRecord> modules;
    private final Map<String, Set<String>> moduleDependencies;
    private final Map<String, NamespaceLoadState> namespaceStates;
    private final Set<String> blankNamespaces;

    private ContextSnapshot(
        String currentNamespace,
        Map<String, Map<String, Object>> values,
        Map<String, Map<String, HaraVar>> bindings,
        Map<String, Map<String, IMetadata>> metadata,
        Map<String, Map<String, HaraVar.Origin>> origins,
        Map<String, String> roles,
        Map<String, Map<String, HaraMacro>> macros,
        Map<String, Map<String, String>> aliases,
        Map<String, String> globalAliases,
        Map<String, String> globalImports,
        Map<String, String> nativeFlavors,
        Map<String, Map<String, Object>> nativeImports,
        Map<String, ModuleRecord> modules,
        Map<String, Set<String>> moduleDependencies,
        Map<String, NamespaceLoadState> namespaceStates,
        Set<String> blankNamespaces) {
      this.currentNamespace = currentNamespace;
      this.values = values;
      this.bindings = bindings;
      this.metadata = metadata;
      this.origins = origins;
      this.roles = roles;
      this.macros = macros;
      this.aliases = aliases;
      this.globalAliases = globalAliases;
      this.globalImports = globalImports;
      this.nativeFlavors = nativeFlavors;
      this.nativeImports = nativeImports;
      this.modules = modules;
      this.moduleDependencies = moduleDependencies;
      this.namespaceStates = namespaceStates;
      this.blankNamespaces = blankNamespaces;
    }
  }

  private static final class ModuleRecord {
    private final String path;
    private final String namespace;
    private final long revision;

    private ModuleRecord(String path, String namespace, long revision) {
      this.path = path;
      this.namespace = namespace;
      this.revision = revision;
    }
  }

  @TruffleBoundary
  private Object parseAndExecute(String sourceText, String name) {
    return evaluationRuntime.evalSource(sourceText, name);
  }

  private static final class HaraArray extends ArrayList<Object>
      implements ICount, INth<Object>, IEmpty, IConj<Object> {
    private HaraArray() {}

    private HaraArray(Object[] values) {
      super(java.util.Arrays.asList(values));
    }

    @Override
    public long count() {
      return size();
    }

    @Override
    public Object nth(long index) {
      if (index < 0 || index >= size()) {
        throw new HaraException("nth index out of bounds: " + index);
      }
      return get((int) index);
    }

    @Override
    public HaraArray empty() {
      return new HaraArray();
    }

    @Override
    public HaraArray conj(Object value) {
      add(value);
      return this;
    }
  }

  private static final class HaraObject extends LinkedHashMap<String, Object>
      implements ICount, IEmpty, IConj<Object> {
    private HaraObject() {}

    private HaraObject(Object[] values) {
      if ((values.length & 1) != 0) {
        throw new HaraException("object expects an even number of string key/value arguments");
      }
      for (int i = 0; i < values.length; i += 2) {
        put(objectKey(values[i], "constructor"), values[i + 1]);
      }
    }

    private HaraObject(HaraObject source) {
      super(source);
    }

    @Override
    public long count() {
      return size();
    }

    @Override
    public HaraObject empty() {
      return new HaraObject();
    }

    @Override
    public HaraObject conj(Object value) {
      Object entry = HaraBox.unwrap(value);
      Object key;
      Object item;
      if (entry instanceof java.util.Map.Entry<?, ?> mapEntry) {
        key = mapEntry.getKey();
        item = mapEntry.getValue();
      } else if (entry instanceof hara.lang.protocol.IPair<?, ?> pair) {
        key = pair.getKey();
        item = pair.getValue();
      } else {
        throw new HaraException("IConj/conj object expects a two-element entry");
      }
      put(objectKey(key, "IConj/conj object"), item);
      return this;
    }
  }

  private final class HaraSocket implements IDisplay {
    private final Socket socket;
    private final Object eventCallback;
    private final HaraSocketServer parent;
    private final java.util.List<HaraSocketStream> streams = new java.util.concurrent.CopyOnWriteArrayList<>();
    private final java.util.List<HaraByteStream> byteStreams = new java.util.concurrent.CopyOnWriteArrayList<>();

    private HaraSocket(Socket socket) {
      this(socket, null, null);
    }

    private HaraSocket(Socket socket, Object eventCallback) {
      this(socket, eventCallback, null);
    }

    private HaraSocket(Socket socket, Object eventCallback, HaraSocketServer parent) {
      this.socket = socket;
      this.eventCallback = eventCallback;
      this.parent = parent;
    }

    private HaraSocketStream events() {
      HaraSocketStream stream = new HaraSocketStream(this::closeQuietly);
      streams.add(stream);
      return stream;
    }

    private HaraByteStream bytes() {
      HaraByteStream stream = new HaraByteStream(HaraContext.this, this::closeQuietly);
      byteStreams.add(stream);
      return stream;
    }

    private void emit(Object event) {
      emit(event, 0);
    }

    private void emit(Object event, int byteCount) {
      for (HaraSocketStream stream : streams) stream.publish(event, byteCount);
      @SuppressWarnings("rawtypes")
      IMapType eventMap = event instanceof IMapType ? (IMapType) event : null;
      Object kind = eventMap != null
          ? eventMap.lookup(Keyword.create("type"))
          : null;
      if (Keyword.create("data").equals(kind)) {
        Object bytes = eventMap.lookup(Keyword.create("bytes"));
        for (HaraByteStream stream : byteStreams) stream.publish((byte[]) bytes);
      } else if (Keyword.create("close").equals(kind)) {
        for (HaraByteStream stream : byteStreams) stream.finish();
      } else if (Keyword.create("error").equals(kind)) {
        Object error = eventMap.lookup(Keyword.create("error"));
        for (HaraByteStream stream : byteStreams) stream.fail(new HaraException(String.valueOf(error)));
      }
      if (parent != null) parent.emit(event, byteCount);
      else if (eventCallback != null) invokeInContext(() -> invokeCallable(eventCallback, new Object[] {event}));
    }

    private void closeQuietly() {
      try { socket.close(); } catch (IOException ignored) { }
    }

    private void startDrainer() {
      Thread reader =
          new Thread(
              () -> {
                try (InputStream input = socket.getInputStream()) {
                  byte[] buffer = new byte[8192];
                  int read;
                  while ((read = input.read(buffer)) >= 0) {
                    byte[] bytes = java.util.Arrays.copyOf(buffer, read);
                    emit(socketEvent("data", this, bytes, null), bytes.length);
                  }
                  emit(socketEvent("close", this, null, null));
                } catch (IOException error) {
                  emit(socketEvent("error", this, null, error.getMessage()));
                }
              },
              "hara-socket-reader");
      reader.setDaemon(true);
      reader.start();
    }


    public String display() {
      return "#<socket " + socket.getRemoteSocketAddress() + ">";
    }
  }

  private final class HaraSocketServer implements IDisplay {
    private final ServerSocket server;
    private final Object callback;
    private final java.util.List<HaraSocketStream> streams = new java.util.concurrent.CopyOnWriteArrayList<>();

    private HaraSocketServer(ServerSocket server, Object callback) {
      this.server = server;
      this.callback = callback;
    }

    private String host() { return server.getInetAddress().getHostAddress(); }
    private int port() { return server.getLocalPort(); }
    private HaraSocketStream events() {
      HaraSocketStream stream = new HaraSocketStream(() -> {});
      streams.add(stream);
      return stream;
    }
    private void emit(Object event) {
      emit(event, 0);
    }
    private void emit(Object event, int byteCount) {
      for (HaraSocketStream stream : streams) stream.publish(event, byteCount);
      invokeInContext(() -> invokeCallable(callback, new Object[] {event}));
    }
    private void start() {
      Thread acceptor = new Thread(() -> {
        while (!server.isClosed()) {
          try {
            HaraSocket connection = new HaraSocket(server.accept(), null, this);
            emit(socketEvent("open", connection, null, null, this));
            connection.startDrainer();
          } catch (IOException error) {
            if (!server.isClosed()) emit(socketEvent("error", null, null, error.getMessage(), this));
            return;
          }
        }
      }, "hara-socket-listener");
      acceptor.setDaemon(true);
      acceptor.start();
    }
    private void close() {
      try { server.close(); } catch (IOException error) { throw new HaraException("socket/close failed: " + error.getMessage()); }
    }
    @Override public String display() { return "#<socket-server " + host() + ":" + port() + ">"; }
  }

  private final class HaraSocketStream implements IDisplay {
    private final java.util.ArrayDeque<SocketStreamEntry> events = new java.util.ArrayDeque<>();
    private final Runnable overflow;
    private CompletableFuture<Object> waiting;
    private int queuedBytes;
    private boolean closed;
    private HaraSocketStream(Runnable overflow) { this.overflow = overflow; }
    private synchronized void publish(Object event, int byteCount) {
      if (closed) return;
      if (waiting != null) { waiting.complete(event); waiting = null; return; }
      if (events.size() >= 256 || queuedBytes + byteCount > 1_048_576) {
        closed = true;
        overflow.run();
        return;
      }
      queuedBytes += byteCount;
      events.addLast(new SocketStreamEntry(event, byteCount));
    }
    private synchronized HaraPromise next() {
      if (!events.isEmpty()) {
        SocketStreamEntry entry = events.removeFirst();
        queuedBytes -= entry.byteCount();
        return new HaraPromise(CompletableFuture.completedFuture(entry.event()));
      }
      if (closed) return new HaraPromise(CompletableFuture.completedFuture(socketEvent("close", null, null, null)));
      if (waiting == null) waiting = new CompletableFuture<>();
      return new HaraPromise(waiting);
    }
    @Override public String display() { return "#<socket-stream>"; }
  }

  private record SocketStreamEntry(Object event, int byteCount) {}

  private final class HaraProcess implements IDisplay {
    private final Process process;
    private final OutputStream stdin;
    private final CompletableFuture<Object> stdout;
    private final CompletableFuture<Object> stderr;
    private final CompletableFuture<Object> exit;
    private final HaraByteStream stdoutStream;
    private final HaraByteStream stderrStream;

    private HaraProcess(Process process) {
      this.process = process;
      this.stdin = process.getOutputStream();
      this.stdoutStream = new HaraByteStream(HaraContext.this, () -> {});
      this.stderrStream = new HaraByteStream(HaraContext.this, () -> {});
      this.stdout = drainProcessBytes(process.getInputStream(), stdoutStream);
      this.stderr = drainProcessBytes(process.getErrorStream(), stderrStream);
      this.exit = process.onExit().thenApply(value -> (Object) (long) value.exitValue());
    }

    private CompletableFuture<Object> drainProcessBytes(InputStream input, HaraByteStream stream) {
      return CompletableFuture.supplyAsync(() -> {
        try (input) {
          ByteArrayOutputStream output = new ByteArrayOutputStream();
          byte[] buffer = new byte[8192];
          int read;
          while ((read = input.read(buffer)) >= 0) {
            byte[] bytes = java.util.Arrays.copyOf(buffer, read);
            output.write(bytes, 0, bytes.length);
            stream.publish(bytes);
          }
          stream.finish();
          return output.toByteArray();
        } catch (IOException error) {
          stream.fail(error);
          throw new CompletionException(error);
        }
      });
    }

    @Override public String display() { return "#<process " + process.pid() + ">"; }
  }

  private Object socketEvent(String type, HaraSocket connection, byte[] bytes, String error) {
    return socketEvent(type, connection, bytes, error, null);
  }
  private Object socketEvent(String type, HaraSocket connection, byte[] bytes, String error, HaraSocketServer server) {
    java.util.ArrayList<Object> entries = new java.util.ArrayList<>();
    entries.add(Keyword.create("type")); entries.add(Keyword.create(type));
    if (server != null) { entries.add(Keyword.create("server")); entries.add(server); }
    if (connection != null) { entries.add(Keyword.create("connection")); entries.add(connection); }
    if (bytes != null) { entries.add(Keyword.create("bytes")); entries.add(bytes); }
    if (error != null) { entries.add(Keyword.create("error")); entries.add(error); }
    return hara.lang.data.Map.Standard.from(null, entries.toArray());
  }

  Object promiseValue(CompletableFuture<Object> future) {
    return new HaraPromise(future);
  }

  Object cancellablePromise(CompletableFuture<Object> future, Runnable cancelAction) {
    return new HaraPromise(future, cancelAction, true);
  }

  Object callbackStreamPromise(Object value, Runnable settled) {
    HaraPromise promise = (HaraPromise) promiseFrom(value);
    return new HaraPromise(promise.future.whenComplete((result, error) -> settled.run()));
  }

  Object completedPromise(Object value) {
    return new HaraPromise(CompletableFuture.completedFuture(value));
  }

  Object rejectedPromise(String message) {
    CompletableFuture<Object> future = new CompletableFuture<>();
    future.completeExceptionally(new HaraException(message));
    return new HaraPromise(future);
  }

  private final class HaraPromise implements IPromise {
    private final CompletableFuture<Object> future;
    private final Runnable cancelAction;
    private final boolean providerSettlesCancellation;
    private final java.util.concurrent.atomic.AtomicBoolean cancellationRequested =
        new java.util.concurrent.atomic.AtomicBoolean();

    private HaraPromise(CompletableFuture<Object> future) {
      this(future, () -> {});
    }

    private HaraPromise(CompletableFuture<Object> future, Runnable cancelAction) {
      this(future, cancelAction, false);
    }

    private HaraPromise(
        CompletableFuture<Object> future,
        Runnable cancelAction,
        boolean providerSettlesCancellation) {
      this.future = future;
      this.cancelAction = cancelAction;
      this.providerSettlesCancellation = providerSettlesCancellation;
    }

    @Override
    public Object state() {
      if (future.isCancelled()) return Keyword.create("cancelled");
      if (!future.isDone()) return Keyword.create("pending");
      return future.isCompletedExceptionally()
          ? Keyword.create("rejected")
          : Keyword.create("fulfilled");
    }

    @Override
    public Object value() {
      if (!future.isDone()) throw new HaraException("promise is pending");
      return deref();
    }

    @Override
    public Object then(Object function) {
      return promiseThen(new Object[] {this, function}, false);
    }

    @Override
    public Object catchError(Object function) {
      return promiseThen(new Object[] {this, function}, true);
    }

    @Override
    public Object finallyDo(Object function) {
      return promiseFinally(new Object[] {this, function});
    }

    @Override
    public Object cancel() {
      if (providerSettlesCancellation) {
        if (!future.isDone() && cancellationRequested.compareAndSet(false, true)) cancelAction.run();
      } else if (future.cancel(false)) {
        cancelAction.run();
      }
      return this;
    }

    @Override
    public Object deref() {
      try {
        return HaraPersistentValues.normalize(future.join());
      } catch (CompletionException error) {
        Throwable cause = error.getCause() == null ? error : error.getCause();
        if (cause instanceof HaraException haraError) throw haraError;
        if (cause instanceof hara.lang.protocol.IExInfo && cause instanceof RuntimeException runtime) {
          throw runtime;
        }
        throw new HaraException("Promise rejected: " + cause.getMessage());
      } catch (java.util.concurrent.CancellationException error) {
        throw new HaraException("Promise cancelled");
      }
    }

    @Override
    public Object derefTimeout(long milliseconds, Object timeoutValue) {
      try {
        return HaraPersistentValues.normalize(future.get(milliseconds, TimeUnit.MILLISECONDS));
      } catch (java.util.concurrent.TimeoutException error) {
        return timeoutValue;
      } catch (InterruptedException error) {
        Thread.currentThread().interrupt();
        throw new HaraException("Promise wait interrupted");
      } catch (java.util.concurrent.ExecutionException error) {
        Throwable cause = error.getCause() == null ? error : error.getCause();
        if (cause instanceof HaraException haraError) throw haraError;
        if (cause instanceof hara.lang.protocol.IExInfo && cause instanceof RuntimeException runtime) {
          throw runtime;
        }
        throw new HaraException("Promise rejected: " + cause.getMessage());
      } catch (java.util.concurrent.CancellationException error) {
        throw new HaraException("Promise cancelled");
      }
    }

    @Override
    public String toString() {
      return future.isDone() ? "#<promise realized>" : "#<promise pending>";
    }
  }

  private static final class HaraPromiseRejection extends RuntimeException {
    private final Object value;

    private HaraPromiseRejection(Object value) {
      super(String.valueOf(value));
      this.value = value;
    }
  }

  private static final class UnaryBuiltin implements IFn<Object, Object, Object>, HaraBuiltinFunction {
    private final String name;
    private final Function<Object, Object> implementation;
    private volatile String origin;

    private UnaryBuiltin(String name, Function<Object, Object> implementation) {
      this.name = name;
      this.implementation = implementation;
      this.origin = name.contains("/") ? name : null;
    }

    @Override
    public Function<Object, Object> getArg1() {
      return implementation;
    }

    @Override
    @TruffleBoundary
    @SuppressWarnings({"rawtypes", "unchecked"})
    public Object apply(Object[] arguments) {
      return IFn.applyAsArray(this, arguments);
    }

    @Override
    public String origin() {
      return origin;
    }

    @Override
    public void setOrigin(String origin) {
      this.origin = origin;
    }

    @Override
    public String toString() {
      return "#<builtin " + name + ">";
    }
  }

  private static final class VariadicBuiltin
      implements IFn<Object, Object, Object>, HaraBuiltinFunction {
    private final String name;
    private final Function<Object[], Object> implementation;
    private final boolean recordsExceptionCreation;
    private volatile String origin;

    private VariadicBuiltin(String name, Function<Object[], Object> implementation) {
      this(name, implementation, false);
    }

    private VariadicBuiltin(
        String name, Function<Object[], Object> implementation, boolean recordsExceptionCreation) {
      this.name = name;
      this.implementation = implementation;
      this.recordsExceptionCreation = recordsExceptionCreation;
      this.origin = name.contains("/") ? name : null;
    }

    @Override
    public Supplier<Object> getArg0() {
      return () -> implementation.apply(new Object[0]);
    }

    @Override
    public Function<Object, Object> getArg1() {
      return value -> implementation.apply(new Object[] {value});
    }

    @Override
    public java.util.function.BiFunction<Object, Object, Object> getArg2() {
      return (first, second) -> implementation.apply(new Object[] {first, second});
    }

    @Override
    public Function<Object, Object> getArgN() {
      return values -> implementation.apply((Object[]) values);
    }

    @Override
    @TruffleBoundary
    @SuppressWarnings({"rawtypes", "unchecked"})
    public Object apply(Object[] arguments) {
      return IFn.applyAsArray(this, arguments);
    }

    @Override
    public String origin() {
      return origin;
    }

    @Override
    public void setOrigin(String origin) {
      this.origin = origin;
    }

    @Override
    public boolean recordsExceptionCreation() {
      return recordsExceptionCreation;
    }

    @Override
    public String toString() {
      return "#<builtin " + name + ">";
    }
  }

  private final class HaraNamespace {
    private final String name;
    private final Map<String, HaraVar> vars = new ConcurrentHashMap<>();
    private String role = "standard";

    private HaraNamespace(String name) {
      this.name = name;
    }

    private String name() {
      return name;
    }

    private HaraVar lookup(String symbolName) {
      return vars.get(symbolName);
    }

    private java.util.List<String> symbolNames() {
      return new java.util.ArrayList<>(vars.keySet());
    }

    private java.util.List<String> sortedSymbolNames() {
      java.util.ArrayList<String> names = new java.util.ArrayList<>();
      vars.forEach(
          (symbolName, variable) -> {
            if (name.equals(variable.namespaceName())) names.add(symbolName);
          });
      java.util.Collections.sort(names);
      return names;
    }

    @TruffleBoundary
    private HaraVar define(String symbolName, Object value) {
      return define(symbolName, value, null);
    }

    @TruffleBoundary
    private HaraVar define(String symbolName, Object value, IMetadata metadata) {
      return define(symbolName, value, metadata, definitionOrigin);
    }

    @TruffleBoundary
    private HaraVar define(
        String symbolName, Object value, IMetadata metadata, HaraVar.Origin origin) {
      if (value instanceof HaraBuiltinFunction builtin) {
        // A collected builtin may be exposed by several native namespaces. Preserve the
        // defining namespace recorded when it first enters the inventory; an export alias is
        // not a new function origin.
        if (builtin.origin() == null) {
          builtin.setOrigin(name + "/" + symbolName);
        }
      }
      if (collectingBuiltins && name.equals(collectingBuiltinNamespace)) {
        registerBuiltin(name, symbolName, value, metadata, origin);
        return new HaraVar(name, symbolName, value, metadata, origin);
      }
      HaraVar variable = vars.compute(
          symbolName,
          (ignored, existing) -> {
            if (existing == null) {
              return new HaraVar(name, symbolName, value, metadata, origin);
            }
            if (!name.equals(existing.namespaceName())) {
              return new HaraVar(name, symbolName, value, metadata, origin);
            }
            existing.set(value);
            existing.setMetadata(metadata);
            existing.setOrigin(origin);
            return existing;
          });
      refreshSchemaContract(variable);
      resolvePendingSchemaContracts(variable);
      return variable;
    }

    @SuppressWarnings({"rawtypes", "unchecked"})
    private static IMetadata mergeMetadata(IMetadata existing, IMetadata fallback) {
      if (!(fallback instanceof hara.lang.protocol.IMapType)) return existing;
      hara.lang.protocol.IMapType merged =
          existing instanceof hara.lang.protocol.IMapType
              ? (hara.lang.protocol.IMapType) existing
              : hara.lang.data.Map.Standard.EMPTY;
      java.util.Iterator<java.util.Map.Entry> entries =
          ((hara.lang.protocol.IMapType) fallback).iterator();
      while (entries.hasNext()) {
        java.util.Map.Entry entry = entries.next();
        merged = (hara.lang.protocol.IMapType) merged.assoc(entry.getKey(), entry.getValue());
      }
      return merged;
    }

    private HaraVar refer(String symbolName, HaraVar value) {
      vars.put(symbolName, value);
      return value;
    }

    private void removeReferredVars() {
      vars.entrySet().removeIf(entry -> !name.equals(entry.getValue().namespaceName()));
    }

    private void removeReferredVar(String symbolName) {
      vars.computeIfPresent(
          symbolName,
          (ignored, variable) -> name.equals(variable.namespaceName()) ? variable : null);
    }

    private void removeReferredVar(String symbolName, String sourceNamespace) {
      vars.computeIfPresent(
          symbolName,
          (ignored, variable) ->
              sourceNamespace.equals(variable.namespaceName()) ? null : variable);
    }
  }

  private enum NamespaceLoadState {
    UNLOADED("unloaded"),
    LOADING("loading"),
    LOADED("loaded"),
    FAILED("failed");

    private final String keyword;

    NamespaceLoadState(String keyword) {
      this.keyword = keyword;
    }
  }
}
