package hara.lang.data;

import hara.lang.data.types.ILinkedType;
import hara.lang.protocol.ISequential;
import hara.lang.data.types.ObjPersistent;
import hara.lang.base.G;
import hara.lang.protocol.IMetadata;

import java.util.Iterator;
import java.util.NoSuchElementException;

public class Seq<E> extends ObjPersistent
    implements ISequential<E>, ILinkedType<E> {

  public static final int DISPLAY_LIMIT = 10;

  final Iterator<E> _iter;
  final State<E> _state;

  static class State<V> {
    volatile V _val;
    volatile V _rest;
  }

  @SuppressWarnings("unchecked")
  public Seq(Iterator<E> iter) {
    if (!iter.hasNext()) throw new NoSuchElementException("Seq requires a head");
    _iter = iter;
    _state = new State<E>();
    _state._val = iter.next();
    _state._rest = (E) _state;
  }

  public static <E> Seq<E> create(Iterator<E> iter) {
    return iter.hasNext() ? new Seq<E>(iter) : null;
  }

  public Seq(IMetadata meta, Iterator<E> iter, State<E> state) {
    super(meta);
    _iter = iter;
    _state = state;
  }

  @Override
  public E peekFirst() {
    return _state._val;
  }

  @SuppressWarnings("unchecked")
  @Override
  public Seq<E> popFirst() {
    if (_state._rest == _state) {
      synchronized (_state) {
        if (_state._rest == _state) {
          _state._rest = _iter.hasNext() ? (E) (new Seq<E>(_iter)) : null;
        }
      }
    }
    return (Seq<E>) _state._rest;
  }

  @Override
  public Iterator<E> iterator() {
    return new Iterator<E>() {
      Seq<E> current = Seq.this;
      boolean advance;

      private void advanceIfNeeded() {
        if (advance) {
          current = current == null ? null : current.popFirst();
          advance = false;
        }
      }

      @Override
      public boolean hasNext() {
        advanceIfNeeded();
        return current != null;
      }

      @Override
      public E next() {
        advanceIfNeeded();
        if (current == null) throw new NoSuchElementException();
        E value = current.peekFirst();
        advance = true;
        return value;
      }
    };
  }

  @Override
  public long count() {
    long count = 0;
    Seq<E> current = this;
    while (current != null) {
      count++;
      current = current.popFirst();
    }
    return count;
  }

  @Override
  public String display() {
    StringBuilder output = new StringBuilder("(");
    Iterator<E> values = iterator();
    int displayed = 0;
    while (displayed < DISPLAY_LIMIT && values.hasNext()) {
      if (displayed > 0) output.append(' ');
      output.append(G.display(values.next()));
      displayed++;
    }
    if (values.hasNext()) output.append(displayed == 0 ? "..." : " ...");
    return output.append(')').toString();
  }

  @Override
  public Seq<E> withMeta(IMetadata meta) {
    return new Seq<E>(meta, _iter, _state);
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
