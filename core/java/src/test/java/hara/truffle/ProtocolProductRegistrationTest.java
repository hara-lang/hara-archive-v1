package hara.truffle;

import static org.junit.Assert.assertEquals;

import org.graalvm.polyglot.Context;
import org.junit.Test;

/**
 * Proves that one built-in protocol declaration emits dotted and slash products backed by the
 * same protocol descriptor and dispatch behavior.
 */
public class ProtocolProductRegistrationTest {
  @Test
  public void dottedAndSlashProductsResolveAndShareProtocolDispatch() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).allowAllAccess(true).build()) {
      assertEquals(
          "true",
          context
              .eval(
                  HaraLanguage.ID,
                  "(= std.protocol.iassoc.IAssoc std.protocol.iassoc/IAssoc)")
              .toString());
      assertEquals(
          "true",
          context
              .eval(
                  HaraLanguage.ID,
                  "(= (std.protocol.iassoc/assoc {:a 1} :b 2) "
                      + "(std.protocol.iassoc.IAssoc/assoc {:a 1} :b 2))")
              .toString());
      assertEquals(
          "{:answer 42}",
          context
              .eval(
                  HaraLanguage.ID,
                  "(std.protocol.iassoc.IAssoc/assoc {} :answer 42)")
              .toString());
    }
  }
}
