package hara.lang.data.types;

import hara.lang.protocol.*;

public interface ILinkedType<E>
    extends Iterable<E>, IEmpty, IPushFirst<E>, IPopFirst, IPeekFirst<E>, ICons<E>, ICount {

  @Override
  default ILinkedType<E> cons(E e) {
    return (ILinkedType<E>) pushFirst(e);
  }

  default String startString() {
    return "(";
  }

  default String endString() {
    return ")";
  }
}
