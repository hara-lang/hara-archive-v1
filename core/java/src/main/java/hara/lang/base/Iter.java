package hara.lang.base;

import hara.lang.base.iter.*;
import hara.lang.base.primitive.Array;
import hara.lang.data.HaraCharacter;
import hara.lang.data.MapEntry;
import hara.lang.data.Tuple;
import hara.lang.protocol.IPair;
import hara.lang.protocol.IToMutable;
import hara.lang.protocol.IToPersistent;

import java.util.*;
import java.util.Map.Entry;
import java.util.concurrent.atomic.AtomicReference;
import java.util.function.*;

public interface Iter {

  public interface Nil {
    public static Iterator<Object> ITERATOR = new Iterator<Object>();

    public class Iterator<V> implements EmptyIteratorType<V> {}

    public static ListIterator<Object> LIST_ITERATOR = new ListIterator<Object>();

    public class ListIterator<V> implements EmptyListIteratorType<V> {}

    public static Spliterator<Object> SPLITERATOR = new Spliterator<Object>();

    public class Spliterator<V> implements EmptySpliterator<V> {}
  }

  // Moved to hara.lang.base.iter
  // UnmodifiableIteratorType
  // UnmodifiableListIteratorType
  // EmptySpliterator
  // EmptyIteratorType
  // EmptyListIteratorType
  // ConcatIterator
  // SingletonIterator
  // State

  public static Iterator<?> emptyIterator() {
    return Nil.ITERATOR;
  }

  public static ListIterator<?> emptyListIterator() {
    return Nil.LIST_ITERATOR;
  }

  public static Spliterator<?> emptySpliterator() {
    return Nil.SPLITERATOR;
  }

  @SuppressWarnings("rawtypes")
  public static Iterator iter(Object obj) {
    if (obj == null) {
      return Nil.ITERATOR;
    } else if (obj instanceof Iterator) {
      return (Iterator) obj;
    } else if (obj instanceof Iterable) {
      return ((Iterable) obj).iterator();
    } else if (obj instanceof java.util.Map) {
      return ((java.util.Map) obj).entrySet().iterator();
    } else if (obj instanceof java.util.Map.Entry) {
      Entry e = (Entry) obj;
      return Iter.objects(e.getKey(), e.getValue());
    } else if (obj.getClass().isArray()) {
      return Array.toIter(obj);
    } else {
      throw new Ex.Unsupported();
    }
  }

  public static Iterator<Boolean> booleans(boolean... arr) {
    return new BooleanArrayIterator(arr, 0);
  }

  public static Iterator<Byte> bytes(byte... arr) {
    return new ByteArrayIterator(arr, 0);
  }

  public static Iterator<Character> chars(char... arr) {
    return new CharArrayIterator(arr, 0);
  }

  public static Iterator<HaraCharacter> codePoints(String value) {
    return new CodePointIterator(value);
  }

  public static Iterator<Short> shorts(short... arr) {
    return new ShortArrayIterator(arr, 0);
  }

  public static Iterator<Integer> ints(int... arr) {
    return new IntArrayIterator(arr, 0);
  }

  public static Iterator<Long> longs(long... arr) {
    return new LongArrayIterator(arr, 0);
  }

  public static Iterator<Float> floats(float... arr) {
    return new FloatArrayIterator(arr, 0);
  }

  public static Iterator<Double> doubles(double... arr) {
    return new DoubleArrayIterator(arr, 0);
  }

  @SafeVarargs
  public static <E> Iterator<E> objects(E... arr) {
    return new ArrayIterator<E>(arr, 0);
  }

  public static <E> Iterator<E> from(E[] arr) {
    return new ArrayIterator<>(arr, 0);
  }

  public static <E> Iterator<E> from(Iterable<E> coll) {
    return coll.iterator();
  }

  public static <E> Iterator<E> from(BooleanSupplier hasNext, Supplier<E> next) {
    return new Iterator<E>() {
      @Override
      public boolean hasNext() {
        return hasNext.getAsBoolean();
      }

      @Override
      public E next() {
        return next.get();
      }
    };
  }

