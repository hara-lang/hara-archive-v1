package hara.truffle;

import com.oracle.truffle.api.CallTarget;
import com.oracle.truffle.api.Option;
import com.oracle.truffle.api.TruffleLanguage;
import com.oracle.truffle.api.nodes.Node;
import com.oracle.truffle.api.source.Source;
import com.oracle.truffle.api.source.SourceSection;
import hara.kernel.base.Parser;
import hara.kernel.base.Reader;
import hara.truffle.bytecode.HbcBytecodeRootNode;
import hara.truffle.bytecode.HbcCodec;
import hara.lang.data.Keyword;
import hara.lang.data.Map;
import hara.lang.protocol.IMetadata;
import hara.lang.protocol.IObjType;
import java.util.ArrayList;
import java.util.List;
import org.graalvm.options.OptionCategory;
import org.graalvm.options.OptionDescriptors;
import org.graalvm.options.OptionKey;
import org.graalvm.options.OptionStability;

@TruffleLanguage.Registration(
    id = HaraLanguage.ID,
    name = "Hara",
    implementationName = "Hara Truffle",
    version = "0.1",
    defaultMimeType = HaraLanguage.MIME_TYPE,
    characterMimeTypes = HaraLanguage.MIME_TYPE,
    byteMimeTypes = HaraLanguage.BYTECODE_MIME_TYPE)
public final class HaraLanguage extends TruffleLanguage<HaraContext> {
  public static final String ID = "hara";
  public static final String MIME_TYPE = "application/x-hara";
  public static final String BYTECODE_MIME_TYPE = "application/x-hara-bytecode";

  @Option(
      name = "TestRunner",
      help = "Select the runtime-owned test runner: code.test or native.",
      category = OptionCategory.USER,
      stability = OptionStability.STABLE)
  static final OptionKey<String> TEST_RUNNER = new OptionKey<>("code.test");

  @Option(
      name = "SandboxRestricted",
      help = "Hide host control-plane namespaces inside a private sandbox Runtime.",
      category = OptionCategory.INTERNAL,
      stability = OptionStability.STABLE)
  static final OptionKey<Boolean> SANDBOX_RESTRICTED = new OptionKey<>(false);

  @Option(
      name = "KernelToken",
      help = "Internal token for a trusted SessionKernel embedding.",
      category = OptionCategory.INTERNAL,
      stability = OptionStability.STABLE)
  static final OptionKey<String> KERNEL_TOKEN = new OptionKey<>("");

  @Option(
      name = "SessionId",
      help = "Internal Session identity used to bind runtime-owned instrumentation targets.",
      category = OptionCategory.INTERNAL,
      stability = OptionStability.STABLE)
  static final OptionKey<String> SESSION_ID = new OptionKey<>("");

  @Option(
      name = "FilesystemBindingToken",
      help = "Internal one-use token for an exact Session filesystem binding.",
      category = OptionCategory.INTERNAL,
      stability = OptionStability.STABLE)
  static final OptionKey<String> FILESYSTEM_BINDING_TOKEN = new OptionKey<>("");

  private static final ContextReference<HaraContext> CONTEXT_REFERENCE =
      ContextReference.create(HaraLanguage.class);

  @Override
  protected HaraContext createContext(Env environment) {
    return new HaraContext(environment);
  }

  @Override
  protected void initializeContext(HaraContext context) {
    context.ensureEagerFallbacks();
    context.markInstrumentationReady();
  }

  @Override
  protected OptionDescriptors getOptionDescriptors() {
    return new HaraLanguageOptionDescriptors();
  }

  @Override
  protected void finalizeContext(HaraContext context) {
    context.closeContext();
  }

  public static HaraContext currentContext() {
    return getCurrentContext(HaraLanguage.class);
  }

  /**
   * Node-local context lookup for hot execution paths: resolves through the node's root and
   * engine caches instead of walking the stack the way {@link #currentContext()} does.
   */
  public static HaraContext currentContext(Node node) {
    return CONTEXT_REFERENCE.get(node);
  }

  static HaraLanguage currentLanguage() {
    return getCurrentLanguage(HaraLanguage.class);
  }

  @Override
  protected boolean isThreadAccessAllowed(Thread thread, boolean singleThreaded) {
    return true;
  }

