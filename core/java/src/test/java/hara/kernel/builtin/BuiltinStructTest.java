package hara.kernel.builtin;

import hara.lang.data.Tuple;
import hara.lang.protocol.ILinearType;
import org.junit.Test;
import static org.junit.Assert.*;
import java.util.Arrays;

public class BuiltinStructTest {

  @Test
  public void testStruct() {
    // Struct is abstract, usually tested via implementations like Atom.Standard or
    // Tuple
    // But we can test Tuple creation via BuiltinStruct if exposed
  }

  @Test
  public void testTuples() {
    ILinearType t1 = BuiltinStruct.tuple(new Object[] {"a"});
    assertNotNull(t1);
    assertEquals("a", t1.nth(0));

    ILinearType t2 = BuiltinStruct.tuple(new Object[] {"a"});
    assertEquals(t1.toString(), t2.toString());
  }
}
