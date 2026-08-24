package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;
import java.util.Iterator;

/** Stateful traversal cursor. */
@HaraProtocolBinding(
    namespace = "std.protocol.iiterator", name = "IIterator", parents = {"IIter"})
public interface IIterator<E> extends IIter<E>, Iterator<E> {
  @Override
  default Iterator<E> iter() {
    return this;
  }

  @HaraMethod(value = "iter-next?", arity = 1)
  boolean hasNext();

  @HaraMethod(value = "iter-next", arity = 1)
  E next();
}
