package hara.lang.data;

import hara.lang.data.types.ILinkedType;
import hara.lang.protocol.ISequential;
import hara.lang.data.types.ObjPersistent;
import hara.lang.protocol.IColl;
import hara.lang.protocol.IMetadata;

import java.util.Iterator;

public class Cons<E> extends ObjPersistent
    implements ISequential<E>, ILinkedType<E>, IColl<E> {

  private final E _first;
  private final ILinkedType<E> _more;

  public Cons(IMetadata meta, E first, ILinkedType<E> more) {
    super(meta);
    _first = first;
    _more = more;
  }

  @Override
  public final Cons<E> withMeta(IMetadata meta) {
    return (meta() == meta) ? this : new Cons<E>(meta, _first, _more);
  }

  //
  // I.Seq
  //

  @Override
  public Cons<E> cons(E e) {
    return new Cons<E>(null, e, this);
  }

  @Override
  public Cons<E> conj(E e) {
    return pushFirst(e);
  }

  @Override
  public String startString() {
    return "(";
  }

  @Override
  public String endString() {
    return ")";
  }

  @Override
  public final E peekFirst() {
    return _first;
  }

  @Override
  public final ILinkedType<E> popFirst() {
    return _more;
  }

  @Override
  public long count() {
    return 1 + ((_more == null) ? 0 : _more.count());
  }

  @Override
  public Iterator<E> iterator() {
    return new Iterator<E>() {
      ILinkedType<E> current = Cons.this;

      @Override
      public boolean hasNext() {
        return current != null;
      }

      @Override
      public E next() {
        if (!hasNext()) throw new java.util.NoSuchElementException();
        E val = current.peekFirst();
        current = (ILinkedType<E>) current.popFirst();
        return val;
      }
    };
  }

  @Override
  public Cons<E> pushFirst(E e) {
    return new Cons<E>(_meta, e, this);
  }

  @Override
  public Tuple.Tup0 empty() {
    return Tuple.Tup0.EMPTY.withMeta(_meta);
  }
}