  public static <E> boolean equals(Iterator<E> a, Iterator<E> b, BiPredicate<E, E> equals) {
    while (a.hasNext()) {
      if (!equals.test(a.next(), b.next())) {
        return false;
      }
    }
    return true;
  }

  //
  // Collect
  //

  @SuppressWarnings({"rawtypes", "unchecked"})
  public static <E, R> R collect(
      Function<Iterator, R> term, E[] array, Function<Iterator, Iterator>... pl) {
    return term.apply(stream(from(array), pl));
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  public static <E, R> R collect(
      Function<Iterator, R> term, Iterable<E> coll, Function<Iterator, Iterator>... pl) {
    return term.apply(stream(from(coll), pl));
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  public static <R> R collect(
      Function<Iterator, R> term, Iterator it, Function<Iterator, Iterator>... pl) {
    return term.apply(stream(it, pl));
  }

  //
  // Stream
  //
  @SuppressWarnings({"rawtypes", "unchecked"})
  public static <E, R> Iterator<R> stream(Iterator<E> it, Function<Iterator, Iterator>... fns) {
    return (Iterator<R>) Array.reduce((itr, f) -> f.apply(itr), it, fns);
  }

  //
  // Reduce
  //

  public static <E> E reduce(Iterator<E> it, BiFunction<E, E, E> f) {
    try {
      var _acc = it.next();
      while (it.hasNext()) {
        _acc = f.apply(_acc, it.next());
        if (Reduced.isReduced(_acc)) {
          return (E) Reduced.unreduced(_acc);
        }
      }
      return _acc;
    } finally {
      close(it);
    }
  }

  @SuppressWarnings("unchecked")
  public static <E, R> R reduce(Iterator<E> it, R init, BiFunction<R, E, R> f) {
    try {
      var _acc = init;
      while (it.hasNext()) {
        _acc = f.apply(_acc, it.next());
        if (Reduced.isReduced(_acc)) {
          return (R) Reduced.unreduced(_acc);
        }
      }
      return _acc;
    } finally {
      close(it);
    }
  }

  @SuppressWarnings("unchecked")
  public static <E, R> R reduce(
      Iterator<E> it, R init, BiFunction<R, E, R> f, Function<R, Boolean> end) {
    try {
      var _acc = init;
      while (it.hasNext()) {
        if (end.apply(_acc)) {
          return _acc;
        }
        _acc = f.apply(_acc, it.next());
        if (Reduced.isReduced(_acc)) {
          return (R) Reduced.unreduced(_acc);
        }
      }
      return _acc;
    } finally {
      close(it);
    }
  }

  @SuppressWarnings("unchecked")
  public static <E, R> R reduceIn(Iterator<E> it, R init, BiFunction<R, E, R> f) {
    R _init;
    boolean _change = false;
    if (init instanceof IToMutable) {
      _init = (R) ((IToMutable) init).toMutable();
      _change = true;
    } else {
      _init = init;
    }
    R out = reduce(it, _init, f);
    if (out instanceof IToPersistent && _change == true) {
      return (R) ((IToPersistent) out).toPersistent();
    } else {
      return out;
    }
  }

  @SuppressWarnings({"unchecked", "rawtypes"})
  public static <E, R> long count(Iterator it) {
    return (long) reduce(it, 0, (acc, i) -> acc + 1);
  }

  //
  // Map
  //

  public static <E, R> Function<Iterator<E>, Iterator<R>> map(Function<E, R> f) {
    return (it) -> map(it, f);
  }

  public static <E, R> Iterator<R> map(Iterator<E> it, Function<E, R> f) {
    return closeable(from(it::hasNext, () -> f.apply(it.next())), it);
  }

  public static <E> Consumer<Iterator<E>> each(Consumer<E> f) {
    return (it) -> it.forEachRemaining(f);
  }

  public static <E> void each(Iterator<E> it, Consumer<E> f) {
    it.forEachRemaining(f);
  }

  //
  // Equals
  //

  public static <E, R> Function<Iterator<E>, Iterator<R>> mapcat(
      Function<? super E, ? extends Iterator<? extends R>> f) {
    return (it) -> mapcat(it, f);
  }

  public static <E, R> Iterator<R> mapcat(
      Iterator<E> it, Function<? super E, ? extends Iterator<? extends R>> f) {
    return new Iterator<R>() {

      final Iterator<? extends E> _it = it;
      Iterator<?> _curr = emptyIterator();

      @Override
      public boolean hasNext() {
        while (!_curr.hasNext()) {
          if (!_it.hasNext()) {
            return false;
          }
          _curr = f.apply(_it.next());
        }
        return true;
      }

      @SuppressWarnings("unchecked")
      @Override
      public R next() {
        if (!hasNext()) {
          throw new Ex.NoSuchElement();
        }
        return (R) _curr.next();
      }
    };
  }

  //
  // Filter
  //

  public static <E, R> Function<Iterator<E>, Iterator<E>> filter(Predicate<E> f) {
    return (it) -> filter(it, f);
  }

  public static <E> Iterator<E> filter(Iterator<E> it, Predicate<E> f) {
    return closeable(new Iterator<E>() {

      private E _val = null;
      private State _state = State.NOT_SET;

      private void prime() {
        if (_state == State.NOT_SET) {
          while (it.hasNext()) {
            _val = it.next();
            if (f.test(_val)) {
              _state = State.READY;
              return;
            }
          }
          _state = State.DONE;
        }
      }

      @Override
      public boolean hasNext() {
        prime();
        return _state != State.DONE;
      }

      @Override
      public E next() {
        prime();
        if (_state == State.DONE) {
          throw new NoSuchElementException();
        }
        _state = State.NOT_SET;
        return _val;
      }
    }, it);
  }

  //
  // Keep
  //

  public static <E, R> Function<Iterator<E>, Iterator<R>> keep(Function<E, R> f) {
    return (it) -> keep(it, f);
  }

  public static <E, R> Iterator<R> keep(Iterator<E> it, Function<E, R> f) {
    return new Iterator<R>() {

      private R _out = null;
      private State _state = State.NOT_SET;

      private void prime() {
        if (_state == State.NOT_SET) {
          while (it.hasNext()) {
            E _val = it.next();
            _out = f.apply(_val);
            if (_out != null) {
              _state = State.READY;
              return;
            }
          }
          _state = State.DONE;
        }
      }

      @Override
      public boolean hasNext() {
        prime();
        return _state != State.DONE;
      }

      @Override
      public R next() {
        prime();
        if (_state == State.DONE) {
          throw new NoSuchElementException();
        }

        _state = State.NOT_SET;
        return _out;
      }
    };
  }

  //
  // Group by

  public static <K, E, R> HashMap<K, ArrayList<R>> groupBy(
      Iterator<E> it, Function<E, K> f, Function<E, R> p, HashMap<K, ArrayList<R>> map) {
    return reduce(
        it,
        map,
        (m, e) -> {
          K key = f.apply(e);
          ArrayList<R> l = m.get(key);
          if (l == null) {
            l = new ArrayList<R>();
            m.put(key, l);
          }
          l.add(p.apply(e));
          return m;
        });
  }

  public static <K, E, R> HashMap<K, ArrayList<R>> groupBy(
      Iterator<E> it, Function<E, K> f, Function<E, R> p) {
    return groupBy(it, f, p, new HashMap<K, ArrayList<R>>());
  }

  public static <K, E> Function<Iterator<E>, HashMap<K, ArrayList<E>>> groupBy(Function<E, K> f) {
    return (it) -> groupBy(it, f, x -> x);
  }

  public static <K, E, R> Function<Iterator<E>, HashMap<K, ArrayList<R>>> groupBy(
      Function<E, K> f, Function<E, R> p) {
    return (it) -> groupBy(it, f, p);
  }

  //

  //
  // Range
  //

  public static Iterator<Long> range(long max) {
    return range(0, max);
  }

  public static Iterator<Long> range(long min, long max) {
    return new Iterator<Long>() {

      long _i = min;

      @Override
      public boolean hasNext() {
        return _i < max;
      }

      @Override
      public Long next() {
        if (hasNext()) {
          return _i++;
        } else {
          throw new NoSuchElementException();
        }
      }
    };
  }

  //
  // Range
  //

  public static <E> Function<Iterator<E>, Boolean> contains(Predicate<E> f) {
    return it -> contains(it, f);
  }

  public static <E> boolean contains(Iterator<E> it, Predicate<E> f) {
    while (it.hasNext()) {
      if (f.test(it.next())) {
        return true;
      }
    }
    return false;
  }

  public static <E> String toString(
      Iterator<E> it, String start, String end, String sep, Function<E, String> f) {
    StringBuilder _sb = new StringBuilder().append(start);
    boolean _start = true;
    while (it.hasNext()) {
      if (!_start) {
        _sb.append(sep);
      }
      _start = false;
      _sb.append(f.apply(it.next()));
    }
    return _sb.append(end).toString();
  }

  public static <E> String display(Iterator<E> it, String start, String end, String sep) {
    return toString(it, start, end, sep, G::display);
  }

  public static <E> String display(Iterator<E> it) {
    return display(it, "(", ")", " ");
  }

  /*
   * public static void prString(Iterator<?> it, ) {
   * System.out.println(toString(it));
   * }
   */

  @SuppressWarnings({"unchecked", "rawtypes"})
  public static void pr(Iterator it) {
    System.out.println(display(it, "", "", " "));
  }

  //
  // Nth
  //

  public static <E> E nth(Iterator<E> it, long idx) {
    if (idx < 0) {
      throw new Ex.OutOfBounds();
    }
    var i = idx;
    for (i = idx; i > 0; i--) {
      it.next();
    }
    return it.next();
  }

  //
  // Arrays
  //

  public static <E> ArrayList<E> toArrayList(Iterator<? extends E> it) {
    ArrayList<E> list = new ArrayList<E>();
    while (it.hasNext()) {
      list.add(it.next());
    }
    return list;
  }

  @SafeVarargs
  @SuppressWarnings({"rawtypes", "unchecked"})
  public static <E> ArrayList<E> toArrayList(Object input, Function<Iterator, Iterator>... pl) {
    Iterator it = iter(input);
    return (ArrayList<E>) collect(Iter::toArrayList, it, pl);
  }

  public static Object[] toArray(Iterator<?> it) {
    return toArrayList(it).toArray();
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  public static Object[] toArray(Object input, Function<Iterator, Iterator>... pl) {
    Iterator it = iter(input);
    return collect(Iter::toArray, it, pl);
  }

  @SuppressWarnings({"unchecked", "rawtypes"})
  public static <E> E[] toArray(Iterator<E> it, Class cls) {
    ArrayList<E> c = toArrayList(it);
    E[] arr = (E[]) Array.newArray(cls, c.size());
    Array.fillArray(c.iterator(), arr);
    return arr;
  }

  //
  // Concat
  //

  @SuppressWarnings({"unchecked", "rawtypes"})
  public static Iterator concat(Iterator x, Iterator y) {
    return new ConcatIterator(Iter.objects(x, y));
  }

  public static <E> Iterator<E> concat(Iterator<? extends Iterator<? extends E>> all) {
    return new ConcatIterator<>(all);
  }

  public static <E> Iterator<E> flattenIterables(Iterator<? extends Iterable<E>> it) {
    return concat((Iterator) map(it, Iterable::iterator));
  }

  public static <E> Function<Iterator<E>, Iterator<E>> drop(int n) {
    return (it) -> drop(it, n);
  }

  public static <E> Iterator<E> drop(Iterator<E> it, int n) {
    return closeable(new Iterator<E>() {

      long _n = n;

      private void skip() {
        while (_n > 0 && it.hasNext()) {
          it.next();
          _n--;
        }
      }

      @Override
      public boolean hasNext() {
        skip();
        return it.hasNext();
      }

      @Override
      public E next() {
        skip();
        if (!it.hasNext()) {
          throw new Ex.NoSuchElement();
        }
        return it.next();
      }
    }, it);
  }

  public static <E> Function<Iterator<E>, Iterator<E>> take(int n) {
    return (it) -> take(it, n);
  }

  public static <E> Iterator<E> take(Iterator<E> it, int n) {
    return closeable(new Iterator<E>() {
      long _n = n;

      @Override
      public boolean hasNext() {
        return _n > 0 && it.hasNext();
      }

      @Override
      public E next() {
        if (!hasNext()) {
          throw new NoSuchElementException();
        }
        _n--;
        return it.next();
      }
    }, it);
  }

  @SuppressWarnings({"unchecked", "rawtypes"})
  public static <E> IPair<Iterator<E>, Iterator<E>> duplicate(Iterator<E> it) {
    final java.util.Queue<E> _gap = new java.util.LinkedList<>();
    final AtomicReference<Iterator<E>> _ahead = new AtomicReference<>();
    class PairIterator implements Iterator<E> {

      @Override
      public boolean hasNext() {
        return (this != _ahead.get() && !_gap.isEmpty()) || it.hasNext();
      }

      @Override
      public E next() {
        if (_gap.isEmpty()) {
          _ahead.set(this);
        }
        if (this == _ahead.get()) {
          final E element = it.next();
          _gap.add(element);
          return element;
        } else {
          return _gap.poll();
        }
      }
    }
    return new MapEntry<>(null, new PairIterator(), new PairIterator());
  }

  public static <E> Iterator<E> constantly(E t) {
    return new Iterator<E>() {
      @Override
      public boolean hasNext() {
        return true;
      }

      @Override
      public E next() {
        return t;
      }
    };
  }

  public static <E> Iterator<E> repeatedly(Supplier<E> f) {
    return new Iterator<E>() {
      @Override
      public boolean hasNext() {
        return true;
      }

      @Override
      public E next() {
        return f.get();
      }
    };
  }

  public static <E> Function<Iterator<E>, Boolean> every(Predicate<? super E> pred) {
    return it -> every(it, pred);
  }

  public static <E> boolean every(Iterator<E> it, Predicate<? super E> pred) {
    while (it.hasNext()) {
      if (!pred.test(it.next())) {
        return false;
      }
    }
    return true;
  }

  public static <E> Function<Iterator<E>, Boolean> any(Predicate<? super E> pred) {
    return it -> any(it, pred);
  }

  public static <E> boolean any(Iterator<E> it, Predicate<? super E> pred) {
    while (it.hasNext()) {
      if (pred.test(it.next())) {
        return true;
      }
    }
    return false;
  }

  public static <E> Function<Iterator<E>, E> some(Predicate<? super E> pred) {
    return it -> some(it, pred);
  }

  public static <E> E some(Iterator<E> it, Predicate<? super E> pred) {
    while (it.hasNext()) {
      var e = it.next();
      if (pred.test(e)) {
        return e;
      }
    }
    return null;
  }

  public static <E> Iterator<E> iterate(E seed, Function<? super E, ? extends E> f) {
    return new Iterator<E>() {
      E _curr = seed;

      @Override
      public boolean hasNext() {
        return true;
      }

      @Override
      public E next() {
        E n = f.apply(_curr);
        E p = _curr;
        _curr = n;
        return p;
      }
    };
  }

  public static <E> Iterator<E> cycle(Supplier<Iterator<E>> f) {
    return new CloseableIterator<E>() {
      Iterator<E> _it;
      final ArrayList<E> _cache = new ArrayList<>();
      int _index;
      boolean _exhausted;
      boolean _closed;

      private void initialize() {
        if (_it == null && !_closed) {
          _it = f.get();
        }
      }


      public boolean hasNext() {
        initialize();
        if (_closed) return false;
        if (!_exhausted && _it.hasNext()) return true;
        if (!_exhausted) {
          _exhausted = true;
          Iter.close(_it);
          _it = null;
        }
        return !_cache.isEmpty();
      }


      @Override
      public E next() {
        initialize();
        if (_closed) {
          throw new Ex.NoSuchElement();
        }
        if (!_exhausted && _it.hasNext()) {
          E value = _it.next();
          _cache.add(value);
          return value;
        }
        if (!_exhausted) {
          _exhausted = true;
          Iter.close(_it);
          _it = null;
        }
        if (_cache.isEmpty()) {
          close();
          throw new Ex.NoSuchElement();
        }
        E value = _cache.get(_index);
        _index = (_index + 1) % _cache.size();
        return value;
      }


      @Override
      public void close() {
        if (_closed) return;
        _closed = true;
        Iter.close(_it);
        _it = null;
      }
    };
  }

  public static <A, B> Iterator<IPair<A, B>> zipPair(Iterator<A> it0, Iterator<B> it1) {
    return new Iterator<IPair<A, B>>() {
      @Override
      public boolean hasNext() {
        return it0.hasNext() && it1.hasNext();
      }

      @SuppressWarnings({"rawtypes", "unchecked"})
      @Override
      public IPair<A, B> next() {
        if (!hasNext()) {
          throw new Ex.NoSuchElement();
        }
        return new MapEntry<>(null, it0.next(), it1.next());
      }
    };
  }

  @SuppressWarnings({"rawtypes"})
  public static Iterator<Object[]> zip(Iterator<Iterator<Object>> input) {
    return new CloseableIterator<Object[]>() {
      private Iterator[] its;
      private boolean initialized;
      private boolean closed;

      private void initialize() {
        if (!initialized && !closed) {
          ArrayList<Iterator> acquired = new ArrayList<>();
          try {
            while (input.hasNext()) {
              acquired.add(input.next());
            }
            its = acquired.toArray(new Iterator[0]);
            initialized = true;
          } catch (RuntimeException | Error failure) {
            for (Iterator iterator : acquired) Iter.close(iterator);
            Iter.close(input);
            closed = true;
            throw failure;
          }
        }
      }


      public boolean hasNext() {
        initialize();
        if (closed) return false;
        boolean available = Array.every((it) -> it.hasNext(), its);
        if (!available) close();
        return available;
      }


      public Object[] next() {
        if (!hasNext()) {
          throw new Ex.NoSuchElement();
        }
        return Array.map((it) -> it.next(), Object.class, its);
      }


      public void close() {
        if (closed) return;
        closed = true;
        if (its != null) {
          for (Iterator iterator : its) Iter.close(iterator);
          its = null;
        }
        Iter.close(input);
      }
    };
  }

  public static <E> Iterator<Entry<E, E>> partitionPair(Iterator<E> it) {
    return closeable(new Iterator<Entry<E, E>>() {
      E _v0;
      State _state = State.NOT_SET;

      private void prime() {
        if (_state != State.NOT_SET) {
          return;
        }
        if (!it.hasNext()) {
          _state = State.DONE;
          return;
        }
        _v0 = it.next();
        _state = it.hasNext() ? State.READY : State.DONE;
      }

      @Override
      public boolean hasNext() {
        prime();
        return _state == State.READY;
      }

      @SuppressWarnings({"unchecked", "rawtypes"})
      @Override
      public IPair<E, E> next() {
        prime();
        if (_state == State.READY) {
          var pair = new MapEntry<>(null, _v0, it.next());
          _state = State.NOT_SET;
          return pair;
        }
        throw new Ex.NoSuchElement();
      }
    }, it);
  }

  private static <E> Iterator<E> closeable(Iterator<E> iterator, Iterator<?>... sources) {
    return new CloseableIterator<E>() {
      private boolean closed;

      @Override
      public boolean hasNext() {
        if (closed) return false;
        boolean available = iterator.hasNext();
        if (!available) close();
        return available;
      }


      public E next() {
        if (!hasNext()) throw new Ex.NoSuchElement();
        return iterator.next();
      }


      public void close() {
        if (closed) return;
        closed = true;
        for (Iterator<?> source : sources) Iter.close(source);
      }
    };
  }

  /** Closes an iterator when its source exposes lifecycle semantics. */
  public static void close(Iterator<?> iterator) {
    if (iterator instanceof AutoCloseable) {
      try {
        ((AutoCloseable) iterator).close();
      } catch (Exception error) {
        throw Ex.Sneaky(error);
      }
    }
  }
}
