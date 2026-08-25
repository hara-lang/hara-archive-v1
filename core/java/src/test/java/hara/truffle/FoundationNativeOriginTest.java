package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertSame;
import static org.junit.Assert.assertTrue;

import hara.lang.data.Symbol;
import org.graalvm.polyglot.Context;
import org.junit.Test;

public class FoundationNativeOriginTest {
  @Test
  public void callableSymbolsRetainTheirDefiningNamespace() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      context.eval(HaraLanguage.ID, "nil");
      context.enter();
      try {
        HaraContext hara = HaraLanguage.currentContext();
        assertBuiltinOrigin(hara, "std.native.Base", "apply");
        assertBuiltinOrigin(hara, "std.native.Runtime", "gensym");
        assertBuiltinOrigin(hara, "std.foundation", "+");
      } finally {
        context.leave();
      }
    }
  }

  @Test
  public void freshContextsKeepStringOwnershipAndOriginsSeparate() {
    assertOrigins("std.foundation.string", "length", "std.native.String", "length");
  }

  @Test
  public void freshContextsKeepBytesOwnershipAndOriginsSeparate() {
    assertOrigins("std.foundation.bytes", "count", "std.native.Bytes", "count");
  }

  @Test
  public void freshContextsKeepPromiseOwnershipAndOriginsSeparate() {
    assertOrigins("std.foundation.promise", "run", "std.native.Promise", "run");
  }

  @Test
  public void freshContextsKeepCoroutineOwnershipAndOriginsSeparate() {
    assertOrigins("std.foundation.coroutine", "create", "std.native.Coroutine", "create");
  }

  @Test
  public void canonicalNativeAndProtocolAliasesShareTheirDeclarationVars() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      context.eval(HaraLanguage.ID, "nil");
      context.enter();
      try {
        HaraContext hara = HaraLanguage.currentContext();
        assertSame(
            hara.resolve(Symbol.create("std.native.Base")),
            hara.resolve(Symbol.create("std.foundation", "Base")));
        assertSame(
            hara.resolve(Symbol.create("std.protocol.icount.ICount")),
            hara.resolve(Symbol.create("std.foundation", "ICount")));
      } finally {
        context.leave();
      }
    }
  }

  private static void assertOrigins(
      String foundationNamespace,
      String foundationSymbol,
      String nativeNamespace,
      String nativeSymbol) {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      context.eval(HaraLanguage.ID, "nil");
      context.enter();
      try {
        HaraContext hara = HaraLanguage.currentContext();
        HaraVar foundation = hara.resolve(Symbol.create(foundationNamespace, foundationSymbol));
        HaraVar nativeVar = hara.resolve(Symbol.create(nativeNamespace, nativeSymbol));
        assertNotNull(foundation);
        assertNotNull(nativeVar);
        assertEquals(HaraVar.Origin.HAL_FALLBACK, foundation.origin());
        assertEquals(HaraVar.Origin.RUNTIME_PRIMITIVE, nativeVar.origin());
      } finally {
        context.leave();
      }
    }
  }

  private static void assertBuiltinOrigin(
      HaraContext context, String namespace, String symbol) {
    HaraVar variable = context.resolve(Symbol.create(namespace, symbol));
    assertNotNull(variable);
    assertTrue(variable.deref() instanceof HaraBuiltinFunction);
    assertEquals(
        namespace + "/" + symbol,
        ((HaraBuiltinFunction) variable.deref()).origin());
  }
}
