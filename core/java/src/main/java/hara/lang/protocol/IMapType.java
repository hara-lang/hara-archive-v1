package hara.lang.protocol;

import hara.lang.base.Eq;
import hara.lang.base.G;
import hara.lang.base.Iter;
import hara.lang.declaration.HaraProtocolBinding;
import java.util.Iterator;
import java.util.Map;
import java.util.Map.Entry;
import java.util.function.BiFunction;
import java.util.function.Function;

/** Portable map-category protocol descriptor. */
@HaraProtocolBinding(
    namespace = "std.protocol.imaptype",
    name = "IMapType",
    parents = {
      "IColl", "ICount", "IObjType", "IMetadata", "ILookup", "IAssoc", "IDissoc", "IFind", "IFn"
    })
public interface IMapType<K, V>
    extends IColl<Entry<K, V>>,
        ICount,
        IObjType,
        IMetadata,
        ILookup<K, V>,
        IAssoc<K, V>,
        IDissoc<K>,
        IFind<K, Entry<K, V>>,
        IFn<V, K, V> {

  default java.util.Map<K, V> asJavaMap() {
    return null;
  }

  @Override
  default Constant.MetaType getMetatype() {
    return Constant.MetaType.MAP;
  }

  @Override
  default Constant.ObjType getObjType() {
    return Constant.ObjType.MAP;
  }

  @Override
  default IMapType<K, V> conj(Entry<K, V> entry) {
    return (IMapType<K, V>) assoc(entry.getKey(), entry.getValue());
  }

  @Override
  default V lookup(K key) {
    Entry<K, V> entry = find(key);
    return entry == null ? null : entry.getValue();
  }

  @Override
  default V lookup(K key, V notFound) {
    Entry<K, V> entry = find(key);
    return entry == null ? notFound : entry.getValue();
  }

  @Override
  default Iterator<K> keys() {
    return Iter.map(iterator(), Entry::getKey);
  }

  @Override
  default Iterator<V> vals() {
    return Iter.map(iterator(), Entry::getValue);
  }

  @Override
  default String startString() {
    return "{";
  }

  @Override
  default String endString() {
    return "}";
  }

  @Override
  default String sepString() {
    return ", ";
  }

  @Override
  default String display() {
    return Iter.toString(
        iterator(),
        startString(),
        endString(),
        sepString(),
        entry -> G.display(entry.getKey()) + " " + G.display(entry.getValue()));
  }

  @SuppressWarnings({"unchecked", "rawtypes"})
  @Override
  default boolean equality(Object obj) {
    if (obj instanceof IMapType) {
      return count() == ((IMapType) obj).count()
          && Iter.every(
              iterator(),
              entry -> {
                Map.Entry other = (Entry) ((IMapType) obj).find(entry.getKey());
                return other != null && Eq.eq(other.getValue(), entry.getValue());
              });
    }
    if (obj instanceof java.util.Map) {
      return count() == ((java.util.Map) obj).size()
          && Iter.every(
              ((java.util.Map) obj).entrySet().iterator(),
              entry -> {
                Map.Entry other = (Map.Entry) entry;
                Map.Entry current = (Map.Entry) find((K) other.getKey());
                return current != null && Eq.eq(current.getValue(), other.getValue());
              });
    }
    return false;
  }

  @Override
  default long hashCalc(Constant.HashType type) {
    Function<Object, Long> hash = G.hashFn(type);
    return Iter.reduce(
        iterator(), Long.valueOf(hashSeed().hashCode()), (acc, entry) -> acc + hash.apply(entry));
  }

  @SuppressWarnings({"unchecked", "rawtypes"})
  @Override
  default Function getArg1() {
    return key -> lookup((K) key);
  }

  @SuppressWarnings({"unchecked", "rawtypes"})
  @Override
  default BiFunction getArg2() {
    return (key, notFound) -> lookup((K) key, (V) notFound);
  }
}
