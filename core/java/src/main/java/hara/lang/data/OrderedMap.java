package hara.lang.data;

import hara.lang.base.Iter;
import hara.lang.base.primitive.Array;
import hara.lang.protocol.IMapType;
import hara.lang.data.types.ObjMutable;
import hara.lang.data.types.ObjPersistent;
import hara.lang.protocol.IMetadata;
import hara.lang.protocol.INth;
import hara.lang.protocol.IToMutable;
import hara.lang.protocol.IToPersistent;

import java.util.Iterator;
import java.util.Map.Entry;

public interface OrderedMap<K, V> extends IMapType<K, V>, INth<Entry<K, V>> {

  public interface Base<K, V> extends OrderedMap<K, V> {
    public Map<K, Entry<Integer, V>> _lookup();

    public Vector<Entry<K, V>> _order();

    @Override
    default Iterator<Entry<K, V>> iterator() {
      return Iter.keep(_order().iterator(), x -> x);
    }

    @SuppressWarnings({"unchecked", "rawtypes"})
    @Override
    default Entry<K, V> find(K key) {
      var rec = _lookup().find(key);
      return (rec != null) ? new MapEntry<>(null, key, rec.getValue().getValue()) : null;
    }

    @Override
    default long count() {
      return _lookup().count();
    }

    @Override
    default Entry<K, V> nth(long i) {
      return Iter.nth(iterator(), i);
    }
  }

  @SuppressWarnings("unchecked")
  public class Mutable<K, V> extends ObjMutable implements Base<K, V>, IToPersistent {

    private Vector.Mutable<Entry<K, V>> _order;
    private Map.Mutable<K, Entry<Integer, V>> _lookup;

    public Mutable(IMetadata meta) {
      this(meta, Vector.Mutable.empty(null), Map.Mutable.empty(null));
    }

    public Mutable(
        IMetadata meta,
        Vector.Mutable<Entry<K, V>> order,
        Map.Mutable<K, Entry<Integer, V>> lookup) {
      super(meta);
      _order = order;
      _lookup = lookup;
    }

    @SuppressWarnings("rawtypes")
    public static <K, V> Mutable<K, V> from(IMetadata meta, Object... elements) {
      return into(new Mutable<K, V>(null), (Iterator) Iter.partitionPair(Array.toIter(elements)));
    }

    public static <K, V> Mutable<K, V> into(Iterator<Entry<K, V>> it) {
      return into(new Mutable<K, V>(null), it);
    }

    public static <K, V> Mutable<K, V> into(Mutable<K, V> map, Iterator<Entry<K, V>> it) {
      Iter.reduce(it, map, (m, e) -> m.assoc(e.getKey(), e.getValue()));
      return map;
    }

    @Override
    public Mutable<K, V> empty() {
      _order.empty();
      _lookup.empty();
      return this;
    }

    @SuppressWarnings("rawtypes")
    @Override
    public Mutable<K, V> assoc(K k, V v) {
      var rec = _lookup.find(k);
      if (rec == null) {
        _order.conj(new MapEntry<>(null, k, v));
        _lookup.assoc(k, new MapEntry<>(null, (int) _order.count() - 1, v));
        return this;
      } else {
        if (v == rec.getValue().getValue()) {
          return this;
        } else {
          var idx = rec.getValue().getKey();
          _order.assoc(idx, new MapEntry<>(null, k, v));
          _lookup.assoc(k, new MapEntry<>(null, idx, v));
          return this;
        }
      }
    }

    @Override
    public Mutable<K, V> dissoc(K k) {
      var rec = _lookup.find(k);
      if (rec == null) {
        return this;
      } else {
        var idx = rec.getValue().getKey();
        _order.assoc(idx, null);
        _lookup.dissoc(k);

        // Amortized compaction
        if (_order.count() > 32 && _order.count() > 2 * _lookup.count()) {
          Vector.Mutable<Entry<K, V>> newOrder = Vector.Mutable.empty(null);
          Map.Mutable<K, Entry<Integer, V>> newLookup = Map.Mutable.empty(null);

          Iterator<Entry<K, V>> it = _order.iterator();
          int newIdx = 0;
          while (it.hasNext()) {
            Entry<K, V> entry = it.next();
            if (entry != null) {
              newOrder.conj(entry);
              newLookup.assoc(entry.getKey(), new MapEntry<>(null, newIdx, entry.getValue()));
              newIdx++;
            }
          }
          _order = newOrder;
          _lookup = newLookup;
        }
        return this;
      }
    }

