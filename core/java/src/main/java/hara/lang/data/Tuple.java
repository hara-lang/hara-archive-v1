package hara.lang.data;

import hara.lang.base.Ex;
import hara.lang.base.Iter;
import hara.lang.protocol.ILinearType;
import hara.lang.data.types.ObjEmpty;
import hara.lang.data.types.ObjPersistent;
import hara.lang.protocol.IMetadata;
import hara.lang.protocol.IObjType;
import hara.lang.protocol.IPopFirst;
import hara.lang.protocol.IPopLast;
import hara.lang.protocol.IPushFirst;
import hara.lang.protocol.IPushLast;

import java.util.Iterator;

public interface Tuple {

  /** Returns whether a value uses the compact, fixed-arity vector backend. */
  static boolean isCompact(Object value) {
    return value instanceof Tup0
        || value instanceof Tup1
        || value instanceof Tup2
        || value instanceof Tup3
        || value instanceof Tup4
        || value instanceof Tup5
        || value instanceof Tup6
        || value instanceof Tup7
        || value instanceof Tup8;
  }

  @SuppressWarnings({"unchecked", "rawtypes"})
  public class Tup0 extends ObjEmpty
      implements ILinearType, IPushFirst, IPushLast, IPopFirst, IPopLast {

    public static final Tup0 EMPTY = new Tup0(null);

    public Tup0(IMetadata meta) {
      super(meta);
    }

    @Override
    public Tup0 withMeta(IMetadata meta) {
      return (_meta == meta) ? this : new Tup0(meta);
    }

    @Override
    public Tup1.L pushFirst(Object e) {
      return new Tup1.L(_meta, e);
    }

    @Override
    public Tup1.L pushLast(Object e) {
      return new Tup1.L(_meta, e);
    }

    @Override
    public Tup0 popFirst() {
      return this;
    }

    @Override
    public Tup0 popLast() {
      return this;
    }

    @Override
    public Object peekFirst() {
      return null;
    }

    @Override
    public Object peekLast() {
      return null;
    }
  }

  @SuppressWarnings({"unchecked", "rawtypes"})
  public interface Tup1<A>
      extends ILinearType, IPushFirst, IPushLast, IPopFirst, IPopLast {

    @Override
    default long count() {
      return 1;
    }

    @Override
    default Tup0 empty() {
      return Tup0.EMPTY.withMeta(meta());
    }

    A A();

    public class L<A> extends ObjPersistent implements Tup1<A> {

      final A _a;

      @Override
      public A A() {
        return _a;
      }

      public L(IMetadata meta, A a) {
        super(meta);
        _a = a;
      }

      @Override
      public Iterator iterator() {
        return Iter.objects(_a);
      }

      @Override
      public IObjType withMeta(IMetadata meta) {
        return (_meta == meta) ? this : new L<A>(meta, _a);
      }

      @Override
      public Object nth(long i) {
        switch ((int) i) {
          case 0:
            return _a;
          default:
            throw new Ex.NoSuchElement();
        }
      }

      @Override
      public Tup2 pushFirst(Object e) {
        return new Tup2.L(_meta, e, _a);
      }

      @Override
      public Tup2 pushLast(Object e) {
        return new Tup2.L(_meta, _a, e);
      }

      @Override
      public Tup0 popFirst() {
        return Tup0.EMPTY.withMeta(_meta);
      }

      @Override
      public Tup0 popLast() {
        return Tup0.EMPTY.withMeta(_meta);
      }

      @Override
      public Object peekFirst() {
        return _a;
      }

      @Override
      public Object peekLast() {
        return _a;
      }
    }
  }

  @SuppressWarnings({"unchecked", "rawtypes"})
  public interface Tup2<A, B> extends Tup1<A>, ILinearType {

    @Override
    default long count() {
      return 2;
    }

    B B();

    public class L<A, B> extends ObjPersistent implements Tup2<A, B> {
      A _a;
      B _b;

      public L(IMetadata meta, A a, B b) {
        super(meta);
        _a = a;
        _b = b;
      }

      @Override
      public A A() {
        return _a;
      }

      @Override
      public B B() {
        return _b;
      }

      @Override
      public Iterator iterator() {
        return Iter.objects(_a, _b);
      }

      @Override
      public IObjType withMeta(IMetadata meta) {
        return (_meta == meta) ? this : new L<A, B>(meta, _a, _b);
      }

      @Override
      public Object nth(long i) {
        switch ((int) i) {
          case 0:
            return _a;
          case 1:
            return _b;
          default:
            throw new Ex.NoSuchElement();
        }
      }

      @Override
      public Tup3 pushFirst(Object e) {
        return new Tup3.L(_meta, e, _a, _b);
      }

