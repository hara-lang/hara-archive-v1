package hara.lang.data;

import hara.lang.base.Iter;
import hara.lang.base.primitive.Array;
import hara.lang.protocol.ISetType;
import hara.lang.data.types.ObjMutable;
import hara.lang.data.types.ObjPersistent;
import hara.lang.protocol.IMetadata;
import hara.lang.protocol.IToMutable;
import hara.lang.protocol.IToPersistent;

import java.util.Iterator;

public interface SortedSet<E> extends ISetType<E> {

  public interface Base<E> extends SortedSet<E> {

    public SortedMap<E, E> _lookup();

    @Override
    default Iterator<E> iterator() {
      return _lookup().keys();
    }

    @Override
    default long count() {
      return _lookup().count();
    }

    @Override
    default E find(E key) {
      var rec = _lookup().find(key);
      return (rec == null) ? null : rec.getValue();
    }
  }

  public class Mutable<E> extends ObjMutable implements Base<E>, IToPersistent {

    private SortedMap.Mutable<E, E> _lookup;

    public Mutable(IMetadata meta) {
      super(meta);
      _lookup = new SortedMap.Mutable<E, E>(meta);
    }

    public Mutable(IMetadata meta, SortedMap.Mutable<E, E> lookup) {
      super(meta);
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
      _lookup.assoc(e, e);
      return this;
    }

    @Override
    public Mutable<E> empty() {
      _lookup.empty();
      return this;
    }

    @Override
    public Mutable<E> dissoc(E k) {
      _lookup.dissoc(k);
      return this;
    }

    @Override
    public Standard<E> toPersistent() {
      return new Standard<E>(_meta, _lookup.toPersistent());
    }

    @Override
    public SortedMap<E, E> _lookup() {
      return _lookup;
    }
  }

  public class Standard<E> extends ObjPersistent implements Base<E>, IToMutable {

    private final SortedMap.Standard<E, E> _lookup;

    @SuppressWarnings("rawtypes")
    public static Standard EMPTY = new Standard(null);

    @SuppressWarnings("unchecked")
    public Standard(IMetadata meta) {
      super(meta);
      _lookup = SortedMap.Standard.EMPTY;
    }

    public Standard(IMetadata meta, SortedMap.Standard<E, E> lookup) {
      super(meta);
      _lookup = lookup;
    }

    @Override
    public Standard<E> conj(E e) {
      SortedMap.Standard<E, E> newLookup = _lookup.assoc(e, e);
      if (newLookup == _lookup) {
        return this;
      }
      return new Standard<E>(_meta, newLookup);
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
      return (_meta == meta) ? this : new Standard<E>(meta, _lookup);
    }

    @Override
    public Standard<E> dissoc(E k) {
      SortedMap.Standard<E, E> nlookup = _lookup.dissoc(k);
      return (nlookup == _lookup) ? this : new Standard<E>(_meta, nlookup);
    }

    @Override
    public Mutable<E> toMutable() {
      return new Mutable<E>(_meta, _lookup.toMutable());
    }

    @Override
    public SortedMap<E, E> _lookup() {
      return _lookup;
    }
  }
}
