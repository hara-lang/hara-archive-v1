package hara.lang.data;

import hara.lang.base.Ex;
import hara.lang.base.Iter;
import hara.lang.base.primitive.Array;
import hara.lang.protocol.ISetType;
import hara.lang.data.types.ObjMutable;
import hara.lang.data.types.ObjPersistent;
import hara.lang.protocol.IMetadata;
import hara.lang.protocol.INth;
import hara.lang.protocol.IToMutable;
import hara.lang.protocol.IToPersistent;

import java.util.Iterator;

public interface OrderedSet<E> extends ISetType<E>, INth<E> {

  public interface Base<E> extends OrderedSet<E> {
    public Map<E, Integer> _lookup();

    public Vector<E> _order();

    @Override
    default Iterator<E> iterator() {
      return Iter.keep(_order().iterator(), x -> x);
    }

    @Override
    default E find(E e) {
      var rec = _lookup().find(e);
      return (rec != null) ? e : null;
    }

    @Override
    default long count() {
      return _lookup().count();
    }

    @Override
    default E nth(long i) {
      return Iter.nth(iterator(), i);
    }
  }

  public class Mutable<E> extends ObjMutable implements Base<E>, IToPersistent {

    private Vector.Mutable<E> _order;

    private Map.Mutable<E, Integer> _lookup;

    @SuppressWarnings("unchecked")
    public Mutable(IMetadata meta) {
      this(meta, Vector.Mutable.empty(null), Map.Mutable.empty(null));
    }

    @Override
    public Mutable<E> empty() {
      _order.empty();
      _lookup.empty();
      return this;
    }

    public Mutable(IMetadata meta, Vector.Mutable<E> order, Map.Mutable<E, Integer> lookup) {
      super(meta);
      _order = order;
      _lookup = lookup;
    }

    @SuppressWarnings("unchecked")
    public static <E> Mutable<E> from(IMetadata meta, E... objs) {
      Mutable<E> mut = new Mutable<E>(meta);
      return Array.reduce((arr, e) -> arr.conj(e), mut, objs);
    }

    public static <E> Mutable<E> into(Iterator<E> it) {
      return into(new Mutable<E>(null), it);
    }

    public static <E> Mutable<E> into(Mutable<E> coll, Iterator<E> it) {
      return Iter.reduce(it, coll, (m, e) -> m.conj(e));
    }

    @Override
    public Mutable<E> conj(E e) {
      if (e == null) {
        throw new Ex.Unsupported();
      }
      var rec = _lookup.find(e);
      if (rec == null) {
        _order.conj(e);
        _lookup.assoc(e, (int) _order.count() - 1);
        return this;
      } else {
        return this;
      }
    }

    @Override
    public Mutable<E> dissoc(E e) {
      var rec = _lookup.find(e);
      if (rec == null) {
        return this;
      } else {
        var idx = rec.getValue();
        _order.assoc(idx, null);
        _lookup.dissoc(e);

        // Amortized compaction
        if (_order.count() > 32 && _order.count() > 2 * _lookup.count()) {
          Vector.Mutable<E> newOrder = Vector.Mutable.empty(null);
          Map.Mutable<E, Integer> newLookup = Map.Mutable.empty(null);

          Iterator<E> it = _order.iterator();
          int newIdx = 0;
          while (it.hasNext()) {
            E entry = it.next();
            if (entry != null) {
              newOrder.conj(entry);
              newLookup.assoc(entry, newIdx);
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
    public Standard<E> toPersistent() {
      return new Standard<E>(_meta, _order.toPersistent(), _lookup.toPersistent());
    }

    @Override
    public Map<E, Integer> _lookup() {
      return _lookup;
    }

    @Override
    public Vector<E> _order() {
      return _order;
    }
  }

  public class Standard<E> extends ObjPersistent implements Base<E>, IToMutable {

    private final Map.Standard<E, Integer> _lookup;
    private final Vector.Standard<E> _order;

    @SuppressWarnings("rawtypes")
    public static Standard EMPTY = new Standard(null);

    @SuppressWarnings("unchecked")
    public Standard(IMetadata meta) {
      this(meta, Vector.Standard.empty(null), Map.Standard.EMPTY);
    }

    public Standard(IMetadata meta, Vector.Standard<E> order, Map.Standard<E, Integer> lookup) {
      super(meta);
      _order = order;
      _lookup = lookup;
    }

    @SuppressWarnings("unchecked")
    public static <E> Standard<E> from(IMetadata meta, E... elements) {
      return Mutable.from(meta, elements).toPersistent();
    }

    public static <E> Standard<E> into(Iterator<E> it) {
      return Mutable.into(it).toPersistent();
    }

    public static <E> Standard<E> into(Standard<E> coll, Iterator<E> it) {
      return Mutable.into(coll.toMutable(), it).toPersistent();
    }

    @SuppressWarnings("unchecked")
    @Override
    public Standard<E> empty() {
      return (_meta == null) ? EMPTY : EMPTY.withMeta(_meta);
    }

    @Override
    public Standard<E> withMeta(IMetadata meta) {
      return new Standard<E>(_meta, _order, _lookup);
    }

    @SuppressWarnings({"rawtypes", "unchecked"})
    @Override
    public Standard<E> conj(E e) {
      if (e == null) {
        throw new Ex.Unsupported();
      }
      var rec = _lookup.find(e);
      if (rec == null) {
        var norder = (Vector.Standard) _order.conj(e);
        var nlookup = _lookup.assoc(e, (int) norder.count() - 1);
        return new Standard<E>(_meta, norder, nlookup);
      } else {
        return this;
      }
    }

    @Override
    public Standard<E> dissoc(E e) {
      var rec = _lookup.find(e);
      if (rec == null) {
        return this;
      } else {
        var idx = rec.getValue();
        var norder = _order.assoc((int) idx, null);
        var nlookup = _lookup.dissoc(e);

        // Amortized compaction
        if (norder.count() > 32 && norder.count() > 2 * nlookup.count()) {
          Vector.Mutable<E> newOrder = Vector.Mutable.empty(null);
          Map.Mutable<E, Integer> newLookup = Map.Mutable.empty(null);

          Iterator<E> it = norder.iterator();
          int newIdx = 0;
          while (it.hasNext()) {
            E entry = it.next();
            if (entry != null) {
              newOrder.conj(entry);
              newLookup.assoc(entry, newIdx);
              newIdx++;
            }
          }
          return new Standard<E>(_meta, newOrder.toPersistent(), newLookup.toPersistent());
        }
        return new Standard<E>(_meta, norder, nlookup);
      }
    }

    @Override
    public Map<E, Integer> _lookup() {
      return _lookup;
    }

    @Override
    public Vector<E> _order() {
      return _order;
    }

    @Override
    public Mutable<E> toMutable() {
      return new Mutable<E>(_meta, _order.toMutable(), _lookup.toMutable());
    }
  }
}