      @Override
      public Tup3 pushLast(Object e) {
        return new Tup3.L(_meta, _a, _b, e);
      }

      @Override
      public Tup1 popFirst() {
        return new Tup1.L(_meta, _b);
      }

      @Override
      public Tup1 popLast() {
        return new Tup1.L(_meta, _a);
      }

      @Override
      public Object peekFirst() {
        return _a;
      }

      @Override
      public Object peekLast() {
        return _b;
      }
    }
  }

  @SuppressWarnings({"unchecked", "rawtypes"})
  public interface Tup3<A, B, X> extends Tup2<A, B>, ILinearType {

    @Override
    default long count() {
      return 3;
    }

    public X C();

    public class L<A, B, C> extends ObjPersistent implements Tup3<A, B, C> {
      A _a;
      B _b;
      C _c;

      public L(IMetadata meta, A a, B b, C c) {
        super(meta);
        _a = a;
        _b = b;
        _c = c;
      }

      @Override
      public A A() {
        return _a;
      }

      @Override
      public B B() {
        return _b;
      }

      @Override
      public C C() {
        return _c;
      }

      @Override
      public Iterator iterator() {
        return Iter.objects(_a, _b, _c);
      }

      @Override
      public IObjType withMeta(IMetadata meta) {
        return (_meta == meta) ? this : new L<A, B, C>(meta, _a, _b, _c);
      }

      @Override
      public Object nth(long i) {
        switch ((int) i) {
          case 0:
            return _a;
          case 1:
            return _b;
          case 2:
            return _c;
          default:
            throw new Ex.NoSuchElement();
        }
      }

      @Override
      public Tup4 pushFirst(Object e) {
        return new Tup4.L(_meta, e, _a, _b, _c);
      }

      @Override
      public Tup4 pushLast(Object e) {
        return new Tup4.L(_meta, _a, _b, _c, e);
      }

      @Override
      public Tup2 popFirst() {
        return new Tup2.L(_meta, _b, _c);
      }

      @Override
      public Tup2 popLast() {
        return new Tup2.L(_meta, _a, _b);
      }

      @Override
      public Object peekFirst() {
        return _a;
      }

      @Override
      public Object peekLast() {
        return _c;
      }
    }
  }

  @SuppressWarnings({"unchecked", "rawtypes"})
  public interface Tup4<A, B, C, D> extends Tup3<A, B, C>, ILinearType {

    @Override
    default long count() {
      return 4;
    }

    public D D();

    public class L<A, B, C, D> extends ObjPersistent implements Tup4<A, B, C, D> {
      A _a;
      B _b;
      C _c;
      D _d;

      public L(IMetadata meta, A a, B b, C c, D d) {
        super(meta);
        _a = a;
        _b = b;
        _c = c;
        _d = d;
      }

      @Override
      public A A() {
        return _a;
      }

      @Override
      public B B() {
        return _b;
      }

      @Override
      public C C() {
        return _c;
      }

      @Override
      public D D() {
        return _d;
      }

      @Override
      public Iterator iterator() {
        return Iter.objects(_a, _b, _c, _d);
      }

      @Override
      public IObjType withMeta(IMetadata meta) {
        return (_meta == meta) ? this : new L<A, B, C, D>(meta, _a, _b, _c, _d);
      }

      @Override
      public Object nth(long i) {
        switch ((int) i) {
          case 0:
            return _a;
          case 1:
            return _b;
          case 2:
            return _c;
          case 3:
            return _d;
          default:
            throw new Ex.NoSuchElement();
        }
      }

      @Override
      public Tup5 pushFirst(Object e) {
        return new Tup5.L(_meta, e, _a, _b, _c, _d);
      }

      @Override
      public Tup5 pushLast(Object e) {
        return new Tup5.L(_meta, _a, _b, _c, _d, e);
      }

      @Override
      public Tup3 popFirst() {
        return new Tup3.L(_meta, _b, _c, _d);
      }

      @Override
      public Tup3 popLast() {
        return new Tup3.L(_meta, _a, _b, _c);
      }

      @Override
      public Object peekFirst() {
        return _a;
      }

      @Override
      public Object peekLast() {
        return _d;
      }
    }
  }

  @SuppressWarnings({"unchecked", "rawtypes"})
  public interface Tup5<A, B, C, D, E> extends Tup4<A, B, C, D>, ILinearType {

    @Override
    default long count() {
      return 5;
    }

    public E E();

    public class L<A, B, X, Y, Z> extends ObjPersistent implements Tup5<A, B, X, Y, Z> {
      A _a;
      B _b;
      X _c;
      Y _d;
      Z _e;

