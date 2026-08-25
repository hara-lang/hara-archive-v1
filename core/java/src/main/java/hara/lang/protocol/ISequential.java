package hara.lang.protocol;

import hara.lang.base.Eq;
import hara.lang.data.types.IOrderedType;
import hara.lang.declaration.HaraProtocolBinding;

import java.util.Iterator;
import java.util.List;

/** Portable ordered-sequence protocol. */
@HaraProtocolBinding(
    namespace = "std.protocol.isequential",
    name = "ISequential",
    parents = {"IEquality", "IHash", "IObjType"})
public interface ISequential<E>
    extends Iterable<E>, IEquality, IHash, IObjType, IOrderedType<E> {

  @SuppressWarnings("unchecked")
  @Override
  default boolean equality(Object obj) {
    if (obj instanceof ISequential<?> other) {
      return equalIterators(iterator(), other.iterator());
    }
    if (obj instanceof List<?> other) {
      return equalIterators(iterator(), other.iterator());
    }
    return false;
  }

  private static boolean equalIterators(Iterator<?> left, Iterator<?> right) {
    while (left.hasNext() && right.hasNext()) {
      if (!Eq.eq(left.next(), right.next())) return false;
    }
    return !left.hasNext() && !right.hasNext();
  }

  @Override
  default Constant.ObjType getObjType() {
    return Constant.ObjType.SEQUENTIAL;
  }
}
