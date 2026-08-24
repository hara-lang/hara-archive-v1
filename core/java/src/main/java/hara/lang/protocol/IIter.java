package hara.lang.protocol;

import java.util.Iterator;
import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

/** Produces a fresh traversal cursor for a value. */
@HaraProtocolBinding(namespace = "std.protocol.iiter", name = "IIter")
public interface IIter<E> {
  @HaraMethod(value = "iter", arity = 1)
  Iterator<E> iter();
}