      public L(IMetadata meta, A a, B b, X c, Y d, Z e) {
        super(meta);
        _a = a;
        _b = b;
        _c = c;
        _d = d;
        _e = e;
      }

      @Override
      public A A() {
        return _a;
      }

      @Override
      public B B() {
        return _b;
      }

      @Override
      public X C() {
        return _c;
      }

      @Override
      public Y D() {
        return _d;
      }

      @Override
      public Z E() {
        return _e;
      }

      @Override
      public Iterator iterator() {
        return Iter.objects(_a, _b, _c, _d, _e);
      }

      @Override
      public IObjType withMeta(IMetadata meta) {
        return (_meta == meta) ? this : new L<A, B, X, Y, Z>(meta, _a, _b, _c, _d, _e);
      }

      @Override
      public Object nth(long i) {
        switch ((int) i) {
          case 0:
            return _a;
          case 1:
            return _b;
          case 2:
            return _c;
          case 3:
            return _d;
          case 4:
            return _e;
          default:
            throw new Ex.NoSuchElement();
        }
      }

      @Override
      public Tup6 pushFirst(Object x) {
        return new Tup6.L(_meta, x, _a, _b, _c, _d, _e);
      }

      @Override
      public Tup6 pushLast(Object x) {
        return new Tup6.L(_meta, _a, _b, _c, _d, _e, x);
      }

      @Override
      public Tup4 popFirst() {
        return new Tup4.L(_meta, _b, _c, _d, _e);
      }

      @Override
      public Tup4 popLast() {
        return new Tup4.L(_meta, _a, _b, _c, _d);
      }

      @Override
      public Object peekFirst() {
        return _a;
      }

      @Override
      public Object peekLast() {
        return _e;
      }
    }
  }

  @SuppressWarnings({"unchecked", "rawtypes"})
  public interface Tup6<A, B, C, D, E, F>
      extends Tup5<A, B, C, D, E>, ILinearType {

    @Override
    default long count() {
      return 6;
    }

    public F F();

    public class L<A, B, C, D, E, F> extends ObjPersistent implements Tup6<A, B, C, D, E, F> {
      A _a;
      B _b;
      C _c;
      D _d;
      E _e;
      F _f;

      public L(IMetadata meta, A a, B b, C c, D d, E e, F f) {
        super(meta);
        _a = a;
        _b = b;
        _c = c;
        _d = d;
        _e = e;
        _f = f;
      }

      @Override
      public A A() {
        return _a;
      }

      @Override
      public B B() {
        return _b;
      }

      @Override
      public C C() {
        return _c;
      }

      @Override
      public D D() {
        return _d;
      }

      @Override
      public E E() {
        return _e;
      }

      @Override
      public F F() {
        return _f;
      }

      @Override
      public Iterator iterator() {
        return Iter.objects(_a, _b, _c, _d, _e, _f);
      }

      @Override
      public IObjType withMeta(IMetadata meta) {
        return (_meta == meta) ? this : new L<A, B, C, D, E, F>(meta, _a, _b, _c, _d, _e, _f);
      }

      @Override
      public Object nth(long i) {
        switch ((int) i) {
          case 0:
            return _a;
          case 1:
            return _b;
          case 2:
            return _c;
          case 3:
            return _d;
          case 4:
            return _e;
          case 5:
            return _f;
          default:
            throw new Ex.NoSuchElement();
        }
      }

      @Override
      public Tup7 pushFirst(Object x) {
        return new Tup7.L(_meta, x, _a, _b, _c, _d, _e, _f);
      }

      @Override
      public Tup7 pushLast(Object x) {
        return new Tup7.L(_meta, _a, _b, _c, _d, _e, _f, x);
      }

      @Override
      public Tup5 popFirst() {
        return new Tup5.L(_meta, _b, _c, _d, _e, _f);
      }

      @Override
      public Tup5 popLast() {
        return new Tup5.L(_meta, _a, _b, _c, _d, _e);
      }

      @Override
      public Object peekFirst() {
        return _a;
      }

      @Override
      public Object peekLast() {
        return _f;
      }
    }
  }

