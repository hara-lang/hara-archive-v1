package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import hara.kernel.base.Parser;
import hara.lang.data.Keyword;
import hara.lang.protocol.ILinearType;
import hara.lang.protocol.IMapType;
import hara.spec.SpecRegistry;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.Map;
import java.util.Set;
import org.graalvm.polyglot.Context;
import org.graalvm.polyglot.PolyglotException;
import org.graalvm.polyglot.io.IOAccess;
import org.junit.Test;

/**
 * Executes the module scenarios whose portable fixtures are implemented by the
 * JVM runtime.
 *
 * <p>The corpus remains the source of case identity and expected behavior.
 * Runtime fixtures fail if a referenced case disappears or its expectation
 * changes without a corresponding implementation change.
 */
@org.junit.experimental.categories.Category(hara.spec.RegistryConformance.class)
public class HaraModuleConformanceTest {
  private static final Path CORPUS =
      SpecRegistry.resolve("01-lang/001-language/draft/conformance/modules.edn");

  @Test
  public void executesNamespaceLoadingTransactionAndReloadScenarios()
      throws Exception {
    Map<Object, IMapType> cases = cases();
    Set<Object> executed = new LinkedHashSet<>();

    assertExpectation(cases, "namespace/loading-state", "non-forcing", Boolean.TRUE);
    assertExpectation(
        cases, "namespace/cross-namespace-alias-state", "owner-explicit", Boolean.TRUE);
    assertExpectation(cases, "lazy/non-forcing", "state", Keyword.create("unloaded"));
    assertExpectation(cases, "lazy/failure-state", "state", Keyword.create("failed"));
    assertExpectation(
        cases, "lazy/explicit-retry", "ordinary-force-retries", Boolean.FALSE);
    assertExpectation(cases, "lazy/explicit-retry", "reload-retries", Boolean.TRUE);
    assertExpectation(cases, "module/reload-revision", "revision-increment", 1L);
    assertExpectation(
        cases, "module/reload-rollback", "previous-revision-preserved", Boolean.TRUE);

    new HaraProjectTest()
        .lazyNamespaceStateIsNonForcingAndFailedLoadsRequireExplicitReload();
    executed.addAll(
        ids(
            "namespace/loading-state",
            "namespace/cross-namespace-alias-state",
            "lazy/non-forcing",
            "lazy/qualified-force",
            "lazy/failure-state",
            "lazy/explicit-retry",
            "module/failure-rollback",
            "module/reload-revision",
            "module/reload-rollback",
            "session/namespace-isolation"));

    HaraLanguageTest language = new HaraLanguageTest();
    language.failedModuleEvaluationRollsBackVarsMacrosAndNamespace();
    language.requireCachesCanonicalModulesAndLoadFileIncrementsRevision();
    language.reloadingARequiredMacroModuleRefreshesNewCompilations();
    language.requireRejectsCyclesAndRollsBackPartialModules();
    executed.addAll(
        ids(
            "module/canonical-cache",
            "module/cycle-rollback",
            "module/failure-rollback",
            "module/reload-revision",
            "module/reload-rollback",
            "macro/reload-new-compilation"));

    assertTrue("No module scenarios executed", !executed.isEmpty());
    assertTrue("Runner referenced an unknown module case", cases.keySet().containsAll(executed));
  }

  @Test
  public void executesVarIdentityScenarios() throws Exception {
    Map<Object, IMapType> cases = cases();
    assertExpectation(cases, "namespace/alias-var-identity", "same-var", Boolean.TRUE);
    assertExpectation(cases, "namespace/refer-var-identity", "same-var", Boolean.TRUE);

    HaraLanguageTest language = new HaraLanguageTest();
    language.resolvesQualifiedSymbolsThroughContextLocalAliases();
    language.refersLiveVarIdentityAcrossNamespaces();
  }

