package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import hara.kernel.base.Parser;
import hara.lang.data.Keyword;
import hara.lang.protocol.ILinearType;
import hara.lang.protocol.IMapType;
import hara.spec.SpecRegistry;
import java.nio.file.Files;
import java.nio.file.Path;
import org.graalvm.polyglot.Context;
import org.graalvm.polyglot.Value;
import org.graalvm.polyglot.io.IOAccess;
import org.junit.Test;

/** Executes the Host/session boundary cases declared by the Host runtime specification. */
@org.junit.experimental.categories.Category(hara.spec.RegistryConformance.class)
public class HaraHostSessionConformanceTest {
  private static final Path SPEC =
      SpecRegistry.resolve("00-unsorted/runtime/draft/host-runtime.edn");

  @Test
  public void hostFacadeIdentityAndNamespaceStateAreSessionLocal() throws Exception {
    requireCase("host/type-identity");
    requireCase("host/session-local-facade");
    requireCase("host/namespace-loaded");

    try (SessionKernel kernel = new SessionKernel(false, false)) {
      SessionKernel.Session alpha = kernel.create(SessionModel.SessionId.parse("alpha"));
      SessionKernel.Session beta = kernel.create(SessionModel.SessionId.parse("beta"));
      assertTrue(
          alpha
              .eval("(= Host std.native.Host std.foundation/Host)")
              .asBoolean());
      assertTrue(
          beta
              .eval("(= Host std.native.Host std.foundation/Host)")
              .asBoolean());
      Value alphaHost = alpha.eval("Host");
      Value betaHost = beta.eval("Host");
      assertFalse(alphaHost.equals(betaHost));
      assertEquals("loaded", alpha.eval("(name (ns-state 'std.native))").asString());
      assertEquals(
          "loaded",
          alpha.eval("(name (ns-state 'std.native.Host))").asString());
    }
  }

  @Test
  public void capabilitiesAreDerivedFromEachSessionContext() throws Exception {
    requireCase("host/session-grants");

    try (Context denied = Context.newBuilder(HaraLanguage.ID).build();
        Context granted =
            Context.newBuilder(HaraLanguage.ID).allowIO(IOAccess.ALL).build()) {
      assertFalse(
          denied
              .eval(
                  HaraLanguage.ID,
                  "(deref (Host/capability? \"filesystem\"))")
              .asBoolean());
      assertTrue(
          granted
              .eval(
                  HaraLanguage.ID,
                  "(deref (Host/capability? \"filesystem\"))")
              .asBoolean());
    }
  }

  @Test
  public void sessionTransferRejectsTheHostFacadeAndItsLiveValues() throws Exception {
    requireCase("host/no-live-transfer");

    try (SessionKernel kernel = new SessionKernel(false, false)) {
      SessionKernel.Session session = kernel.create(SessionModel.SessionId.parse("transfer"));
      IllegalArgumentException error =
          assertThrows(
              IllegalArgumentException.class,
              () -> session.evalTransfer("Host"));
      assertTrue(error.getMessage().contains("SESSION_TRANSFER_REJECTED"));
    }
  }

  @Test
  public void rejectedHostCallsRetainStructuredExceptionInfo() throws Exception {
    requireCase("host/rejected-ex-info");

    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      Value result =
          context.eval(
              HaraLanguage.ID,
              """
              (try
                (deref (Host/call "missing" "missing" []))
                (catch Throwable error
                  [(ex-message error)
                   (name (get (ex-data error) :ex/code))]))
              """);
      assertEquals("Host method is unavailable: missing/missing", result.getArrayElement(0).asString());
      assertEquals("method-unavailable", result.getArrayElement(1).asString());

      assertEquals(
          "method-unavailable",
          context
              .eval(
                  HaraLanguage.ID,
                  """
                  (deref
                    (promise/catch
                      (Host/call "missing" "missing" [])
                      (fn [error]
                        (name (get (ex-data error) :ex/code)))))
                  """)
              .asString());
    }

    try (SessionKernel kernel = new SessionKernel(false, false)) {
      assertEquals(
          ":host/method-unavailable",
          hara.lang.base.G.display(
              kernel
                  .create(SessionModel.SessionId.parse("host-error"))
                  .evalTransfer(
                      """
                      (try
                        (deref (Host/call "missing" "missing" []))
                        (catch Throwable error
                          (get (ex-data error) :ex/code)))
                      """)));
    }
  }

  private static void requireCase(String id) throws Exception {
    Object document = Parser.LispReader.readString(Files.readString(SPEC), null);
    ILinearType entries =
        (ILinearType)
            ((IMapType) document).lookup(Keyword.create("host", "conformance"));
    Keyword expected = keyword(id);
    for (int index = 0; index < entries.count(); index++) {
      IMapType entry = (IMapType) entries.nth(index);
      if (expected.equals(entry.lookup(Keyword.create("id")))) return;
    }
    throw new AssertionError("Missing Host conformance case :" + id);
  }

  private static Keyword keyword(String value) {
    int separator = value.indexOf('/');
    return Keyword.create(
        value.substring(0, separator), value.substring(separator + 1));
  }
}