  @Override
  protected CallTarget parse(ParsingRequest request) {
    HaraContext context = currentContext();
    Source source = request.getSource();
    if (source.hasBytes() || BYTECODE_MIME_TYPE.equals(source.getMimeType())) {
      try {
        return HbcBytecodeRootNode.compile(this, HbcCodec.decode(source.getBytes().toByteArray()));
      } catch (RuntimeException error) {
        throw new HaraException("Unable to read Hara bytecode " + source.getName() + ": " + error.getMessage());
      }
    }
    SourceSection sourceSection =
        source.getLength() == 0
            ? source.createUnavailableSection()
            : source.createSection(0, source.getLength());
    Object[] forms;
    try {
      forms = readAll(source.getCharacters().toString(), source.getName());
    } catch (hara.kernel.base.Parser.LispReader.ReaderException error) {
      Throwable cause = error.getCause();
      String detail = cause == null ? error.getMessage() : cause.getMessage();
      throw new HaraException(
          "Unable to read Hara source "
              + source.getName()
              + " at line "
              + error.line()
              + ", column "
              + error.column()
              + ": "
              + detail);
    }
    ensureFoundationWhenDemanded(forms, context);
    return HaraAnalyzer.compile(this, forms, sourceSection, context);
  }

  static CallTarget compileHalc(Object[] forms, String sourceName) {
    HaraContext context = currentContext();
    ensureFoundationWhenDemanded(forms, context);
    return FoundationHalcLowerer.compile(currentLanguage(), context, forms);
  }

  static CallTarget compileHalc(HalcArtifact.Module module, String sourceName) {
    HaraContext context = currentContext();
    ensureFoundationWhenDemanded(module.forms, context);
    context.installHalcSchemas(module.schemas);
    return FoundationHalcLowerer.compile(currentLanguage(), context, module.forms);
  }

  private static void ensureFoundationWhenDemanded(Object[] forms, HaraContext context) {
    if (foundationSensitiveNamespaceConfiguration(forms)
        || FoundationFallbackDefinitions.requiresInitialization(forms, context)
        || FoundationFallbackDemand.requires(forms, context)) {
      context.ensureEagerFallbacks();
    }
  }

  /**
   * A later lazy load refreshes ordinary namespaces with all Foundation Vars. Materialize before
   * applying selective exposure or override configuration so the declaration can remove exactly
   * the bindings it intends and a later evaluation cannot reintroduce them.
   */
  @SuppressWarnings("rawtypes")
  private static boolean foundationSensitiveNamespaceConfiguration(Object[] forms) {
    Keyword config = Keyword.create("config");
    Keyword override = Keyword.create("override");
    Keyword expose = Keyword.create("expose");
    Keyword require = Keyword.create("require");
    for (Object form : forms) {
      if (!(form instanceof hara.lang.data.List<?> declaration)
          || declaration.count() == 0
          || !(declaration.nth(0) instanceof hara.lang.data.Symbol operator)
          || operator.getNamespace() != null
          || !("ns".equals(operator.getName()) || "ns+".equals(operator.getName()))) {
        continue;
      }
      int start = "ns".equals(operator.getName()) ? 2 : 1;
      for (int index = start; index < declaration.count(); index++) {
        if (!(declaration.nth(index) instanceof hara.lang.data.List<?> clause)
            || clause.count() == 0) {
          continue;
        }
        if (require.equals(clause.nth(0))) {
          for (int requirement = 1; requirement < clause.count(); requirement++) {
            if (requiresFoundationNamespace(clause.nth(requirement))) return true;
          }
        }
        if (clause.count() == 2
            && config.equals(clause.nth(0))
            && clause.nth(1) instanceof hara.lang.protocol.IMapType<?, ?> options) {
          hara.lang.protocol.IMapType raw = options;
          if (raw.lookup(override) != null || raw.lookup(expose) != null) return true;
        }
      }
    }
    return false;
  }

  private static boolean requiresFoundationNamespace(Object value) {
    if (!(value instanceof hara.lang.protocol.ILinearType<?> requirement)
        || requirement.count() == 0
        || !(requirement.nth(0) instanceof hara.lang.data.Symbol namespace)) {
      return false;
    }
    return "std.foundation".equals(namespace.display());
  }

  static Object[] readAll(String source, String sourceName) {
    Reader reader = new Reader(source);
    Object eof = new Object();
    List<Object> forms = new ArrayList<>();
    Object form;
    do {
      form = Parser.LispReader.read(reader, false, eof, false, null);
      if (form != eof) {
        forms.add(withSourceName(form, sourceName));
      }
    } while (form != eof);
    return forms.toArray();
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  private static Object withSourceName(Object form, String sourceName) {
    if (!(form instanceof IObjType) || sourceName == null) return form;
    IObjType object = (IObjType) form;
    IMetadata metadata = object.meta();
    hara.lang.protocol.IMapType updated =
        metadata instanceof hara.lang.protocol.IMapType
            ? (hara.lang.protocol.IMapType)
                ((hara.lang.protocol.IMapType) metadata)
                    .assoc(Keyword.create("file"), sourceName)
            : Map.Standard.from(null, Keyword.create("file"), sourceName);
    return object.withMeta(updated);
  }
}
