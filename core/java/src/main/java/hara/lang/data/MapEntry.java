package hara.lang.data;

import hara.lang.base.Ex;
import hara.lang.base.Eq;
import hara.lang.base.G;
import hara.lang.base.Iter;
import hara.lang.data.types.ObjPersistent;
import hara.lang.protocol.Constant;
import hara.lang.protocol.ICount;
import hara.lang.protocol.IEquality;
import hara.lang.protocol.IMetadata;
import hara.lang.protocol.INth;
import hara.lang.protocol.IPair;
import java.util.Iterator;

/** The immutable, pair-only representation of one map entry. */
public final class MapEntry<K, V> extends ObjPersistent
    implements IPair<K, V>, ICount, INth<Object>, IEquality, Iterable<Object> {
  private final K key;
  private final V value;

  public MapEntry(IMetadata meta, K key, V value) {
    super(meta);
    this.key = key;
    this.value = value;
  }

  @Override
  public K getKey() {
    return key;
  }

  @Override
  public V getValue() {
    return value;
  }

  @Override
  public long count() {
    return 2;
  }

  @Override
  public Object nth(long index) {
    if (index == 0) return key;
    if (index == 1) return value;
    throw new Ex.NoSuchElement();
  }

  @Override
  public Iterator<Object> iterator() {
    return Iter.objects(key, value);
  }

  @Override
  public MapEntry<K, V> withMeta(IMetadata meta) {
    return _meta == meta ? this : new MapEntry<>(meta, key, value);
  }

  @Override
  public Constant.ObjType getObjType() {
    return Constant.ObjType.MAP_ENTRY;
  }

  /** Map entries retain the ordered two-value hash used by sequential tuples. */
  @Override
  public String hashSeed() {
    return "::SEQUENTIAL";
  }

  @Override
  public long hashCalc(Constant.HashType type) {
    return Iter.reduce(
        iterator(),
        Long.valueOf(hashSeed().hashCode()),
        (acc, item) -> (acc * 31) + G.hashFn(type).apply(item));
  }

  @Override
  public boolean equality(Object other) {
    return other instanceof MapEntry<?, ?> entry
        && Eq.eq(key, entry.getKey())
        && Eq.eq(value, entry.getValue());
  }

  @Override
  public boolean equals(Object other) {
    return equality(other);
  }

  @Override
  public int hashCode() {
    return Long.hashCode(hashCalc(Constant.HashType.RAPID));
  }

  @Override
  public String display() {
    return "[" + G.display(key) + " " + G.display(value) + "]";
  }
}
