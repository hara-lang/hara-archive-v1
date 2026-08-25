package hara.lang.protocol;

import hara.lang.data.Cons;
import hara.lang.data.Seq;
import hara.lang.declaration.HaraProtocolBinding;

/** Portable linear-collection category protocol descriptor. */
@HaraProtocolBinding(
    namespace = "std.protocol.ilineartype",
    name = "ILinearType",
    parents = {
      "ISequential", "IColl", "IPeekFirst", "IPeekLast", "ICons", "IConj", "INth", "ICount"
    })
public interface ILinearType<E>
    extends
        ISequential<E>,
        IColl<E>,
        IPeekFirst<E>,
        IPeekLast<E>,
        ICons<E>,
        IConj<E>,
        INth<E>,
        ICount {

  @Override
  default ICons<E> cons(E element) {
    Seq<E> tail = Seq.create(iterator());
    return new Cons<>(null, element, tail);
  }

  @SuppressWarnings("unchecked")
  @Override
  default ILinearType<E> conj(E element) {
    return (ILinearType<E>) ((IPushLast<E>) this).pushLast(element);
  }

  @Override
  default String startString() {
    return "[";
  }

  @Override
  default String endString() {
    return "]";
  }
}
