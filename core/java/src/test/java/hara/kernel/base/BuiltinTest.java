package hara.kernel.base;

import hara.kernel.builtin.BuiltinBasic;
import hara.kernel.builtin.BuiltinCheck;
import hara.lang.data.Atom;
import hara.kernel.builtin.BuiltinBasic;
import hara.kernel.builtin.BuiltinCheck;
import java.math.BigInteger;
import org.junit.Test;

import static org.junit.Assert.*;

public class BuiltinTest {

  @Test
  public void testBasicAtom() {
    Atom.Standard<Integer> atom = BuiltinBasic.atom(10);
    assertEquals(Integer.valueOf(10), atom.deref());
  }

  @Test
  public void testBasicCompare() {
    assertEquals(0, BuiltinBasic.compare(1, 1));
    assertEquals(1, BuiltinBasic.compare(2, 1));
    assertEquals(-1, BuiltinBasic.compare(1, 2));
  }

  @Test
  public void testCheckIsClass() {
    assertTrue(BuiltinCheck.isClass(String.class));
    assertFalse(BuiltinCheck.isClass("hello"));
  }

  @Test
  public void testCheckIsFalsey() {
    assertTrue(BuiltinCheck.isFalsey(null));
    assertTrue(BuiltinCheck.isFalsey(false));
    assertFalse(BuiltinCheck.isFalsey(true));
    assertFalse(BuiltinCheck.isFalsey(0));
  }

  @Test
  public void testCheckNumericTypes() {
    assertTrue(BuiltinCheck.isLong(10));
    assertTrue(BuiltinCheck.isLong(10L));
    assertTrue(BuiltinCheck.isLong(BigInteger.valueOf(10)));
    assertFalse(BuiltinCheck.isLong(BigInteger.ONE.shiftLeft(63)));
    assertFalse(BuiltinCheck.isLong(10.0));
    assertTrue(BuiltinCheck.isDouble(10.0f));
    assertTrue(BuiltinCheck.isDouble(10.0));
    assertFalse(BuiltinCheck.isDouble(10L));
  }

  @Test
  public void testCheckIsTruthy() {
    assertFalse(BuiltinCheck.isTruthy(null));
    assertFalse(BuiltinCheck.isTruthy(false));
    assertTrue(BuiltinCheck.isTruthy(true));
    assertTrue(BuiltinCheck.isTruthy(0));
  }
}
