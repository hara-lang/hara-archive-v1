package hara.kernel.builtin;

import hara.kernel.base.Module;
import hara.lang.base.Ex;
import hara.lang.base.Iter;
import hara.lang.data.*;
import hara.lang.data.types.IVectorType;
import hara.lang.protocol.*;

import java.util.Iterator;
import java.util.Map.Entry;

import static hara.kernel.base.Module.ReduceInit.*;
import static hara.kernel.base.Module.ReduceType.*;

@SuppressWarnings({"unchecked", "rawtypes"})
@Module.Ns(name = "global", tag = "collection")
public interface BuiltinCollection {
  @Module.Fn(name = "assoc", protocol = true)
  public static <K, V> IAssoc assoc(IAssoc coll, K key, V val) {
    if (coll instanceof IVectorType && !(key instanceof Integer)) {
      return ((IAssoc<Integer, V>) coll).assoc(assocIndex(key), val);
    }
    return coll.assoc(key, val);
  }

  static Integer assocIndex(Object key) {
    if (!(key instanceof Number)) {
      throw new Ex.Runtime(
          "assoc index must be a number, got: "
              + (key instanceof IDisplay ? ((IDisplay) key).display() : String.valueOf(key)));
    }
    if (key instanceof Double || key instanceof Float) {
      double value = ((Number) key).doubleValue();
      if (value != Math.rint(value)) {
        throw new Ex.Runtime("assoc index must be an integer, got: " + key);
      }
    }
    long index = ((Number) key).longValue();
    if (index < Integer.MIN_VALUE || index > Integer.MAX_VALUE) {
      throw new Ex.Runtime("assoc index out of range: " + index);
    }
    return (int) index;
  }

  @Module.Fn(name = "assoc", option = true)
  public static <K, V> java.util.Map<K, V> assoc(java.util.Map<K, V> coll, K key, V val) {
    coll.put(key, val);
    return coll;
  }

  @Module.Fn(name = "conj", protocol = true)
  @Module.Reduce(type = ARRAY, init = EMPTY_VECTOR)
  public static <E> IConj conj(IConj coll, E e) {
    return coll.conj(e);
  }

  @Module.Fn(name = "conj", option = true)
  public static <E> java.util.List<E> conj(java.util.List<E> coll, E e) {
    coll.add(e);
    return coll;
  }

  @Module.Fn(name = "conj", option = true)
  public static <K, V> java.util.Map<K, V> conj(java.util.Map<K, V> coll, Entry<K, V> e) {
    coll.put(e.getKey(), e.getValue());
    return coll;
  }

  @Module.Fn(name = "conj", option = true)
  public static <E> java.util.Set<E> conj(java.util.Set<E> coll, E e) {
    coll.add(e);
    return coll;
  }

  @Module.Fn(name = "cons", protocol = true)
  @Module.Reduce(type = ARRAY, init = EMPTY_LIST)
  public static <E> ICons cons(ICons coll, E e) {
    return coll.cons(e);
  }

  @Module.Fn(name = "cons", option = true)
  public static <E> java.util.List<E> cons(java.util.List<E> coll, E e) {
    coll.add(0, e);
    return coll;
  }

  @Module.Fn(name = "cons", protocol = true)
  public static <E> ICons cons(E e, ICons coll) {
    return coll.cons(e);
  }

  @Module.Fn(name = "cons", option = true)
  public static <E> java.util.List<E> cons(E e, java.util.List<E> coll) {
    coll.add(0, e);
    return coll;
  }

  @Module.Fn(name = "count", option = true)
  public static <E> long count(E[] e) {
    return e.length;
  }

  @Module.Fn(name = "count", protocol = true)
  public static long count(ICount coll) {
    return coll.count();
  }

  @Module.Fn(name = "count", option = true)
  public static long count(String s) {
    return s.length();
  }

  @Module.Fn(name = "count", option = true)
  public static long count(Iterable coll) {
    return Iter.count(Iter.iter(coll));
  }

  @Module.Fn(name = "count", option = true)
  public static long count(java.util.Collection coll) {
    return coll.size();
  }

  @Module.Fn(name = "dissoc", protocol = true)
  @Module.Reduce(type = SELF, init = NIL)
  public static <K, V> IDissoc<K> dissoc(IDissoc coll, K key) {
    return coll.dissoc(key);
  }

  @Module.Fn(name = "dissoc", option = true)
  public static <K, V> java.util.Map<K, V> dissoc(java.util.Map coll, K key) {
    coll.remove(key);
    return coll;
  }

  @Module.Fn(name = "dissoc", option = true)
  public static <K, V> java.util.Set<K> dissoc(java.util.Set<K> coll, K key) {
    coll.remove(key);
    return coll;
  }

