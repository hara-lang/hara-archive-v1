package hara.lang.base.primitive;

import hara.lang.base.Iter;
import hara.lang.data.MapEntry;
import hara.lang.protocol.ICount;
import hara.lang.protocol.ILookup;

import java.lang.ref.Reference;
import java.lang.ref.ReferenceQueue;
import java.lang.ref.WeakReference;
import java.util.Iterator;
import java.util.Map.Entry;
import java.util.concurrent.ConcurrentHashMap;
import java.util.function.Supplier;

public final class RefCache<K, V> implements ILookup<K, Reference<V>>, ICount {

  final ConcurrentHashMap<K, Reference<V>> _lu;
  final ReferenceQueue<V> _rq;

  public RefCache() {
    _lu = new ConcurrentHashMap<K, Reference<V>>();
    _rq = new ReferenceQueue<V>();
  }

  public void clearCache() {
    if (_rq.poll() != null) {
      while (_rq.poll() != null) {}

      var it = _lu.entrySet().iterator();
      Iter.filter(it, (e) -> (e.getValue() == null) || (e.getValue().get() == null))
          .forEachRemaining((e) -> _lu.remove(e.getKey()));
    }
  }

  @Override
  public long count() {
    return _lu.size();
  }

  public void deregister(K key) {
    _lu.remove(key);
  }

  @SuppressWarnings({"unchecked", "rawtypes"})
  @Override
  public Entry<K, Reference<V>> find(K key) {
    var ret = _lu.getOrDefault(key, null);
    return (ret == null) ? null : new MapEntry(null, key, ret);
  }

  public V get(K key) {
    var ref = _lu.get(key);
    return (ref != null) ? ref.get() : null;
  }

  public ConcurrentHashMap<K, Reference<V>> getLookup() {
    return _lu;
  }

  public V getOrCreate(K key, Supplier<Reference<V>> f) {
    var ref = _lu.get(key);
    if (ref != null) {
      var v = ref.get();
      if (v != null) {
        return v;
      }
    }

    ref = f.get();
    _lu.put(key, ref);
    return ref.get();
  }

  public ReferenceQueue<V> getQueue() {
    return _rq;
  }

  @Override
  public Iterator<K> keys() {
    return _lu.keys().asIterator();
  }

  @Override
  public Reference<V> lookup(K key) {
    return _lu.get(key);
  }

  public void register(K key, V obj) {
    _lu.put(key, new WeakReference<V>(obj, _rq));
  }

  @Override
  public Iterator<Reference<V>> vals() {
    return _lu.values().iterator();
  }
}
