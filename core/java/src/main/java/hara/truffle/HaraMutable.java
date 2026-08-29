package hara.truffle;

import com.oracle.truffle.api.CompilerDirectives.TruffleBoundary;
import com.oracle.truffle.api.interop.InteropLibrary;
import com.oracle.truffle.api.interop.InvalidArrayIndexException;
import com.oracle.truffle.api.interop.TruffleObject;
import com.oracle.truffle.api.interop.UnknownIdentifierException;
import com.oracle.truffle.api.library.ExportLibrary;
import com.oracle.truffle.api.library.ExportMessage;
import hara.lang.base.G;
import hara.lang.data.Keyword;
import hara.lang.data.Symbol;
import hara.lang.data.Tuple;
import hara.lang.protocol.Constant;
import hara.lang.protocol.ICount;
import hara.lang.protocol.IFind;
import hara.lang.protocol.ILookup;
import hara.lang.protocol.IMetadata;
import hara.lang.protocol.IObjType;
import java.util.Iterator;
import java.util.Map;

/** Fixed-shape mutable named value. Equality and hashing follow shared storage identity. */
@ExportLibrary(InteropLibrary.class)
public final class HaraMutable
    implements TruffleObject,
        IObjType,
        ILookup<Object, Object>,
        IFind<Object, Map.Entry<Object, Object>>,
        ICount,
        Iterable<Map.Entry<Object, Object>> {
  private static final class State {
    private final Object[] values;

    private State(Object[] values) {
      this.values = values.clone();
    }
  }

  private final HaraMutableType type;
  private final State state;
  private final IMetadata metadata;

  public HaraMutable(HaraMutableType type, Object[] values) {
    this(type, new State(values), null);
    if (values.length != type.arity()) {
      throw new IllegalArgumentException(
          "mutable field/value arity mismatch: expected "
              + type.arity()
              + ", got "
              + values.length);
    }
  }

  private HaraMutable(HaraMutableType type, State state, IMetadata metadata) {
    this.type = type;
    this.state = state;
    this.metadata = metadata;
  }

  public HaraMutableType type() {
    return type;
  }

  public Object read(String field) throws UnknownIdentifierException {
    int index = type.fieldIndex(field);
    if (index < 0) {
      throw UnknownIdentifierException.create(field);
    }
    return state.values[index];
  }

  public Object write(String field, Object replacement) throws UnknownIdentifierException {
    int index = type.fieldIndex(field);
    if (index < 0) {
      throw UnknownIdentifierException.create(field);
    }
    state.values[index] = replacement;
    return replacement;
  }

  Object[] orderedValues() {
    return state.values.clone();
  }

  hara.lang.data.Map.Standard<Object, Object> asMap() {
    String[] fields = type.fields();
    Object[] entries = new Object[fields.length * 2];
    for (int index = 0; index < fields.length; index++) {
      entries[index * 2] = Keyword.create(fields[index]);
      entries[index * 2 + 1] = state.values[index];
    }
    return hara.lang.data.Map.Standard.from(metadata, entries);
  }

  @Override
  public IMetadata meta() {
    return metadata;
  }

  @Override
  public HaraMutable withMeta(IMetadata metadata) {
    return this.metadata == metadata ? this : new HaraMutable(type, state, metadata);
  }

  @Override
  public long hashCalc(Constant.HashType hashType) {
    return System.identityHashCode(state);
  }

  @Override
  public String display() {
    return toString();
  }

  @Override
  public boolean equals(Object other) {
    return other instanceof HaraMutable mutable && state == mutable.state;
  }

  @Override
  public int hashCode() {
    return System.identityHashCode(state);
  }

  @Override
  @TruffleBoundary
  public String toString() {
    String[] fields = type.fields();
    StringBuilder result = new StringBuilder("#<").append(type.name());
    for (int index = 0; index < fields.length; index++) {
      result
          .append(index == 0 ? " " : ", ")
          .append(fields[index])
          .append("=")
          .append(G.display(state.values[index]));
    }
    return result.append(">").toString();
  }

  @Override
  public Map.Entry<Object, Object> find(Object key) {
    int index = indexOfKey(key);
    if (index < 0) {
      return null;
    }
    Keyword canonical = Keyword.create(type.fields()[index]);
    return new hara.lang.data.MapEntry<>(null, canonical, state.values[index]);
  }

  @Override
  public Iterator<Object> keys() {
    String[] fields = type.fields();
    return new Iterator<Object>() {
      private int index;

      @Override
      public boolean hasNext() {
        return index < fields.length;
      }

      @Override
      public Object next() {
        return Keyword.create(fields[index++]);
      }
    };
  }

  @Override
  public Iterator<Object> vals() {
    return new Iterator<Object>() {
      private int index;

      @Override
      public boolean hasNext() {
        return index < state.values.length;
      }

      @Override
      public Object next() {
        return state.values[index++];
      }
    };
  }

  @Override
  public Iterator<Map.Entry<Object, Object>> iterator() {
    String[] fields = type.fields();
    return new Iterator<Map.Entry<Object, Object>>() {
      private int index;

      @Override
      public boolean hasNext() {
        return index < state.values.length;
      }

      @Override
      public Map.Entry<Object, Object> next() {
        int current = index++;
        return new hara.lang.data.MapEntry<>(
            null, Keyword.create(fields[current]), state.values[current]);
      }
    };
  }

  @Override
  public long count() {
    return state.values.length;
  }

  private int indexOfKey(Object key) {
    if (key instanceof Keyword keyword) {
      if (keyword.getNamespace() != null) {
        return -1;
      }
      return type.fieldIndex(keyword.getName());
    }
    if (key instanceof Symbol symbol) {
      if (symbol.getNamespace() != null) {
        return -1;
      }
      return type.fieldIndex(symbol.getName());
    }
    if (key instanceof String string) {
      return type.fieldIndex(string);
    }
    return -1;
  }

  @ExportMessage
  boolean hasMembers() {
    return true;
  }

  @ExportMessage
  Object getMembers(boolean includeInternal) {
    return new MemberNames(type.fields());
  }

  @ExportMessage
  boolean isMemberReadable(String member) {
    return type.fieldIndex(member) >= 0;
  }

  @ExportMessage
  boolean isMemberModifiable(String member) {
    return type.fieldIndex(member) >= 0;
  }

  @ExportMessage
  boolean isMemberInsertable(String member) {
    return false;
  }

  @ExportMessage
  Object readMember(String member) throws UnknownIdentifierException {
    return HaraBox.export(read(member));
  }

  @ExportMessage
  void writeMember(String member, Object replacement) throws UnknownIdentifierException {
    write(member, HaraBox.unwrap(replacement));
  }

  @ExportLibrary(InteropLibrary.class)
  static final class MemberNames implements TruffleObject {
    private final String[] names;

    MemberNames(String[] names) {
      this.names = names;
    }

    @ExportMessage
    boolean hasArrayElements() {
      return true;
    }

    @ExportMessage
    long getArraySize() {
      return names.length;
    }

    @ExportMessage
    boolean isArrayElementReadable(long index) {
      return index >= 0 && index < names.length;
    }

    @ExportMessage
    Object readArrayElement(long index) throws InvalidArrayIndexException {
      if (!isArrayElementReadable(index)) {
        throw InvalidArrayIndexException.create(index);
      }
      return names[(int) index];
    }
  }
}