  @Module.Fn(name = "empty", protocol = true)
  public static IEmpty empty(IEmpty coll) {
    return coll.empty();
  }

  @Module.Fn(name = "empty", option = true)
  public static <E> java.util.Collection<E> empty(java.util.Collection<E> coll) {
    coll.clear();
    return coll;
  }

  @Module.Fn(name = "find", protocol = true)
  public static <K, V> V find(IFind lu, K key) {
    return (V) lu.find(key);
  }

  @Module.Fn(name = "find", option = true)
  public static <K, V> Entry<K, V> find(java.util.Map lu, K key) {
    return lu.containsKey(key) ? BuiltinStruct.pair(key, (V) lu.get(key)) : null;
  }

  @Module.Fn(name = "find", option = true)
  public static <K, V> V find(java.util.Set lu, V key) {
    return lu.contains(key) ? key : null;
  }

  @Module.Fn(name = "get", protocol = true, method = "lookup")
  public static <K, V> V get(ILookup<K, V> lu, K key) {
    return lu.lookup(key);
  }

  @Module.Fn(name = "get", protocol = true, method = "lookup")
  public static <K, V> V get(ILookup<K, V> lu, K key, V notFound) {
    return lu.lookup(key, notFound);
  }

  @Module.Fn(name = "get", option = true)
  public static <K, V> V get(java.util.Map<K, V> lu, K key) {
    return lu.get(key);
  }

  @Module.Fn(name = "get", option = true)
  public static <K, V> V get(java.util.Map<K, V> lu, K key, V notFound) {
    return lu.getOrDefault(key, notFound);
  }

  @Module.Fn(name = "get", option = true)
  public static <E> E get(java.util.Set<E> lu, E key) {
    return lu.contains(key) ? key : null;
  }

  @Module.Fn(name = "get", option = true)
  public static <E> E get(java.util.Set<E> lu, E key, E notFound) {
    return lu.contains(key) ? key : notFound;
  }

  @Module.Fn(name = "has?", protocol = true, method = "has")
  public static <K> boolean has(IFind lu, K key) {
    return lu.has(key);
  }

  @Module.Fn(name = "has?", option = true)
  public static <K> boolean has(java.util.Map lu, K key) {
    return lu.containsKey(key);
  }

  @Module.Fn(name = "has?", option = true)
  public static <K> boolean has(java.util.Set lu, K key) {
    return lu.contains(key);
  }

  @Module.Fn(name = "index:of", protocol = true, method = "indexOf")
  public static <K, V> K indexOf(IIndexed<K, V> lu, V val) {
    return lu.indexOf(val);
  }

  @Module.Fn(name = "index:key", protocol = true, method = "indexOfKey")
  public static <K, V> long indexOfKey(IIndexedKV<K, V> lu, K key) {
    return lu.indexOfKey(key);
  }

  @Module.Fn(name = "index:val", protocol = true, method = "indexOfVal")
  public static <K, V> long indexOfVal(IIndexedKV<K, V> lu, V val) {
    return lu.indexOfVal(val);
  }

  @Module.Fn(name = "into", protocol = true)
  public static <ITR> IConj into(IConj coll, ITR source) {
    return Iter.reduceIn(Iter.iter(source), coll, BuiltinCollection::conj);
  }

  @Module.Fn(name = "into", option = true)
  public static <E, ITR> java.util.List<E> into(java.util.List coll, ITR source) {
    return Iter.reduceIn(Iter.iter(source), coll, BuiltinCollection::conj);
  }

  @Module.Fn(name = "into", option = true)
  public static <K, V, ITR> java.util.Map<K, V> into(java.util.Map coll, ITR source) {
    return Iter.reduceIn((Iterator<Entry>) Iter.iter(source), coll, BuiltinCollection::conj);
  }

  @Module.Fn(name = "into", option = true)
  public static <E, ITR> java.util.Set<E> into(java.util.Set coll, ITR source) {
    return Iter.reduceIn(Iter.iter(source), coll, BuiltinCollection::conj);
  }

  @Module.Fn(name = "iter", complete = true)
  public static <ITR> Iterator iter(ITR obj) {
    return Iter.iter(obj);
  }

  @Module.Fn(name = "iter-next?", complete = true)
  public static boolean iterHas(Iterator iterator) {
    return iterator.hasNext();
  }

  @Module.Fn(name = "iter-next", complete = true)
  public static Object iterNext(Iterator iterator) {
    if (!iterator.hasNext()) {
      throw new Ex.Runtime("iter-next reached the end of the iterator");
    }
    return iterator.next();
  }

  @Module.Fn(name = "iter-close", complete = true)
  public static Object iterClose(Iterator iterator) {
    if (iterator instanceof AutoCloseable) {
      try {
        ((AutoCloseable) iterator).close();
      } catch (Exception error) {
        throw Ex.Sneaky(error);
      }
    }
    return null;
  }

