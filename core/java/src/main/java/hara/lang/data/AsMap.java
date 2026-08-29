package hara.lang.data;

import hara.lang.base.Iter;
import hara.lang.protocol.IMapType;
import hara.lang.data.types.ObjMutable;

import java.util.Iterator;
import java.util.Map.Entry;

public class AsMap<K, V> extends ObjMutable implements IMapType<K, V> {

  final java.util.Map<K, V> _m;

  public AsMap(java.util.Map<K, V> m) {
    _m = m;
  }

  @Override
  public AsMap<K, V> assoc(K k, V v) {
    _m.put(k, v);
    return this;
  }

  @Override
  public long count() {
    return _m.size();
  }

  @Override
  public AsMap<K, V> dissoc(K k) {
    _m.remove(k);
    return this;
  }

  @Override
  public AsMap<K, V> empty() {
    _m.clear();
    return this;
  }

  @Override
  public Entry<K, V> find(K key) {
    return (_m.containsKey(key)) ? new MapEntry<K, V>(null, key, _m.get(key)) : null;
  }

  @Override
  public boolean has(K key) {
    return _m.containsKey(key);
  }

  @Override
  public Iterator<Entry<K, V>> iterator() {
    return Iter.map(
        _m.entrySet().iterator(), entry -> new MapEntry<>(null, entry.getKey(), entry.getValue()));
  }

  @Override
  public V lookup(K key) {
    return _m.get(key);
  }

  @Override
  public V lookup(K key, V notFound) {
    return _m.getOrDefault(key, notFound);
  }
}