  @SuppressWarnings({"unchecked", "rawtypes"})
  public interface Tup7<A, B, C, D, E, F, G>
      extends Tup6<A, B, C, D, E, F>, ILinearType {

    @Override
    default long count() {
      return 7;
    }

    public G G();

    public class L<A, B, C, D, E, F, G> extends ObjPersistent implements Tup7<A, B, C, D, E, F, G> {
      A _a;
      B _b;
      C _c;
      D _d;
      E _e;
      F _f;
      G _g;

      public L(IMetadata meta, A a, B b, C c, D d, E e, F f, G g) {
        super(meta);
        _a = a;
        _b = b;
        _c = c;
        _d = d;
        _e = e;
        _f = f;
        _g = g;
      }

      @Override
      public A A() {
        return _a;
      }

      @Override
      public B B() {
        return _b;
      }

      @Override
      public C C() {
        return _c;
      }

      @Override
      public D D() {
        return _d;
      }

      @Override
      public E E() {
        return _e;
      }

      @Override
      public F F() {
        return _f;
      }

      @Override
      public G G() {
        return _g;
      }

      @Override
      public Iterator iterator() {
        return Iter.objects(_a, _b, _c, _d, _e, _f, _g);
      }

      @Override
      public IObjType withMeta(IMetadata meta) {
        return (_meta == meta)
            ? this
            : new L<A, B, C, D, E, F, G>(meta, _a, _b, _c, _d, _e, _f, _g);
      }

      @Override
      public Object nth(long i) {
        switch ((int) i) {
          case 0:
            return _a;
          case 1:
            return _b;
          case 2:
            return _c;
          case 3:
            return _d;
          case 4:
            return _e;
          case 5:
            return _f;
          case 6:
            return _g;
          default:
            throw new Ex.NoSuchElement();
        }
      }

      @Override
      public Tup8 pushFirst(Object x) {
        return new Tup8.L(_meta, x, _a, _b, _c, _d, _e, _f, _g);
      }

      @Override
      public Tup8 pushLast(Object x) {
        return new Tup8.L(_meta, _a, _b, _c, _d, _e, _f, _g, x);
      }

      @Override
      public Tup6 popFirst() {
        return new Tup6.L(_meta, _b, _c, _d, _e, _f, _g);
      }

      @Override
      public Tup6 popLast() {
        return new Tup6.L(_meta, _a, _b, _c, _d, _e, _f);
      }

      @Override
      public Object peekFirst() {
        return _a;
      }

      @Override
      public Object peekLast() {
        return _g;
      }
    }
  }

  @SuppressWarnings({"unchecked", "rawtypes"})
  public interface Tup8<A, B, C, D, E, F, G, H>
      extends Tup7<A, B, C, D, E, F, G>, ILinearType {

    @Override
    default long count() {
      return 8;
    }

    public H H();

    public class L<A, B, C, D, E, F, G, H> extends ObjPersistent
        implements Tup8<A, B, C, D, E, F, G, H> {
      A _a;
      B _b;
      C _c;
      D _d;
      E _e;
      F _f;
      G _g;
      H _h;

      public L(IMetadata meta, A a, B b, C c, D d, E e, F f, G g, H h) {
        super(meta);
        _a = a;
        _b = b;
        _c = c;
        _d = d;
        _e = e;
        _f = f;
        _g = g;
        _h = h;
      }

      @Override
      public A A() {
        return _a;
      }

      @Override
      public B B() {
        return _b;
      }

      @Override
      public C C() {
        return _c;
      }

      @Override
      public D D() {
        return _d;
      }

      @Override
      public E E() {
        return _e;
      }

      @Override
      public F F() {
        return _f;
      }

      @Override
      public G G() {
        return _g;
      }

      @Override
      public H H() {
        return _h;
      }

      @Override
      public Iterator iterator() {
        return Iter.objects(_a, _b, _c, _d, _e, _f, _g, _h);
      }

      @Override
      public IObjType withMeta(IMetadata meta) {
        return (_meta == meta)
            ? this
            : new L<A, B, C, D, E, F, G, H>(meta, _a, _b, _c, _d, _e, _f, _g, _h);
      }

      @Override
      public Object nth(long i) {
        switch ((int) i) {
          case 0:
            return _a;
          case 1:
            return _b;
          case 2:
            return _c;
          case 3:
            return _d;
          case 4:
            return _e;
          case 5:
            return _f;
          case 6:
            return _g;
          case 7:
            return _h;
          default:
            throw new Ex.NoSuchElement();
        }
      }

      @Override
      public IPushFirst pushFirst(Object x) {
        return List.Standard.from(_meta, x, _a, _b, _c, _d, _e, _f, _g, _h);
      }

      @Override
      public IPushLast pushLast(Object x) {
        return Vector.Standard.from(_meta, _a, _b, _c, _d, _e, _f, _g, _h, x);
      }

      @Override
      public Tup7 popFirst() {
        return new Tup7.L(_meta, _b, _c, _d, _e, _f, _g, _h);
      }

      @Override
      public Tup7 popLast() {
        return new Tup7.L(_meta, _a, _b, _c, _d, _e, _f, _g);
      }

      @Override
      public Object peekFirst() {
        return _a;
      }

      @Override
      public Object peekLast() {
        return _h;
      }
    }
  }
}