    @Override
    public Standard<K, V> toPersistent() {
      return new Standard<K, V>(_meta, _order.toPersistent(), _lookup.toPersistent());
    }

    @Override
    public Map<K, Entry<Integer, V>> _lookup() {
      return _lookup;
    }

    @Override
    public Vector<Entry<K, V>> _order() {
      return _order;
    }
  }

  @SuppressWarnings("unchecked")
  public class Standard<K, V> extends ObjPersistent implements Base<K, V>, IToMutable {
    private final Map.Standard<K, Entry<Integer, V>> _lookup;
    private final Vector.Standard<Entry<K, V>> _order;

    @SuppressWarnings("rawtypes")
    public static Standard EMPTY = new Standard(null);

    public Standard(IMetadata meta) {
      this(meta, Vector.Standard.empty(null), Map.Standard.EMPTY);
    }

    public Standard(
        IMetadata meta,
        Vector.Standard<Entry<K, V>> order,
        Map.Standard<K, Entry<Integer, V>> lookup) {
      super(meta);
      _order = order;
      _lookup = lookup;
    }

    public static <K, V> Standard<K, V> from(IMetadata meta, Object... elements) {
      return (Standard<K, V>) Mutable.from(meta, elements).toPersistent();
    }

    public static <K, V> Standard<K, V> into(Iterator<Entry<K, V>> it) {
      return Mutable.into(it).toPersistent();
    }

    public static <K, V> Standard<K, V> into(Standard<K, V> map, Iterator<Entry<K, V>> it) {
      return Mutable.into(map.toMutable(), it).toPersistent();
    }

    @Override
    public Standard<K, V> empty() {
      return (_meta == null) ? EMPTY : EMPTY.withMeta(_meta);
    }

    @Override
    public Standard<K, V> withMeta(IMetadata meta) {
      return new Standard<K, V>(meta, _order, _lookup);
    }

    @SuppressWarnings("rawtypes")
    @Override
    public Standard<K, V> assoc(K k, V v) {
      var rec = _lookup.find(k);
      if (rec == null) {
        var norder = (Vector.Standard) _order.conj(new MapEntry<>(null, k, v));
        var nlookup = _lookup.assoc(k, new MapEntry<>(null, (int) norder.count() - 1, v));
        return new Standard<K, V>(_meta, norder, nlookup);
      } else {
        if (v == rec.getValue().getValue()) {
          return this;
        } else {
          var idx = rec.getValue().getKey();
          var norder = _order.assoc(idx, new MapEntry<>(null, k, v));
          var nlookup = _lookup.assoc(k, new MapEntry<>(null, idx, v));
          return new Standard<K, V>(_meta, norder, nlookup);
        }
      }
    }

    @Override
    public Standard<K, V> dissoc(K k) {
      var rec = _lookup.find(k);
      if (rec == null) {
        return this;
      } else {
        var idx = rec.getValue().getKey();
        var norder = _order.assoc(idx, null);
        var nlookup = _lookup.dissoc(k);

        // Amortized compaction
        if (norder.count() > 32 && norder.count() > 2 * nlookup.count()) {
          Vector.Mutable<Entry<K, V>> newOrder = Vector.Mutable.empty(null);
          Map.Mutable<K, Entry<Integer, V>> newLookup = Map.Mutable.empty(null);

          Iterator<Entry<K, V>> it = norder.iterator();
          int newIdx = 0;
          while (it.hasNext()) {
            Entry<K, V> entry = it.next();
            if (entry != null) {
              newOrder.conj(entry);
              newLookup.assoc(entry.getKey(), new MapEntry<>(null, newIdx, entry.getValue()));
              newIdx++;
            }
          }
          return new Standard<K, V>(_meta, newOrder.toPersistent(), newLookup.toPersistent());
        }
        return new Standard<K, V>(_meta, norder, nlookup);
      }
    }

    @Override
    public Map<K, Entry<Integer, V>> _lookup() {
      return _lookup;
    }

    @Override
    public Vector<Entry<K, V>> _order() {
      return _order;
    }

    @Override
    public Mutable<K, V> toMutable() {
      return new Mutable<K, V>(_meta, _order.toMutable(), _lookup.toMutable());
    }
  }
}
