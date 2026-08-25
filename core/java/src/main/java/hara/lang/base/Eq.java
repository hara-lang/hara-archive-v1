package hara.lang.base;

import hara.lang.base.primitive.Num;
import hara.lang.protocol.IMapType;
import hara.lang.protocol.ISequential;
import hara.lang.protocol.IEquality;

import java.util.Iterator;
import java.util.List;
import java.util.function.BiPredicate;

public interface Eq {

  public static boolean eq(Object a, Object b) {
    if (a == b) {
      return true;
    } else if (a == null) {
      return b instanceof IEquality && ((IEquality) b).equality(null);
    } else if (b == null) {
      return a instanceof IEquality && ((IEquality) a).equality(null);
    } else if (a instanceof Number && b instanceof Number) {
      return Num.eq(a, b);
    } else if (a instanceof byte[] && b instanceof byte[]) {
      return java.util.Arrays.equals((byte[]) a, (byte[]) b);
    } else if (a instanceof IMapType<?, ?> left && b instanceof IMapType<?, ?> right) {
      if (left.count() != right.count()) return false;
      for (Object value : left) {
        java.util.Map.Entry<?, ?> entry = (java.util.Map.Entry<?, ?>) value;
        Object matching = ((IMapType) right).find(entry.getKey());
        if (!(matching instanceof java.util.Map.Entry<?, ?> other)
            || !eq(entry.getValue(), other.getValue())) return false;
      }
      return true;
    } else if (a instanceof ISequential<?> left && b instanceof ISequential<?> right) {
      return left.equality(right);
    } else if (a instanceof IEquality) {
      return ((IEquality) a).equality(b);
    } else if (b instanceof IEquality) {
      return ((IEquality) b).equality(a);
    } else {
      return a.equals(b);
    }
  }

  public static boolean eqIterable(Iterable<Object> a, Iterable<Object> b) {
    return eqIterator(a.iterator(), b.iterator(), Eq::eq);
  }

  public static boolean eqList(List<Object> a, List<Object> b) {
    if (a.size() != b.size()) {
      return false;
    } else {
      return eqIterator(a.iterator(), b.iterator(), Eq::eq);
    }
  }

  public static boolean eqList(List<Object> a, Iterator<Object> b) {
    return eqIterator(a.iterator(), b, Eq::eq);
  }

  public static boolean eqIterator(Iterator<Object> a, Iterator<Object> b) {
    return eqIterator(a, b, Eq::eq);
  }

  public static boolean eqIterator(
      Iterator<Object> a, Iterator<Object> b, BiPredicate<Object, Object> equals) {
    return Iter.equals(a, b, equals);
  }
}
