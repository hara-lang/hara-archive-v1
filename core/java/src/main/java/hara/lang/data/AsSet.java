package hara.lang.data;

import hara.lang.protocol.ISetType;
import hara.lang.data.types.ObjMutable;

import java.util.Iterator;

public class AsSet<E> extends ObjMutable implements ISetType<E> {
  final java.util.Set<E> _s;

  public AsSet(java.util.Set<E> s) {
    _s = s;
  }

  @Override
  public AsSet<E> conj(E e) {
    _s.add(e);
    return this;
  }

  @Override
  public long count() {
    return _s.size();
  }

  @Override
  public AsSet<E> dissoc(E k) {
    _s.remove(k);
    return this;
  }

  @Override
  public AsSet<E> empty() {
    _s.clear();
    return this;
  }

  @Override
  public E find(E key) {
    return (_s.contains(key)) ? key : null;
  }

  @Override
  public Iterator<E> iterator() {
    return _s.iterator();
  }
}
