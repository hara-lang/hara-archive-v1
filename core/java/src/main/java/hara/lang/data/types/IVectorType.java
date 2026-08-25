package hara.lang.data.types;

import hara.lang.protocol.*;

public interface IVectorType<E>
    extends IColl<E>,
        ISequentialLookupType<E>,
        ILinearType<E>,
        INth<E>,
        IPopLast,
        IPushLast<E>,
        IFn<E, Integer, E> {

  @Override
  default IVectorType<E> conj(E v) {
    return (IVectorType<E>) pushLast(v);
  }

  @Override
  default String startString() {
    return "[";
  }

  @Override
  default String endString() {
    return "]";
  }

  @Override
  public default E invoke(Integer key) {
    return nth(key);
  }
}
