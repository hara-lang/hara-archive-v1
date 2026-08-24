package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.assertSame;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import hara.lang.data.Symbol;
import java.util.Map;
import org.graalvm.polyglot.Context;
import org.graalvm.polyglot.PolyglotException;
import org.junit.Test;

/**
 * Proves that one built-in protocol declaration emits dotted and slash products backed by the
 * same descriptor Vars, method Vars, and protocol dispatch state.
 */
public class ProtocolProductRegistrationTest {
  @Test
  public void dottedAndSlashProductsResolveAndShareExactVars() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).allowAllAccess(true).build()) {
      context.eval(HaraLanguage.ID, "nil");
      context.enter();
      try {
        HaraContext runtime = HaraLanguage.currentContext();
        HaraVar interfaceDescriptor =
            requireVar(runtime, "std.protocol.iassoc.IAssoc", "IAssoc");
        HaraVar protocolDescriptor = requireVar(runtime, "std.protocol.iassoc", "IAssoc");
        HaraVar interfaceMethod =
            requireVar(runtime, "std.protocol.iassoc.IAssoc", "assoc");
        HaraVar protocolMethod = requireVar(runtime, "std.protocol.iassoc", "assoc");

        assertSame(interfaceDescriptor, protocolDescriptor);
        assertSame(interfaceMethod, protocolMethod);
        assertSame(interfaceMethod.get(), protocolMethod.get());
        assertSame(interfaceMethod.meta(), protocolMethod.meta());
        assertEquals(interfaceMethod.origin(), protocolMethod.origin());
      } finally {
        context.leave();
      }

      assertTrue(
          context
              .eval(
                  HaraLanguage.ID,
                  "(= std.protocol.iassoc.IAssoc std.protocol.iassoc/IAssoc)")
              .asBoolean());
      assertTrue(
          context
              .eval(
                  HaraLanguage.ID,
                  "(= (std.protocol.iassoc/assoc {:a 1} :b 2) "
                      + "(std.protocol.iassoc.IAssoc/assoc {:a 1} :b 2))")
              .asBoolean());
      assertEquals(
          "{:answer 42}",
          context
              .eval(
                  HaraLanguage.ID,
                  "(std.protocol.iassoc.IAssoc/assoc {} :answer 42)")
              .toString());
    }
  }

  @Test
  public void pairedMethodsReportIdenticalArityErrors() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).allowAllAccess(true).build()) {
      String protocolError =
          guestError(context, "(std.protocol.iassoc/assoc {})");
      String interfaceError =
          guestError(context, "(std.protocol.iassoc.IAssoc/assoc {})");

      assertTrue(protocolError, protocolError.contains("protocol/arity"));
      assertEquals(protocolError, interfaceError);
    }
  }

  @Test
  public void extensionInstalledThroughOneProductIsVisibleThroughBoth() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).allowAllAccess(true).build()) {
      assertTrue(
          context
              .eval(
                  HaraLanguage.ID,
                  "(= (do "
                      + "(defstruct ProductBox [value]) "
                      + "(extend-type ProductBox std.protocol.iassoc/IAssoc "
                      + "  (assoc [this key value] [key value])) "
                      + "[(std.protocol.iassoc/assoc (ProductBox nil) :slash 1) "
                      + " (std.protocol.iassoc.IAssoc/assoc (ProductBox nil) :dotted 2)]) "
                      + "[[:slash 1] [:dotted 2]])")
              .asBoolean());
    }
  }

  @Test
  public void rootReplacementThroughEitherProductIsImmediatelyShared() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).allowAllAccess(true).build()) {
      context.eval(HaraLanguage.ID, "nil");
      context.enter();
      try {
        HaraContext runtime = HaraLanguage.currentContext();
        HaraVar interfaceMethod =
            requireVar(runtime, "std.protocol.iassoc.IAssoc", "assoc");
        HaraVar protocolMethod = requireVar(runtime, "std.protocol.iassoc", "assoc");
        Object original = interfaceMethod.get();
        try {
          Object first = runtime.libraryFunction("test.protocol/first", ignored -> "first");
          protocolMethod.reset(first);
          assertSame(first, interfaceMethod.get());
          assertEquals("first", runtime.invokeCallable(interfaceMethod.get(), new Object[0]));

          Object second = runtime.libraryFunction("test.protocol/second", ignored -> "second");
          interfaceMethod.reset(second);
          assertSame(second, protocolMethod.get());
          assertEquals("second", runtime.invokeCallable(protocolMethod.get(), new Object[0]));
        } finally {
          interfaceMethod.reset(original);
        }
      } finally {
        context.leave();
      }
    }
  }

  @Test
  public void collisionPreflightLeavesTheDeclarationUnpublished() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).allowAllAccess(true).build()) {
      context.eval(HaraLanguage.ID, "nil");
      context.enter();
      try {
        HaraContext runtime = HaraLanguage.currentContext();
        String interfaceNamespace = "test.product.IProbe";
        String ownerNamespace = "test.product";
        HaraProtocol protocol = new HaraProtocol(interfaceNamespace, Map.of("probe", 1));
        Object method =
            runtime.libraryFunction("test.product.IProbe/probe", ignored -> "probe");

        runtime.runInNamespace(
            interfaceNamespace,
            () -> {
              runtime.define(Symbol.create("IProbe"), protocol);
              runtime.define(Symbol.create("probe"), method);
            });
        HaraVar interfaceDescriptor = requireVar(runtime, interfaceNamespace, "IProbe");
        HaraVar interfaceMethod = requireVar(runtime, interfaceNamespace, "probe");

        runtime.runInNamespace(
            ownerNamespace,
            () -> runtime.define(Symbol.create("probe"), "collision"));

        HaraException error =
            assertThrows(
                HaraException.class,
                () ->
                    ProtocolProductLibraryProvider.publishProducts(
                        runtime, "IProbe", protocol));
        assertTrue(error.getMessage(), error.getMessage().contains("Protocol product collision"));
        assertNull(runtime.resolve(Symbol.create(ownerNamespace, "IProbe")));
        assertEquals(
            "collision", requireVar(runtime, ownerNamespace, "probe").get());
        assertSame(
            interfaceDescriptor,
            requireVar(runtime, interfaceNamespace, "IProbe"));
        assertSame(interfaceMethod, requireVar(runtime, interfaceNamespace, "probe"));
      } finally {
        context.leave();
      }
    }
  }

  private static HaraVar requireVar(HaraContext context, String namespace, String name) {
    HaraVar variable = context.resolve(Symbol.create(namespace, name));
    assertNotNull(namespace + "/" + name, variable);
    return variable;
  }

  private static String guestError(Context context, String source) {
    PolyglotException error =
        assertThrows(
            PolyglotException.class,
            () -> context.eval(HaraLanguage.ID, source));
    return error.getMessage();
  }
}
