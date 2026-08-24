package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.assertTrue;

import hara.lang.data.Atom;
import hara.lang.data.Keyword;
import hara.lang.data.Map;
import hara.lang.data.Vector;
import org.graalvm.polyglot.Context;
import org.graalvm.polyglot.Value;
import org.junit.Test;

/** Acceptance matrix for one language-agnostic registry over guest and Java-backed values. */
public class LanguageAgnosticProtocolAcceptanceTest {
  @Test
  public void oneProtocolRegistryHandlesHaraStructAndJavaCollection() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      Value measure =
          context.eval(
              HaraLanguage.ID,
              "(do "
                  + "  (defstruct Box [size]) "
                  + "  (extend-type Box ICount (count [self] (:size self))) "
                  + "  (fn [value] (ICount/count value)))");

      assertEquals(3L, measure.execute(Vector.Standard.from(null, "a", "b", "c")).asLong());
      assertEquals(2L, measure.execute(Map.Standard.from(null, "a", 1, "b", 2)).asLong());
      assertEquals(
          4L, context.eval(HaraLanguage.ID, "(ICount/count (Box 4))").asLong());
    }
  }

  @Test
  public void adaptsFunctionStateAndMetadataProtocolsThroughTheSameBoundary() {
    HaraProtocol ifn = new HaraProtocol("IFn", java.util.Map.of("invoke", -1));
    HaraProtocolRuntime.installForTest(ifn);
    Map.Standard<String, String> map = Map.Standard.from(null, "key", "value");
    assertEquals("value", ifn.invoke("invoke", map, new Object[] {"key"}));

    HaraProtocol deref = new HaraProtocol("IDeref", java.util.Map.of("deref", 1));
    HaraProtocolRuntime.installForTest(deref);
    Atom.Basic<Long> atom = new Atom.Basic<>(41L);
    assertEquals(41L, deref.invoke("deref", atom, new Object[0]));

    HaraProtocol reset = new HaraProtocol("IReset", java.util.Map.of("reset", 2));
    HaraProtocolRuntime.installForTest(reset);
    assertEquals(42L, reset.invoke("reset", atom, new Object[] {42L}));
    assertEquals(42L, deref.invoke("deref", atom, new Object[0]));

    HaraProtocol metadata =
        new HaraProtocol("IObjType", java.util.Map.of("meta", 1, "with-meta", 2));
    HaraProtocolRuntime.installForTest(metadata);
    Keyword keyword = Keyword.create("core/value");
    assertNull(metadata.invoke("meta", keyword, new Object[0]));

  }

  @Test
  public void haraStructsCanBeExtendedForFunctionProtocols() {
    HaraType type = new HaraType("CallableBox", new String[] {"value"});
    HaraStruct value = new HaraStruct(type, new Object[] {41L});

    HaraProtocol ifn = new HaraProtocol("IFn", java.util.Map.of("invoke", -1));
    HaraProtocolRuntime.installForTest(ifn);
    ifn.extend(type, "invoke", (receiver, arguments) -> 41L + ((Number) arguments[0]).longValue());
    assertEquals(43L, ifn.invoke("invoke", value, new Object[] {2L}));

    assertTrue(value.toString().contains("CallableBox"));
  }
  @Test
  public void oneHaraAlgorithmConsumesAndProducesGuestAndJavaBackedCollections() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      Value appendAndCount =
          context.eval(
              HaraLanguage.ID,
              "(do "
                  + "  (defstruct Bucket [items]) "
                  + "  (extend-type Bucket IConj "
                  + "    (conj [self item] (Bucket (conj (:items self) item)))) "
                  + "  (extend-type Bucket ICount "
                  + "    (count [self] (count (:items self)))) "
                  + "  (fn [value] "
                  + "    (let [updated (conj value 42)] (count updated))))");

      assertEquals(3L, appendAndCount.execute(Vector.Standard.from(null, 1L, 2L)).asLong());
      Value bucket = context.eval(HaraLanguage.ID, "(Bucket [1 2])");
      assertEquals(3L, appendAndCount.execute(bucket).asLong());
    }
  }

}
