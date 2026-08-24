package hara.kernel.base;

import hara.kernel.builtin.BuiltinRuntime;
import hara.lang.data.Symbol;
import hara.lang.protocol.ILinearType;
import org.junit.Test;

import static org.junit.Assert.*;

public class RTTest {

  @Test
  public void testLoader() {
    RT.Loader loader = new RT.Loader();
    try {
      Class<?> stringClass = loader.loadClass("java.lang.String", false);
      assertNotNull("Should be able to load java.lang.String", stringClass);
      assertEquals("java.lang.String", stringClass.getName());
    } catch (ClassNotFoundException e) {
      fail("ClassNotFoundException for java.lang.String");
    }
  }

  @Test
  public void testInstance() {
    RT.Instance<Object> rt = new RT.Instance<>(null, "test");
    assertNotNull(rt);
  }

  @Test
  public void testEval() {
    RT.Instance<Object> rt = new RT.Instance<>(null, "test");
    Object result = rt.eval(rt.readString("(+ 1 2)"));
    assertEquals(3L, result);
  }

  @Test
  public void testVariadicStr() {
    RT.Instance<Object> rt = new RT.Instance<>(null, "test");

    assertEquals("", rt.eval(rt.readString("(str)")));
    assertEquals("1", rt.eval(rt.readString("(str 1)")));
    assertEquals("123", rt.eval(rt.readString("(str 1 2 3)")));
  }

  @Test
  public void testVariadicConj() {
    RT.Instance<Object> rt = new RT.Instance<>(null, "test");

    ILinearType<?> empty = (ILinearType<?>) rt.eval(rt.readString("(conj)"));
    ILinearType<?> unary = (ILinearType<?>) rt.eval(rt.readString("(conj 1)"));
    ILinearType<?> many = (ILinearType<?>) rt.eval(rt.readString("(conj [] 1 2 3)"));
    assertEquals(0, empty.count());
    assertEquals(1, unary.count());
    assertEquals(1L, unary.nth(0));
    assertEquals(3, many.count());
    assertEquals(3L, many.nth(2));
  }

  @Test
  public void testSetAndGetObj() {
    RT.Instance<Object> rt = new RT.Instance<>(null, "test");
    Symbol sym = Symbol.create("a");
    hara.kernel.base.Var var = new hara.kernel.base.Var(sym.getName(), 10);
    rt.setObj(sym, var);
    assertEquals(var, rt.getObj(sym));
  }

  @Test
  public void testClassEnvAlias() {
    RT.Instance<Object> rt = new RT.Instance<>(null, "test");
    rt.getCurrentNs().nativeFlavor = "jvm";
    RT.ClassEnv classEnv = rt._userEnv._class;
    Symbol sym = Symbol.create("Str");

    classEnv.addAlias(sym, String.class);
    assertEquals(String.class, classEnv.find(sym).getValue());

    classEnv.removeAlias(sym);
    assertNull(classEnv.find(sym));
  }

  @Test
  public void testUserEnvNamespaces() {
    RT.Instance<Object> rt = new RT.Instance<>(null, "test");
    Symbol nsName = Symbol.create("my.ns");
    rt.addNamespace(nsName);

    assertNotNull(rt.getNamespace(nsName));

    Symbol varSym = Symbol.create("my.ns", "a");
    hara.kernel.base.Var var = new hara.kernel.base.Var("a", 100);
    rt.setObj(varSym, var);

    assertEquals(var, rt.getObj(varSym));
  }

  @Test
  public void testInstanceClassLoading() {
    RT.Instance<Object> rt = new RT.Instance<>(null, "test");
    Class<?> cls = rt.classFor("java.util.ArrayList");
    assertNotNull(cls);
    assertEquals(java.util.ArrayList.class, cls);

    // Test implicit java.lang prefix
    Class<?> strCls = rt.classFor("String");
    assertEquals(String.class, strCls);
  }

  @Test
  public void testPathHandling() throws java.net.MalformedURLException {
    RT.Instance<Object> rt = new RT.Instance<>(null, "test");
    java.net.URL url = new java.net.URL("file:///tmp/");
    rt.pathAdd(url);

    boolean found = false;
    for (Object u : rt.pathCache()) {
      if (u.equals(url)) {
        found = true;
        break;
      }
    }
    assertTrue("URL should be in path cache", found);
  }

  @Test
  public void testLegacyParityPrimitives() {
    RT.Instance<Object> rt = new RT.Instance<>(null, "test");

    assertEquals(42L, rt.eval(rt.readString("((fn [x & xs] (+ x (count xs))) 40 1 2)")));
    assertEquals(6L, rt.eval(rt.readString("(apply (fn [a b c] (+ a b c)) 1 [2 3])")));
    assertEquals(42L, rt.eval(rt.readString("(do (defn answer [x] (+ x 1)) (answer 41))")));
    assertEquals(1L, rt.eval(rt.readString("(iter-next (iter [1 2]))")));

    ILinearType<?> array = (ILinearType<?>) rt.eval(rt.readString("(array 1 2)"));
    assertEquals(2, array.count());
    assertEquals(2L, array.nth(1));
  }

  @Test
  public void testHalResourceLoading() {
    RT.Instance<Object> rt = new RT.Instance<>(null, "test");

    assertEquals(42L, BuiltinRuntime.loadResource(rt, "hara/core-language-resource.hal"));
    assertNull(BuiltinRuntime.require(rt, "hara/core-language-resource.hal"));
    assertEquals(42L, rt.eval(rt.readString("core-language-resource-answer")));
  }
}
