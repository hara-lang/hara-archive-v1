package hara.lang.protocol;

import hara.lang.base.Ex;
import hara.lang.protocol.ILinearType;
import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

import java.util.Iterator;
import java.util.function.BiFunction;
import java.util.function.Function;
import java.util.function.Supplier;

@HaraProtocolBinding(namespace = "std.protocol.ifn", name = "IFn")
public interface IFn<R, T1, T2> extends Function<Object, R> {

  @SuppressWarnings("rawtypes")
  public static <R, T1, T2> R applyAsIterator(IFn<R, T1, T2> f, Iterator vargs) {
    return f.getArgN().apply(vargs);
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  public static <R, T1, T2> R applyAsIterator(IFn<R, T1, T2> f, Iterator it, long size) {
    switch ((int) size) {
      case 0:
        return f.getArg0().get();
      case 1:
        return f.getArg1().apply((T1) it.next());
      case 2:
        return f.getArg2().apply((T1) it.next(), (T2) it.next());
      default:
        return f.getArgN().apply(it);
    }
  }

  @SuppressWarnings("unchecked")
  public static <R, T1, T2> R applyAsArray(IFn<R, T1, T2> f, Object[] vargs) {
    int len = vargs.length;
    switch (len) {
      case 0:
        return f.getArg0().get();
      case 1:
        return f.getArg1().apply((T1) vargs[0]);
      case 2:
        return f.getArg2().apply((T1) vargs[0], (T2) (T1) vargs[1]);
      default:
        return f.getArgN().apply(vargs);
    }
  }

  @SuppressWarnings({"rawtypes"})
  @Override
  default R apply(Object vargs) {
    if (vargs instanceof Iterator) {
      return applyAsIterator(this, (Iterator) vargs);
    } else if (vargs.getClass().isArray()) {
      return applyAsArray(this, (Object[]) vargs);
    } else if (vargs instanceof java.util.List) {
      var l = (java.util.List) vargs;
      return applyAsIterator(this, l.iterator(), l.size());
    } else if (vargs instanceof ILinearType) {
      var l = (ILinearType) vargs;
      return applyAsIterator(this, l.iterator(), l.count());
    } else if (vargs instanceof Iterable) {
      return applyAsIterator(this, (Iterator) vargs);
    } else {
      throw new Ex.Unsupported();
    }
  }

  default Supplier<R> getArg0() {
    throw new Ex.Arity(0, "No arity 0");
  }

  default Function<T1, R> getArg1() {
    throw new Ex.Arity(1, "No arity 1");
  }

  default BiFunction<T1, T2, R> getArg2() {
    throw new Ex.Arity(2, "No arity 2");
  }

  default Function<Object, R> getArgN() {
    throw new Ex.Arity(0, "No arity N");
  }

  default R invoke() {
    return getArg0().get();
  }

  default R invoke(T1 a1) {
    return getArg1().apply(a1);
  }

  default R invoke(T1 a1, T2 a2) {
    return getArg2().apply(a1, a2);
  }

  @HaraMethod(value = "invoke", arity = -1, variadic = true)
  default R invoke(Object... vargs) {
    return getArgN().apply(vargs);
  }
}