  @Test
  public void executesCallableVarScenariosFromSharedSpec() throws Exception {
    Map<Object, IMapType> cases = cases();
    for (String id :
        new String[] {
          "namespace/callable-var-precedence",
          "namespace/callable-var-lexical-shadow",
          "namespace/callable-var-late-binding",
          "namespace/referred-var-shadowed"
        }) {
      IMapType scenario = cases.get(keyword(id));
      assertTrue("Missing callable Var scenario :" + id, scenario != null);
      String setup = (String) scenario.lookup(Keyword.create("setup"));
      String source = (String) scenario.lookup(Keyword.create("source"));
      IMapType expectation = requireMap(scenario, "expect");
      try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
        context.eval(HaraLanguage.ID, setup);
        Object display = expectation.lookup(Keyword.create("display"));
        if (display instanceof String) {
          assertEquals(id, display, context.eval(HaraLanguage.ID, source).toString());
        } else {
          String marker =
              (String) expectation.lookup(Keyword.create("error-contains"));
          PolyglotException error =
              assertThrows(
                  id,
                  PolyglotException.class,
                  () -> context.eval(HaraLanguage.ID, source));
          assertTrue(id + ": " + error.getMessage(), error.getMessage().contains(marker));
        }
      }
    }
  }

  @Test
  public void executesDependencyFirstSourceOrderScenario() throws Exception {
    Map<Object, IMapType> cases = cases();
    assertExpectation(
        cases,
        "module/dependency-order",
        "order",
        Keyword.create("dependency-first-source-order"));

    Path directory = Files.createTempDirectory("hara-module-order-");
    Path dependency = directory.resolve("dependency.hal");
    Path root = directory.resolve("root.hal");
    try {
      Files.writeString(dependency, "(ns module.dependency) (def value 41)");
      String dependencyPath = escaped(dependency);
      Files.writeString(
          root,
          "(ns module.root) "
              + "(require \""
              + dependencyPath
              + "\" {:as 'dependency}) "
              + "(def answer (+ dependency/value 1))");

      try (Context context =
          Context.newBuilder(HaraLanguage.ID).allowIO(IOAccess.ALL).build()) {
        context.eval(HaraLanguage.ID, "(require \"" + escaped(root) + "\")");
        assertEquals(42, context.eval(HaraLanguage.ID, "module.root/answer").asInt());
      }
    } finally {
      Files.deleteIfExists(root);
      Files.deleteIfExists(dependency);
      Files.deleteIfExists(directory);
    }
  }

  @Test
  public void executesWithNamespaceScenarios() throws Exception {
    Map<Object, IMapType> cases = cases();
    assertExpectation(cases, "namespace/with-ns-success", "caller-restored", Boolean.TRUE);
    assertExpectation(cases, "namespace/with-ns-failure", "caller-restored", Boolean.TRUE);
    assertExpectation(
        cases,
        "namespace/with-ns-lexical-isolation",
        "caller-locals-visible",
        Boolean.FALSE);

    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      context.eval(
          HaraLanguage.ID,
          "(ns target) (def answer 41) (ns user) (def caller-value 7)");
      assertEquals(
          42,
          context
              .eval(HaraLanguage.ID, "(with-ns 'target (def answer 42) answer)")
              .asInt());
      assertEquals(42, context.eval(HaraLanguage.ID, "target/answer").asInt());
      assertEquals(7, context.eval(HaraLanguage.ID, "caller-value").asInt());

      assertThrows(
          PolyglotException.class,
          () ->
              context.eval(
                  HaraLanguage.ID,
                  "(with-ns 'target (throw :with-ns-failed))"));
      assertEquals(7, context.eval(HaraLanguage.ID, "caller-value").asInt());

      assertThrows(
          PolyglotException.class,
          () ->
              context.eval(
                  HaraLanguage.ID,
                  "(let [caller-local 42] (with-ns 'target caller-local))"));
      assertEquals(7, context.eval(HaraLanguage.ID, "caller-value").asInt());
    }
  }

  @Test
  public void executesFacadeVarCopyScenario() throws Exception {
    Map<Object, IMapType> cases = cases();
    assertExpectation(cases, "namespace/facade-var-copy", "same-var", Boolean.FALSE);
    assertExpectation(cases, "namespace/facade-var-copy", "copied-root", Boolean.TRUE);
    assertExpectation(cases, "namespace/facade-var-copy", "copied-metadata", Boolean.TRUE);

    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      context.eval(
          HaraLanguage.ID,
          "(ns source (:config {:blank true})) (def ^{:doc \"copied\"} answer 41)");
      context.eval(
          HaraLanguage.ID,
          "(ns target (:config {:blank true})) "
              + "(std.foundation/intern-var 'target 'answer (var source/answer))");
      context.eval(HaraLanguage.ID, "(ns user)");
      assertTrue(
          !context
              .eval(
                  HaraLanguage.ID,
                  "(std.foundation/= (var source/answer) (var target/answer))")
              .asBoolean());
      assertEquals(
          41,
          context.eval(HaraLanguage.ID, "(deref (var target/answer))").asInt());
      assertEquals(
          "copied",
          context
              .eval(
                  HaraLanguage.ID,
                  "(std.foundation/get (std.foundation/meta (var target/answer)) :doc)")
              .asString());
    }
  }

  @Test
  public void declaresJvmResourcePrecedenceScenario() throws Exception {
    Map<Object, IMapType> cases = cases();
    assertExpectation(cases, "module/resource-precedence", "deterministic", Boolean.TRUE);
    assertExpectation(
        cases, "module/resource-precedence", "declared-by-runtime-profile", Boolean.TRUE);

    Object document = Parser.LispReader.readString(Files.readString(CORPUS), null);
    IMapType profiles =
        (IMapType) ((IMapType) document).lookup(Keyword.create("runtime", "profiles"));
    IMapType profile = (IMapType) profiles.lookup(Keyword.create("jvm-truffle"));
    ILinearType order = (ILinearType) profile.lookup(Keyword.create("resource-order"));
    Object[] expected = {
      Keyword.create("loaded-native-namespace"),
      Keyword.create("runtime-library"),
      Keyword.create("project-source"),
      Keyword.create("classpath-source"),
      Keyword.create("extension-package")
    };
    assertEquals(expected.length, order.count());
    for (int index = 0; index < expected.length; index++) {
      assertEquals(expected[index], order.nth(index));
    }

    Path root = Files.createTempDirectory("hara-module-precedence-");
    Path project = root.resolve("project.edn");
    Path projectSource = root.resolve("src/precedence/demo.hal");
    Path nativeShadow = root.resolve("src/std/native/Host.hal");
    try {
      Files.writeString(
          project,
          "{:hara/type :project :project/id precedence "
              + ":project/source-paths [\"src\"]}");
      Files.createDirectories(projectSource.getParent());
      Files.writeString(
          projectSource,
          "(ns precedence.demo) (def source :project)");
      Files.createDirectories(nativeShadow.getParent());
      Files.writeString(
          nativeShadow,
          "(ns std.native.Host) (def shadow-loaded true)");

      try (Context context =
          Context.newBuilder(HaraLanguage.ID)
              .currentWorkingDirectory(root)
              .allowIO(IOAccess.ALL)
              .build()) {
        assertEquals(
            ":project",
            context
                .eval(
                    HaraLanguage.ID,
                    "(ns user (:require [precedence.demo :as precedence])) precedence/source")
                .toString());
        context.eval(HaraLanguage.ID, "(require 'std.native.Host)");
        assertEquals(
            "nil",
            context
                .eval(
                    HaraLanguage.ID,
                    "(pr-str (resolve 'Host/shadow-loaded))")
                .asString());
      }
    } finally {
      Files.deleteIfExists(nativeShadow);
      Files.deleteIfExists(nativeShadow.getParent());
      Files.deleteIfExists(nativeShadow.getParent().getParent());
      Files.deleteIfExists(projectSource);
      Files.deleteIfExists(projectSource.getParent());
      Files.deleteIfExists(projectSource.getParent().getParent());
      Files.deleteIfExists(root.resolve("src"));
      Files.deleteIfExists(project);
      Files.deleteIfExists(root);
    }
  }

  @Test
  public void executesSessionDynamicBindingScenario() throws Exception {
    Map<Object, IMapType> cases = cases();
    assertExpectation(
        cases, "session/dynamic-unwind", "binding-session-local", Boolean.TRUE);
    assertExpectation(
        cases, "session/dynamic-unwind", "restored-after-error", Boolean.TRUE);

    try (SessionKernel kernel = new SessionKernel(false, false)) {
      SessionKernel.Session alpha = kernel.create(SessionModel.SessionId.parse("alpha"));
      SessionKernel.Session beta = kernel.create(SessionModel.SessionId.parse("beta"));
      alpha.eval("(def ^:dynamic *answer* 1)");
      beta.eval("(def ^:dynamic *answer* 10)");
      assertThrows(
          IllegalArgumentException.class,
          () -> alpha.eval("(binding [*answer* 2] (throw :binding-failed))"));
      assertEquals(1, alpha.eval("*answer*").asInt());
      assertEquals(10, beta.eval("*answer*").asInt());
    }
  }

  @Test
  public void executesSessionTransferScenarios() throws Exception {
    Map<Object, IMapType> cases = cases();
    assertExpectation(cases, "session/immutable-transfer", "immutable-data", Boolean.TRUE);
    for (String kind :
        new String[] {
          "functions",
          "vars",
          "mutable-references",
          "streams",
          "sockets",
          "host-handles"
        }) {
      assertExpectation(cases, "session/reject-live-transfer", kind, Boolean.FALSE);
    }

    try (SessionKernel kernel = new SessionKernel(false, false)) {
      SessionKernel.Session session = kernel.create(SessionModel.SessionId.parse("transfer"));
      assertEquals(
          "{:answer [1 2 {:nested #{:immutable}}]}",
          hara.lang.base.G.display(
              session.evalTransfer("{:answer [1 2 {:nested #{:immutable}}]}")));
      session.eval("(def answer 42)");
      for (String source :
          new String[] {
            "(fn [value] value)",
            "(var answer)",
            "(atom 1)",
            "(iter [1 2 3])",
            "(Host/call \"missing\" \"missing\" [])"
          }) {
        IllegalArgumentException error =
            assertThrows(IllegalArgumentException.class, () -> session.evalTransfer(source));
        assertTrue(source, error.getMessage().contains("SESSION_TRANSFER_REJECTED"));
      }
    }
  }

  @Test
  public void executesRetainedReplScenarios() throws Exception {
    Map<Object, IMapType> cases = cases();
    assertExpectation(cases, "repl/retained-state", "namespace-retained", Boolean.TRUE);
    assertExpectation(cases, "repl/retained-state", "history-retained", Boolean.TRUE);
    assertExpectation(cases, "repl/retained-state", "multiline", Boolean.TRUE);
    assertExpectation(cases, "repl/error-recovery", "session-survives", Boolean.TRUE);
    assertExpectation(cases, "repl/error-recovery", "namespace-restored", Boolean.TRUE);

    MainTest repl = new MainTest();
    repl.replRetainsContextAcrossInputsAndSupportsMultilineForms();
    repl.replRetainsCompletedFormHistory();
    repl.replContinuesAfterGuestErrors();
  }

  @Test
  public void executesSourceAndHirParityScenario() throws Exception {
    Map<Object, IMapType> cases = cases();
    assertExpectation(cases, "module/source-hir-parity", "same-value", Boolean.TRUE);
    assertExpectation(
        cases, "module/source-hir-parity", "same-var-metadata", Boolean.TRUE);
    assertExpectation(
        cases, "module/source-hir-parity", "same-error-category", Boolean.TRUE);

    String source = "(ns parity.demo) (defn value \"answer\" [] 42) (value)";
    Object[] forms = HaraLanguage.readAll(source, "parity/demo.hal");
    byte[] artifact =
        HalcArtifact.encode(
            "parity.demo",
            "parity/demo.hal",
            source.getBytes(StandardCharsets.UTF_8),
            forms);
    HalcArtifact.Module module = HalcArtifact.decode(artifact);

    try (Context sourceContext = Context.newBuilder(HaraLanguage.ID).build();
        Context hirContext = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(42, sourceContext.eval(HaraLanguage.ID, source).asInt());
      hirContext.eval(HaraLanguage.ID, "nil");
      Object hirResult;
      hirContext.enter();
      try {
        hirResult =
            HaraLanguage.compileHalc(module, "hir:parity/demo.hal").call();
      } finally {
        hirContext.leave();
      }
      assertEquals(42L, ((Number) hirResult).longValue());
      assertEquals(
          sourceContext
              .eval(HaraLanguage.ID, "(get (meta (var parity.demo/value)) :doc)")
              .asString(),
          hirContext
              .eval(HaraLanguage.ID, "(get (meta (var parity.demo/value)) :doc)")
              .asString());

      PolyglotException sourceError =
          assertThrows(
              PolyglotException.class,
              () -> sourceContext.eval(HaraLanguage.ID, "(throw :parity-failed)"));
      Object[] failure = HaraLanguage.readAll("(throw :parity-failed)", "parity/failure.hal");
      RuntimeException hirError;
      hirContext.enter();
      try {
        hirError =
            assertThrows(
                RuntimeException.class,
                () ->
                    HaraLanguage.compileHalc(failure, "hir:parity/failure.hal")
                        .call());
      } finally {
        hirContext.leave();
      }
      assertTrue(sourceError.isGuestException());
      assertTrue(hirError instanceof HaraException);
    }
  }

  @Test
  public void typedHalcUsesPrimitiveTruffleParameterSlotsWithGuardedFallback() {
    String source =
        "(ns typed.truffle) "
            + "(def Unary [:fn [:int] :int]) "
            + "(defn ^{:schema #'Unary} identity-int [value] value) "
            + "(identity-int 42)";
    HalcArtifact.Module module =
        HalcArtifact.decode(
            HalcArtifact.encode(
                "typed.truffle",
                "typed/truffle.hal",
                source.getBytes(StandardCharsets.UTF_8),
                HaraLanguage.readAll(source, "typed/truffle.hal")));
    long typedBefore = FoundationHalcLowerer.typedParameterSlotCount();

    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      context.eval(HaraLanguage.ID, "nil");
      context.enter();
      try {
        assertEquals(42L, ((Number) HaraLanguage.compileHalc(module, "typed/truffle.hal").call()).longValue());
      } finally {
        context.leave();
      }
      assertEquals("dynamic", context.eval(HaraLanguage.ID, "(identity-int \"dynamic\")").asString());
    }
    assertTrue(FoundationHalcLowerer.typedParameterSlotCount() > typedBefore);
  }

  @Test
  public void executesNsRequireReloadFromSharedSpecFixture() throws Exception {
    IMapType scenario = cases().get(keyword("module/ns-require-reload"));
    assertTrue("Missing shared ns reload scenario", scenario != null);
    IMapType fixture = requireMap(scenario, "fixture");
    IMapType resource = requireMap(fixture, "resource");
    String namespace = (String) resource.lookup(Keyword.create("namespace"));
    String relativePath = (String) resource.lookup(Keyword.create("path"));
    IMapType revisions = requireMap(resource, "revisions");
    Object rawSteps = fixture.lookup(Keyword.create("steps"));
    assertTrue("Reload fixture :steps must be sequential", rawSteps instanceof ILinearType);

    Path root = Files.createTempDirectory("hara-ns-reload-conformance-");
    Files.writeString(
        root.resolve("project.edn"),
        "{:hara/type :project :project/id conformance :project/source-paths [\"src\"]}");
    Path sourcePath = root.resolve("src").resolve(relativePath);
    Files.createDirectories(sourcePath.getParent());

    try (Context context =
        Context.newBuilder(HaraLanguage.ID)
            .currentWorkingDirectory(root)
            .allowIO(IOAccess.ALL)
            .build()) {
      ILinearType steps = (ILinearType) rawSteps;
      for (int index = 0; index < steps.count(); index++) {
        assertTrue("Reload fixture step must be a map", steps.nth(index) instanceof IMapType);
        IMapType step = (IMapType) steps.nth(index);
        Keyword operation = (Keyword) step.lookup(Keyword.create("op"));
        String operationName =
            operation.getNamespace() == null
                ? operation.getName()
                : operation.getNamespace() + "/" + operation.getName();
        if ("resource/use".equals(operationName)) {
          Keyword revision = (Keyword) step.lookup(Keyword.create("revision"));
          String source = (String) revisions.lookup(revision);
          assertTrue("Missing source for " + revision, source != null);
          Files.writeString(sourcePath, source);
          continue;
        }
        if ("assert/revision".equals(operationName)) {
          Number expected = (Number) step.lookup(Keyword.create("expect"));
          assertEquals(
              expected.longValue(),
              context
                  .eval(
                      HaraLanguage.ID,
                      "(std.foundation/module-revision \"" + escaped(sourcePath) + "\")")
                  .asLong());
          continue;
        }
        assertEquals("eval", operationName);
        String source = (String) step.lookup(Keyword.create("source"));
        IMapType expectation = requireMap(step, "expect");
        Object display = expectation.lookup(Keyword.create("display"));
        Object expectsError = expectation.lookup(Keyword.create("error"));
        Object marker = expectation.lookup(Keyword.create("error-contains"));
        if (display instanceof String) {
          assertEquals(display, context.eval(HaraLanguage.ID, source).toString());
        } else {
          PolyglotException error =
              assertThrows(
                  PolyglotException.class,
                  () -> context.eval(HaraLanguage.ID, source));
          if (Boolean.TRUE.equals(expectsError)) continue;
          assertTrue("Expected :error or :error-contains", marker instanceof String);
          assertTrue(
              error.getMessage(),
              error.getMessage().toLowerCase().contains(((String) marker).toLowerCase()));
        }
      }
    }
    assertEquals("conformance.reload-target", namespace);
  }

  private static Map<Object, IMapType> cases() throws Exception {
    Object document = Parser.LispReader.readString(Files.readString(CORPUS), null);
    assertTrue("Module corpus must be an EDN map", document instanceof IMapType);
    Object entries = ((IMapType) document).lookup(Keyword.create("cases"));
    assertTrue("Module corpus :cases must be sequential", entries instanceof ILinearType);
    Map<Object, IMapType> indexed = new LinkedHashMap<>();
    ILinearType values = (ILinearType) entries;
    for (int index = 0; index < values.count(); index++) {
      assertTrue("Module case must be a map", values.nth(index) instanceof IMapType);
      IMapType value = (IMapType) values.nth(index);
      Object id = value.lookup(Keyword.create("id"));
      assertTrue("Module case must have a keyword id", id instanceof Keyword);
      assertTrue("Duplicate module case " + id, indexed.put(id, value) == null);
    }
    return indexed;
  }

  private static void assertExpectation(
      Map<Object, IMapType> cases, String id, String key, Object expected) {
    IMapType scenario = cases.get(keyword(id));
    assertTrue("Missing executable module case :" + id, scenario != null);
    Object expectation = scenario.lookup(Keyword.create("expect"));
    assertTrue("Module case has no expectation map :" + id, expectation instanceof IMapType);
    assertEquals(
        "Unexpected module expectation :" + id + " :" + key,
        expected,
        ((IMapType) expectation).lookup(Keyword.create(key)));
  }

  private static IMapType requireMap(IMapType source, String key) {
    Object value = source.lookup(Keyword.create(key));
    assertTrue("Expected map at :" + key, value instanceof IMapType);
    return (IMapType) value;
  }

  private static Set<Object> ids(String... values) {
    Set<Object> ids = new LinkedHashSet<>();
    for (String value : values) ids.add(keyword(value));
    return ids;
  }

  private static Keyword keyword(String value) {
    int separator = value.indexOf('/');
    return separator < 0
        ? Keyword.create(value)
        : Keyword.create(value.substring(0, separator), value.substring(separator + 1));
  }

  private static String escaped(Path path) {
    return path.toString().replace("\\", "\\\\").replace("\"", "\\\"");
  }
}
