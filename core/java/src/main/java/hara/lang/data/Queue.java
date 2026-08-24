package hara.lang.data;

import hara.lang.base.Ex;
import hara.lang.base.Iter;
import hara.lang.base.primitive.Array;
import hara.lang.protocol.ILinearType;
import hara.lang.data.types.ISequentialLookupType;
import hara.lang.data.types.ObjMutable;
import hara.lang.data.types.ObjPersistent;
import hara.lang.protocol.IColl;
import hara.lang.protocol.IMetadata;
import hara.lang.protocol.IPopFirst;
import hara.lang.protocol.IPopLast;
import hara.lang.protocol.IPushFirst;
import hara.lang.protocol.IPushLast;
import hara.lang.protocol.IToMutable;
import hara.lang.protocol.IToPersistent;

import java.util.ArrayList;
import java.util.Iterator;

public interface Queue<E>
    extends IColl<E>,
        ILinearType<E>,
        ISequentialLookupType<E>,
        IPushFirst<E>,
        IPushLast<E>,
        IPopFirst,
        IPopLast {

  public static int MAX_LENGTH = 1024;

  public interface Base<E> extends Queue<E> {
    public int _size();

    public int _offset();

    public Vector<E> _head();

    public Vector<E> _tail();

    public List<Vector<E>> _buffer();

    @Override
    default E peekFirst() {
      if (_size() == 0) {
        return null;
      }
      return _head().nth(_offset());
    }

    @Override
    default E peekLast() {
      if (_size() == 0) {
        return null;
      }
      if (_tail().count() > 0) {
        return _tail().peekLast();
      } else if (_buffer().count() == 0) {
        return _head().peekLast();
      } else {
        return _buffer().peekLast().peekLast();
      }
    }

    @SuppressWarnings("unchecked")
    @Override
    default Iterator<E> iterator() {
      if (_size() == 0) {
        return (Iterator<E>) Iter.emptyIterator();
      }
      ArrayList<Iterator<E>> all = new ArrayList<>();
      all.add(Iter.drop(_head().iterator(), _offset()));
      Iter.reduce(
          _buffer().iterator(),
          all,
          (arr, v) -> {
            arr.add(v.iterator());
            return arr;
          });
      all.add(_tail().iterator());
      return Iter.concat(all.iterator());
    }

    @Override
    default E nth(long i) {
      if (i >= 0 && i < _size()) {
        var space = MAX_LENGTH - _offset();
        if (i < space) {
          return _head().nth(_offset() + i);
        } else {
          long j = i - space;
          int seg = (int) (j / MAX_LENGTH);
          int idx = (int) (j % MAX_LENGTH);

          if (seg < _buffer().count()) {
            return _buffer().nth(seg).nth(idx);
          } else {
            return _tail().nth(j - _buffer().count() * (long) MAX_LENGTH);
          }
        }
      } else {
        throw new Ex.OutOfBounds();
      }
    }

    @Override
    default long count() {
      return _size();
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

  public class Mutable<E> extends ObjMutable implements Base<E>, IToPersistent {
    int _size;
    int _offset;

    Vector<E> _head;
    Vector<E> _tail;
    List<Vector<E>> _buffer;

    @SuppressWarnings("unchecked")
    public Mutable(IMetadata meta) {
      super(meta);
      _size = 0;
      _offset = 0;
      _head = Vector.Mutable.empty(null);
      _tail = Vector.Mutable.empty(null);
      _buffer = new List.Mutable<Vector<E>>(null, List.DEFAULT_CAPACITY);
    }

    public Mutable(
        IMetadata meta,
        int size,
        int offset,
        Vector<E> head,
        Vector<E> tail,
        List<Vector<E>> buffer) {
      super(meta);
      _size = size;
      _offset = offset;

      _head = head;
      _tail = tail;
      _buffer = buffer;
    }

    @SuppressWarnings("unchecked")
    public static <E> Mutable<E> from(IMetadata meta, E... objs) {
      var vec = Mutable.empty(meta);
      return Array.reduce((v, e) -> v.pushLast(e), vec, objs);
    }

    @SuppressWarnings({"rawtypes"})
    public static Mutable empty(IMetadata meta) {
      return new Mutable(meta);
    }

    public static <E> Mutable<E> into(Iterator<E> it) {
      return into(new Mutable<E>(null), it);
    }

    public static <E> Mutable<E> into(Mutable<E> coll, Iterator<E> it) {
      return Iter.reduce(it, coll, (m, e) -> m.pushLast(e));
    }

    @SuppressWarnings({"unchecked"})
    @Override
    public Mutable<E> pushFirst(E e) {
      ArrayList<E> values = new ArrayList<>();
      iterator().forEachRemaining(values::add);
      _size = 0;
      _offset = 0;
      _head = Vector.Mutable.empty(null);
      _tail = Vector.Mutable.empty(null);
      _buffer = new List.Mutable<Vector<E>>(null, List.DEFAULT_CAPACITY);
      pushLast(e);
      values.forEach(this::pushLast);
      return this;
    }

    @SuppressWarnings({"unchecked"})
    @Override
    public Mutable<E> pushLast(E e) {
      var space = MAX_LENGTH - _offset;
      if (_size < space) {
        _head.conj(e);
      } else {
        _tail.conj(e);
        if (_tail.count() < MAX_LENGTH) {

        } else {
          _buffer.conj(_tail);
          _tail = Vector.Mutable.empty(null);
        }
      }
      _size++;
      return this;
    }

    @Override
    public Mutable<E> popFirst() {
      if (_size == 0) {
        return this;
      }

      _offset++;
      _size--;
      if (_offset < MAX_LENGTH) {
        return this;
      } else {
        _offset = 0;
        if (_buffer.count() == 0) {
          _head = _tail;
        } else {
          _head = _buffer.peekFirst();
          _buffer.popFirst();
        }
      }
      return this;
    }

    @SuppressWarnings({"unchecked", "rawtypes"})
    @Override
    public Mutable<E> popLast() {

      if (_size == 0) {
        return this;
      }

      _size--;
      if (_tail.count() > 0) {
        _tail.popLast();
      } else if (_buffer.count() == 0) {
        _head.popLast();
      } else {
        _tail = _buffer.peekLast();
        _buffer = (List) _buffer.popLast();
      }
      return this;
    }

    @SuppressWarnings({"unchecked"})
    @Override
    public Mutable<E> empty() {
      _size = 0;
      _offset = 0;
      _head = (Vector<E>) _head.empty();
      _tail = (Vector<E>) _tail.empty();
      _buffer = (List<Vector<E>>) _buffer.empty();
      return this;
    }

    @Override
    public int _size() {
      return _size;
    }

    @Override
    public int _offset() {
      return _offset;
    }

    @Override
    public Vector<E> _head() {
      return _head;
    }

    @Override
    public Vector<E> _tail() {
      return _tail;
    }

    @Override
    public List<Vector<E>> _buffer() {
      return _buffer;
    }

    @SuppressWarnings({"unchecked", "rawtypes"})
    @Override
    public Standard<E> toPersistent() {
      var head = ((Vector.Mutable<E>) _head).toPersistent();
      var tail = ((Vector.Mutable<E>) _tail).toPersistent();
      var buff = ((List.Mutable) _buffer).toPersistent();
      return new Standard<E>(_meta, _size, _offset, head, tail, buff);
    }
  }

  public class Standard<E> extends ObjPersistent implements Base<E>, IToMutable {
    final int _size;
    final int _offset;

    final Vector<E> _head;
    final Vector<E> _tail;
    final List<Vector<E>> _buffer;

    // STATIC
    public static final Standard<Object> EMPTY = new Standard<>(null);

    @SuppressWarnings("unchecked")
    public Standard(IMetadata meta) {
      super(meta);
      _size = 0;
      _offset = 0;
      _head = (Vector<E>) Vector.Standard.EMPTY;
      _tail = (Vector<E>) Vector.Standard.EMPTY;
      _buffer = List.Standard.EMPTY;
    }

    private Standard(
        IMetadata meta,
        int size,
        int offset,
        Vector<E> head,
        Vector<E> tail,
        List<Vector<E>> buffer) {
      super(meta);
      _size = size;
      _offset = offset;

      _head = head;
      _tail = tail;
      _buffer = buffer;
    }

    @SuppressWarnings("unchecked")
    public static <E> Standard<E> empty(IMetadata meta) {
      Standard<E> ret = (Standard<E>) EMPTY;
      return (meta == null) ? ret : ret.withMeta(meta);
    }

    @SuppressWarnings("unchecked")
    public static <E> Standard<E> from(IMetadata meta, E... objs) {
      return Mutable.from(meta, objs).toPersistent();
    }

    public static <E> Standard<E> into(Iterator<E> it) {
      return Mutable.into(it).toPersistent();
    }

    @SuppressWarnings("unchecked")
    public static <E> Standard<E> into(Standard<E> coll, Iterator<E> it) {
      return Mutable.into(coll.toMutable(), it).toPersistent();
    }

    @SuppressWarnings({"unchecked", "rawtypes"})
    @Override
    public Standard<E> pushFirst(E e) {
      Standard<E> output = Standard.<E>empty(_meta).pushLast(e);
      return into(output, iterator());
    }

    @SuppressWarnings({"unchecked", "rawtypes"})
    @Override
    public Standard<E> pushLast(E e) {
      var space = MAX_LENGTH - _offset;
      int nsize = _size + 1;
      if (_size < space) {
        var nhead = (Vector) _head.conj(e);
        return new Standard<E>(_meta, nsize, _offset, nhead, _tail, _buffer);
      } else {
        var ntail = (Vector<E>) _tail.conj(e);
        if (ntail.count() < MAX_LENGTH) {
          return new Standard<E>(_meta, nsize, _offset, _head, ntail, _buffer);
        } else {
          return new Standard<E>(
              _meta,
              nsize,
              _offset,
              _head,
              (Vector<E>) Vector.Standard.EMPTY,
              (List) _buffer.conj(ntail));
        }
      }
    }

    @SuppressWarnings({"unchecked", "rawtypes"})
    @Override
    public Standard<E> popFirst() {
      if (_size == 0) {
        return this;
      }
      var noffset = _offset + 1;
      if (noffset < MAX_LENGTH) {
        return new Standard<E>(_meta, _size - 1, noffset, _head, _tail, _buffer);
      } else {
        if (_buffer.count() == 0) {
          return new Standard<E>(
              _meta, _size - 1, 0, _tail, (Vector<E>) Vector.Standard.EMPTY, _buffer);
        } else {
          var nhead = _buffer.peekFirst();
          return new Standard<E>(_meta, _size - 1, 0, nhead, _tail, (List) _buffer.popFirst());
        }
      }
    }

    @SuppressWarnings({"unchecked", "rawtypes"})
    @Override
    public Standard<E> popLast() {

      if (_size == 0) {
        return this;
      }
      var nsize = _size - 1;
      if (_tail.count() > 0) {
        return new Standard<E>(_meta, nsize, _offset, _head, (Vector) _tail.popLast(), _buffer);
      } else if (_buffer.count() == 0) {
        return new Standard<E>(_meta, nsize, _offset, (Vector) _head.popLast(), _tail, _buffer);
      } else {
        var ntail = _buffer.peekLast();
        return new Standard<E>(_meta, _size - 1, _offset, _head, ntail, (List) _buffer.popLast());
      }
    }

    @Override
    public Standard<E> withMeta(IMetadata meta) {
      return (meta == _meta) ? this : new Standard<E>(meta, _size, _offset, _head, _tail, _buffer);
    }

    @SuppressWarnings({"rawtypes", "unchecked"})
    @Override
    public Standard<E> empty() {
      return new Standard(
          _meta, 0, 0, Vector.Standard.EMPTY, Vector.Standard.EMPTY, List.Standard.EMPTY);
    }

    @Override
    public int _size() {
      return _size;
    }

    @Override
    public int _offset() {
      return _offset;
    }

    @Override
    public Vector<E> _head() {
      return _head;
    }

    @Override
    public Vector<E> _tail() {
      return _tail;
    }

    @Override
    public List<Vector<E>> _buffer() {
      return _buffer;
    }

    @Override
    @SuppressWarnings({"rawtypes", "unchecked"})
    public Mutable toMutable() {
      var head = ((Vector.Standard<E>) _head).toMutable();
      var tail = ((Vector.Standard<E>) _tail).toMutable();
      var buff = ((List.Standard) _buffer).toMutable();
      return new Mutable<E>(_meta, _size, _offset, head, tail, buff);
    }
  }
}
