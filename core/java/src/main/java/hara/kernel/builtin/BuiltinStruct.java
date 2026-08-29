package hara.kernel.builtin;

import hara.kernel.base.Module;
import hara.lang.base.G;
import hara.lang.base.Iter;
import hara.lang.data.*;
import hara.lang.data.Tuple.*;
import hara.lang.protocol.ILinearType;
import hara.lang.protocol.IMapType;
import hara.lang.protocol.IPair;
import hara.lang.base.Ex;

import java.util.Iterator;
import java.util.Map.Entry;

@SuppressWarnings({"unchecked", "rawtypes"})
@Module.Ns(name = "global", tag = "structure")
public interface BuiltinStruct {
  //
  // Coll
  //

  @Module.Fn(name = "hash-map", vargs = true, complete = true)
  public static <ITR, K, V> Map.Standard<K, V> hashMap(ITR elements) {
    return Map.Standard.into(Iter.partitionPair(Iter.iter(elements)));
  }

  @Module.Fn(name = "hash-set", vargs = true, complete = true)
  public static <ITR, E> Set.Standard<E> hashSet(ITR elements) {
    return Set.Standard.into(Iter.iter(elements));
  }

  @Module.Fn(name = "j:hash-map", vargs = true, complete = true)
  public static <ITR> java.util.HashMap jHashMap(ITR vargs) {
    var m = new java.util.HashMap();
    Iter.each(
        Iter.partitionPair(Iter.iter(vargs)),
        (p) -> m.put(((Entry) p).getKey(), ((Entry) p).getValue()));
    return m;
  }

  @Module.Fn(name = "j:hash-set", vargs = true, complete = true)
  public static <ITR> java.util.HashSet jHashSet(ITR vargs) {
    var s = new java.util.HashSet();
    Iter.each(Iter.iter(vargs), (e) -> s.add(e));
    return s;
  }

  //
  // Checks
  //

  @Module.Fn(name = "j:list", vargs = true, complete = true)
  public static <ITR> java.util.ArrayList jList(ITR vargs) {
    return Iter.toArrayList(Iter.iter(vargs));
  }

  @Module.Fn(name = "list", vargs = true, complete = true)
  public static <ITR, E> List.Standard<E> list(ITR elements) {
    return List.Standard.into(Iter.iter(elements));
  }

  public static <ITR, K, V> OrderedMap.Standard<K, V> orderedMap(ITR elements) {
    return OrderedMap.Standard.into(Iter.partitionPair(Iter.iter(elements)));
  }

  public static <ITR, E> OrderedSet.Standard<E> orderedSet(ITR elements) {
    return OrderedSet.Standard.into(Iter.iter(elements));
  }

  @Module.Fn(name = "pair", complete = true)
  public static <K, V> IPair<K, V> pair(K key, V val) {
    return new MapEntry<>(null, key, val);
  }

  @Module.Fn(name = "symbol", complete = true)
  public static hara.lang.data.Symbol symbol(String name) {
    return hara.lang.data.Symbol.create(name);
  }

  public static <ITR, E> Queue.Standard<E> queue(ITR elements) {
    return Queue.Standard.into(Iter.iter(elements));
  }

  public static <ITR, E> Deque.Standard<E> deque(ITR elements) {
    return Deque.Standard.into(Iter.iter(elements));
  }

  public static <ITR, K, V extends Comparable<? super V>> PriorityMap.Standard<K, V> priorityMap(
      ITR elements) {
    java.util.ArrayList<Object> values = Iter.toArrayList(Iter.iter(elements));
    return PriorityMap.Standard.from(null, values.toArray());
  }

  public static <ITR, K, V> SortedMap.Standard<K, V> sortedMap(ITR elements) {
    return SortedMap.Standard.into(Iter.partitionPair(Iter.iter(elements)));
  }

  public static <ITR, E> SortedSet.Standard<E> sortedSet(ITR elements) {
    return SortedSet.Standard.into(Iter.iter(elements));
  }

  @Module.Fn(name = "to:facade", complete = true)
  public static <E> ILinearType<E> toFacade(java.util.List<E> l) {
    return new AsList<E>(l);
  }

  @Module.Fn(name = "to:facade", complete = true)
  public static <K, V> IMapType<K, V> toFacade(java.util.Map<K, V> m) {
    return new AsMap<K, V>(m);
  }

  @Module.Fn(name = "tup", complete = true)
  public static Tup0 tup() {
    return Tup0.EMPTY;
  }

  @Module.Fn(name = "tup", complete = true)
  public static Tup1.L tup(Object a) {
    return new Tup1.L(null, a);
  }

  @Module.Fn(name = "tup", complete = true)
  public static Tup2.L tup(Object a, Object b) {
    return new Tup2.L(null, a, b);
  }

  @Module.Fn(name = "tup", complete = true)
  public static Tup3.L tup(Object a, Object b, Object c) {
    return new Tup3.L(null, a, b, c);
  }

  @Module.Fn(name = "tup", complete = true)
  public static Tup4.L tup(Object a, Object b, Object c, Object d) {
    return new Tup4.L(null, a, b, c, d);
  }

  @Module.Fn(name = "tup", complete = true)
  public static Tup5.L tup(Object a, Object b, Object c, Object d, Object e) {
    return new Tup5.L(null, a, b, c, d, e);
  }

  public static ILinearType tuple(Object[] xs) {
    switch (xs.length) {
      case 0:
        return Tup0.EMPTY;
      case 1:
        return new Tup1.L(null, xs[0]);
      case 2:
        return new Tup2.L(null, xs[0], xs[1]);
      case 3:
        return new Tup3.L(null, xs[0], xs[1], xs[2]);
      case 4:
        return new Tup4.L(null, xs[0], xs[1], xs[2], xs[3]);
      case 5:
        return new Tup5.L(null, xs[0], xs[1], xs[2], xs[3], xs[4]);
      case 6:
        return new Tup6.L(null, xs[0], xs[1], xs[2], xs[3], xs[4], xs[5]);
      case 7:
        return new Tup7.L(null, xs[0], xs[1], xs[2], xs[3], xs[4], xs[5], xs[6]);
      case 8:
        return new Tup8.L(null, xs[0], xs[1], xs[2], xs[3], xs[4], xs[5], xs[6], xs[7]);
      default:
        throw new Ex.Arity(xs.length, "");
    }
  }

  @Module.Fn(name = "array", vargs = true, complete = true)
  public static <ITR, E> Vector.Mutable<E> array(ITR elements) {
    return Vector.Mutable.into(Iter.iter(elements));
  }

  @Module.Fn(name = "vector", vargs = true, complete = true)
  public static <ITR, E> Vector.Standard<E> vector(ITR elements) {
    return Vector.Standard.into(Iter.iter(elements));
  }
}
