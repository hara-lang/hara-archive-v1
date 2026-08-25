package hara.lang.protocol;

import hara.lang.base.Eq;
import hara.lang.base.G;
import hara.lang.base.Iter;
import hara.lang.declaration.HaraProtocolBinding;
import java.util.function.Function;

/** Portable set-category protocol descriptor. */
@HaraProtocolBinding(
    namespace = "std.protocol.isettype",
    name = "ISetType",
    parents = {"IColl", "ICount", "IObjType", "IDissoc", "IFind", "IFn"})
public interface ISetType<E>
    extends IColl<E>, ICount, IObjType, IDissoc<E>, IFind<E, E>, IFn<E, E, E> {

  default java.util.Set<E> asJavaSet() {
    return null;
  }

  @Override
  default Constant.ObjType getObjType() {
    return Constant.ObjType.SET;
  }

  @SuppressWarnings({"unchecked", "rawtypes"})
  @Override
  default boolean equality(Object obj) {
    if (obj instanceof ISetType) {
      return count() == ((ISetType) obj).count()
          && Iter.every(iterator(), element -> ((ISetType) obj).find(element) != null);
    }
    if (obj instanceof java.util.Set) {
      return count() == ((java.util.Set) obj).size()
          && Iter.every(
              ((java.util.Set) obj).iterator(), element -> find((E) element) != null);
    }
    return false;
  }

  @Override
  default long hashCalc(Constant.HashType type) {
    Function<Object, Long> hash = G.hashFn(type);
    return Iter.reduce(
        iterator(), Long.valueOf(hashSeed().hashCode()), (acc, element) -> acc + hash.apply(element));
  }

  @Override
  default String startString() {
    return "#{";
  }

  @Override
  default String endString() {
    return "}";
  }

  @Override
  default E invoke(E key) {
    return find(key);
  }

  @Override
  default E invoke(E key, E notFound) {
    E value = find(key);
    return value == null ? notFound : value;
  }
}
