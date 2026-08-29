package hara.lang.data.types;

import hara.lang.base.Iter;
import hara.lang.data.MapEntry;
import hara.lang.protocol.*;

import java.util.Iterator;
import java.util.Map.Entry;

public interface ISequentialLookupType<E>
    extends ISequential<E>,
        Iterable<E>,
        ICount,
        INth<E>,
        ILookup<Long, E>,
        IPeekFirst<E>,
        IPeekLast<E> {

  @Override
  default Entry<Long, E> find(Long idx) {
    if (idx >= 0 && idx < count()) {
      E out = nth(idx);
      return new MapEntry<>(null, idx, out);
    }
    return null;
  }

  @Override
  default Iterator<Long> keys() {
    return Iter.range(0, count());
  }

  @Override
  default E lookup(Long idx) {
    return nth(idx);
  }

  @Override
  default E peekFirst() {
    return count() == 0 ? null : nth(0);
  }

  @Override
  default E peekLast() {
    long size = count();
    return size == 0 ? null : nth(size - 1);
  }

  @Override
  default Iterator<E> vals() {
    return this.iterator();
  }
}
