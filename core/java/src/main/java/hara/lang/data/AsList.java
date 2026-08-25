package hara.lang.data;

import hara.lang.protocol.ILinearType;
import hara.lang.data.types.ObjMutable;

import java.util.Iterator;

public class AsList<E> extends ObjMutable
    implements ILinearType<E>,
        hara.lang.protocol.IPushFirst<E>,
        hara.lang.protocol.IPushLast<E>,
        hara.lang.protocol.IPopFirst,
        hara.lang.protocol.IPopLast {

  final java.util.List<E> _l;

  public AsList(java.util.List<E> l) {
    _l = l;
  }

  @Override
  public long count() {
    return _l.size();
  }

  @Override
  public AsList<E> empty() {
    _l.clear();
    return this;
  }

  @Override
  public Iterator<E> iterator() {
    return _l.iterator();
  }

  @Override
  public E nth(long i) {
    return _l.get((int) i);
  }

  @Override
  public E peekFirst() {
    return _l.get(0);
  }

  @Override
  public E peekLast() {
    return _l.get(_l.size() - 1);
  }

  @Override
  public AsList<E> popFirst() {
    _l.remove(0);
    return this;
  }

  @Override
  public AsList<E> popLast() {
    _l.remove(_l.size() - 1);
    return this;
  }

  @Override
  public AsList<E> pushFirst(E e) {
    _l.add(0, e);
    return this;
  }

  @Override
  public AsList<E> pushLast(E e) {
    _l.add(e);
    return this;
  }
}
