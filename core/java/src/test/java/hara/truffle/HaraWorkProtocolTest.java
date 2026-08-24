package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import hara.lang.protocol.IClosed;
import hara.lang.protocol.IComponent;
import hara.lang.protocol.IWork;
import hara.lang.protocol.IWorkExecutor;
import hara.lang.protocol.IWorkHost;
import hara.lang.protocol.IWorkRef;
import hara.lang.protocol.IWorkRun;
import hara.lang.protocol.IWorkStore;
import hara.spec.SpecRegistry;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Map;
import org.graalvm.polyglot.Context;
import org.junit.Test;

public class HaraWorkProtocolTest {
  @Test
  public void nativeInterfacesPreserveTheWorkLifecycleHierarchy() {
    assertTrue(IComponent.class.isAssignableFrom(IWorkHost.class));
    assertTrue(IWorkRef.class.isAssignableFrom(IWorkRun.class));
    assertTrue(IClosed.class.isAssignableFrom(IWorkRun.class));
    assertFalse(IComponent.class.isAssignableFrom(IWorkExecutor.class));
    assertFalse(IComponent.class.isAssignableFrom(IWorkStore.class));
  }

  @Test
  public void adaptsJavaWorkAndReferenceValues() {
    HaraProtocol work = new HaraProtocol("IWork", Map.of("work-spec", 1));
    HaraProtocolRuntime.installForTest(work);
    HaraProtocol reference = new HaraProtocol("IWorkRef", Map.of("work-id", 1));
    HaraProtocolRuntime.installForTest(reference);

    IWork workValue = () -> Map.of("op", "pure");
    IWorkRef referenceValue = () -> "run-1";

    assertEquals(Map.of("op", "pure"), work.invoke("work-spec", workValue, new Object[0]));
    assertEquals("run-1", reference.invoke("work-id", referenceValue, new Object[0]));
  }

  @Test
  public void adaptsJavaExecutorAndStoreValues() {
    HaraProtocol executor =
        new HaraProtocol("IWorkExecutor", Map.of("work-execute", 2));
    HaraProtocolRuntime.installForTest(executor);
    HaraProtocol store =
        new HaraProtocol(
            "IWorkStore", Map.of("work-query", 2, "work-transact", 2));
    HaraProtocolRuntime.installForTest(store);

    IWorkExecutor executorValue = request -> Map.of("executed", request);
    IWorkStore storeValue =
        new IWorkStore() {
          @Override
          public Object workQuery(Object query) {
            return Map.of("query", query);
          }

          @Override
          public Object workTransact(Object transition) {
            return Map.of("transact", transition);
          }
        };

    Map<String, String> request = Map.of("leaf", "compile");
    Map<String, String> query = Map.of("query", "run");
    Map<String, String> transition = Map.of("run", "run-1");

    assertEquals(
        Map.of("executed", request),
        executor.invoke("work-execute", executorValue, new Object[] {request}));
    assertEquals(
        Map.of("query", query),
        store.invoke("work-query", storeValue, new Object[] {query}));
    assertEquals(
        Map.of("transact", transition),
        store.invoke("work-transact", storeValue, new Object[] {transition}));
  }

  @Test
  @org.junit.experimental.categories.Category(hara.spec.RegistryConformance.class)
  public void guestTypesExtendNativeWorkProtocolsAndParents() throws Exception {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      String corpus =
          Files.readString(
              specsRegistry()
                  .resolve(
                      "01-lang/001-language/draft/conformance/fixtures/protocol_behavioral.hal"));
      context.eval(HaraLanguage.ID, corpus);
      String methods = context.eval(HaraLanguage.ID, "(capability-protocol-results)").toString();
      String receivers =
          context.eval(HaraLanguage.ID, "(protocol-capability-receiver-results)").toString();
      assertFalse(methods, methods.contains(":pass false"));
      assertEquals(20, methods.split(":pass true", -1).length - 1);
      assertFalse(receivers, receivers.contains(":pass false"));
      assertEquals(8, receivers.split(":pass true", -1).length - 1);
    }
  }

  @Test
  public void legacyWorkValuesUseUnqualifiedNativeProtocols() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertTrue(context.eval(HaraLanguage.ID, "IWork").toString().contains("IWork"));
      assertTrue(
          context
              .eval(HaraLanguage.ID, "(ns work.protocol.legacy) IWork")
              .toString()
              .contains("IWork"));
      assertEquals(
          "[{:op :pure} \"run-legacy\"]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns work.protocol.legacy "
                      + "(:require [work.base.model :as base])) "
                      + "(let [work (base/work-value {:op :pure}) "
                      + "      reference (base/work-reference \"run-legacy\")] "
                      + "  [(IWork/work-spec work) "
                      + "   (IWorkRef/work-id reference)])")
              .toString());
    }
  }

  private static Path specsRegistry() {
    return SpecRegistry.root();
  }
}
