package hara.kernel.builtin;

import hara.kernel.base.Module;
import hara.lang.base.*;
import hara.lang.base.primitive.Num;
import hara.lang.data.*;
import hara.lang.protocol.*;

import java.util.concurrent.ExecutionException;
import java.util.concurrent.TimeoutException;

@SuppressWarnings({"unchecked", "rawtypes"})
@Module.Ns(name = "global", tag = "basic")
public interface BuiltinBasic {

  @Module.Fn(name = "atom", complete = true, doc = "Creates a standard atom with the given value.")
  public static <V> Atom.Standard<V> atom(V val) {
    return new Atom.Standard<V>(val);
  }

  @Module.Fn(name = "volatile", complete = true)
  public static <V> hara.lang.base.primitive.Volatile<V> atomVolatile(V val) {
    return new hara.lang.base.primitive.Volatile<V>(val);
  }

  @Module.Fn(name = "compare", complete = true)
  public static int compare(Object k1, Object k2) {
    if (k1 == k2) return 0;
    if (k1 != null) {
      if (k2 == null) return 1;
      if (k1 instanceof Number) return Num.compare((Number) k1, (Number) k2);
      return ((Comparable<Object>) k1).compareTo(k2);
    }
    return -1;
  }

  @Module.Fn(name = "counter", complete = true)
  public static hara.lang.base.primitive.Counter counter() {
    return new hara.lang.base.primitive.Counter(-1);
  }

  @Module.Fn(name = "counter", complete = true)
  public static hara.lang.base.primitive.Counter counter(Integer start) {
    return new hara.lang.base.primitive.Counter(start);
  }

  @Module.Fn(name = "deref", protocol = true, doc = "Dereferences the given reference object.")
  public static <V> V deref(IDeref<V> ref) {
    return ref.deref();
  }

  @Module.Fn(name = "deref", protocol = true, method = "derefTimeout")
  public static <V> V deref(IDerefTimeout<V> ref, long ms, V timeoutVal) {
    return ref.derefTimeout(ms, timeoutVal);
  }

  @Module.Fn(name = "deref", option = true)
  public static <V> V deref(java.util.concurrent.Future<V> ref) {
    try {
      return ref.get();
    } catch (InterruptedException | ExecutionException t) {
      throw Ex.Sneaky(t);
    }
  }

  @Module.Fn(name = "deref", option = true)
  public static <V> V deref(java.util.concurrent.Future<V> ref, long ms, V timeoutVal) {
    try {
      return ref.get(ms, java.util.concurrent.TimeUnit.MILLISECONDS);
    } catch (InterruptedException | ExecutionException t) {
      throw Ex.Sneaky(t);
    } catch (TimeoutException e) {
      return timeoutVal;
    }
  }

  @Module.Fn(name = "flag", complete = true)
  public static hara.lang.base.primitive.Flag flag() {
    return new hara.lang.base.primitive.Flag(false);
  }

  @Module.Fn(name = "flag", complete = true)
  public static hara.lang.base.primitive.Flag flag(Boolean val) {
    return new hara.lang.base.primitive.Flag(val);
  }

  @Module.Fn(name = "hash", protocol = true)
  public static long hash(IHash obj) {
    return obj.hashGet();
  }

  @Module.Fn(name = "hash", fallback = true)
  public static long hash(Object obj) {
    return obj.hashCode();
  }

  @Module.Fn(name = "keyword", complete = true)
  public static Keyword keyword(String nsname) {
    return Keyword.create(nsname);
  }

  @Module.Fn(name = "keyword", complete = true)
  public static Keyword keyword(String ns, String name) {
    return Keyword.create(ns, name);
  }

  //
  //
  //

  @Module.Fn(name = "meta", protocol = true)
  public static IMetadata meta(IObjType obj) {
    return obj.meta();
  }

  @Module.Fn(name = "meta", fallback = true)
  public static IMetadata meta(Object obj) {
    return null;
  }

  @Module.Fn(name = "realize", protocol = true)
  public static <V> V realize(IRealize<V> obj) {
    return obj.realize();
  }

  @Module.Fn(name = "realized?", protocol = true)
  public static <V> boolean realized(IRealize<V> obj) {
    return obj.isRealized();
  }

  @Module.Fn(
      name = "reset!",
      protocol = true,
      doc =
          "Sets the value of atom to new value without regard for the current value. Returns the new value.")
  public static <V> V reset(IReset<V> obj, V val) {
    return obj.reset(val);
  }

  @Module.Fn(name = "symbol", complete = true)
  public static Symbol symbol(String nsname) {
    return Symbol.create(nsname);
  }

  @Module.Fn(name = "symbol", complete = true)
  public static Symbol symbol(String ns, String name) {
    return Symbol.create(ns, name);
  }

  @Module.Fn(name = "type", complete = true)
  public static Class<? extends Object> type(Object x) {
    return (x != null) ? x.getClass() : null;
  }

  @Module.Fn(name = "with-meta", protocol = true)
  public static IObjType withMeta(IObjType obj, IMetadata meta) {
    return obj.withMeta(meta);
  }
}