  @Module.Fn(name = "keys", protocol = true)
  public static <K, V> Iterator<K> keys(ILookup<K, V> m) {
    return m.keys();
  }

  @Module.Fn(name = "keys", option = true)
  public static <K, V> Iterator<K> keys(java.util.Map<K, V> m) {
    return m.keySet().iterator();
  }

  @Module.Fn(name = "merge", protocol = true)
  @Module.Reduce(type = SELF, init = EMPTY_MAP)
  public static <ITR> IAssoc merge(IAssoc coll, ITR other) {
    Iterator<Entry> it = Iter.iter(other);
    return Iter.reduce(
        it,
        coll,
        (m, e) -> {
          return m.assoc(e.getKey(), e.getValue());
        });
  }

  @Module.Fn(name = "merge", option = true)
  public static <K, V, ITR> java.util.Map<K, V> merge(java.util.Map coll, ITR other) {
    Iterator<Entry> it = Iter.iter(other);
    return Iter.reduce(
        it,
        coll,
        (m, e) -> {
          m.put(e.getKey(), e.getValue());
          return m;
        });
  }

  @Module.Fn(name = "nth", option = true)
  public static <E> E nth(E[] coll, long idx) {
    return coll[((int) idx)];
  }

  @Module.Fn(name = "nth", protocol = true)
  public static <E> E nth(INth<E> coll, long idx) {
    return coll.nth(idx);
  }

  @Module.Fn(name = "nth", option = true)
  public static <E> E nth(Iterable<E> coll, long idx) {
    return (E) Iter.nth(Iter.iter(coll), idx);
  }

  @Module.Fn(name = "nth", option = true)
  public static <E> E nth(java.util.List<E> coll, long idx) {
    return coll.get((int) idx);
  }

  @Module.Fn(name = "nth", option = true)
  public static hara.lang.data.HaraCharacter nth(String s, long idx) {
    int index = Math.toIntExact(idx);
    return hara.lang.data.HaraCharacter.of(
        s.codePointAt(s.offsetByCodePoints(0, index)));
  }

  @Module.Fn(name = "range", complete = true)
  public static Iterator<Long> range(Long max) {
    return Iter.range(max);
  }

  @Module.Fn(name = "range", complete = true)
  public static Iterator<Long> range(Long min, Long max) {
    return Iter.range(min, max);
  }

  @Module.Fn(name = "to:list", complete = true)
  public static <ITR> List.Standard toList(ITR source) {
    return List.Standard.into(
        Iter.flattenIterables(Iter.map(Iter.iter(source), (e) -> BuiltinStruct.list(e))));
  }

  @Module.Fn(name = "to:map", complete = true)
  public static <ITR> Map.Standard toMap(ITR source) {
    return Map.Standard.into(Iter.iter(source));
  }

  @Module.Fn(name = "to:mutable", complete = true)
  public static IMutable toMutable(IToMutable coll) {
    return coll.toMutable();
  }

  @Module.Fn(name = "to:persistent", complete = true)
  public static IPersistent toPersistent(IToPersistent coll) {
    return coll.toPersistent();
  }

  @Module.Fn(name = "to:seq", complete = true)
  public static <ITR> Seq toSeq(ITR source) {
    return Seq.create(Iter.iter(source));
  }

  @Module.Fn(name = "to:set", complete = true)
  public static <ITR> Set.Standard toSet(ITR source) {
    return Set.Standard.into(Iter.iter(source));
  }

  @Module.Fn(name = "to:vec", complete = true)
  public static <ITR> Vector.Standard toVec(ITR source) {
    return Vector.Standard.into(Iter.iter(source));
  }

  @Module.Fn(name = "vals", protocol = true)
  public static <K, V> Iterator<V> vals(ILookup<K, V> m) {
    return m.vals();
  }

  @Module.Fn(name = "vals", option = true)
  public static <K, V> Iterator<V> vals(java.util.Map<K, V> m) {
    return m.values().iterator();
  }

  @Module.Fn(name = "zip", vargs = true, complete = true)
  public static <ITR> Iterator zip(ITR elements) {
    return Iter.zip(Iter.iter(elements));
  }

  @Module.Fn(name = "zipmap", complete = true)
  public static <ITR> Map.Standard zipmap(ITR keys, ITR vals) {
    return toMap(Iter.zipPair(Iter.iter(keys), Iter.iter(vals)));
  }

  @Module.Fn(name = "concat", complete = true, vargs = true)
  public static Iterator concat(Object args) {
    return Iter.concat(Iter.map(Iter.iter(args), Iter::iter));
  }
}
