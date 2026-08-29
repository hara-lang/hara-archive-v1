package hara.truffle;

import com.oracle.truffle.api.CompilerDirectives.TruffleBoundary;
import com.oracle.truffle.api.interop.InteropLibrary;
import com.oracle.truffle.api.interop.InvalidArrayIndexException;
import com.oracle.truffle.api.interop.TruffleObject;
import com.oracle.truffle.api.interop.UnknownIdentifierException;
import com.oracle.truffle.api.library.ExportLibrary;
import com.oracle.truffle.api.library.ExportMessage;
import hara.lang.base.Eq;
import hara.lang.base.G;
import hara.lang.data.Keyword;
import hara.lang.data.Symbol;
import hara.lang.data.Tuple;
import hara.lang.protocol.Constant;
import hara.lang.protocol.IAssoc;
import hara.lang.protocol.ICount;
import hara.lang.protocol.IDissoc;
import hara.lang.protocol.IEmpty;
import hara.lang.protocol.IFind;
import hara.lang.protocol.ILookup;
import hara.lang.protocol.IMetadata;
import hara.lang.protocol.IObjType;
import java.util.Iterator;
import java.util.Map;

@ExportLibrary(InteropLibrary.class)
public final class HaraStruct
    implements TruffleObject,
        IObjType,
        ILookup<Object, Object>,
        IAssoc<Object, Object>,
        IFind<Object, Map.Entry<Object, Object>>,
        IDissoc<Object>,
        IEmpty,
        ICount,
        Iterable<Map.Entry<Object, Object>> {
  private final HaraType type;
  private final hara.lang.data.OrderedMap.Standard<Object, Object> values;

  public HaraStruct(HaraType type, Object[] orderedValues) {
    this(type, fromOrderedValues(type, orderedValues, null));
  }

  private HaraStruct(
      HaraType type, hara.lang.data.OrderedMap.Standard<Object, Object> values) {
    this.type = type;
    this.values = values;
  }

  private static hara.lang.data.OrderedMap.Standard<Object, Object> fromOrderedValues(
      HaraType type, Object[] orderedValues, IMetadata metadata) {
    String[] fields = type.fields();
    if (orderedValues.length != fields.length) {
      throw new IllegalArgumentException(
          "struct field/value arity mismatch: expected "
              + fields.length
              + ", got "
              + orderedValues.length);
    }
    Object[] entries = new Object[fields.length * 2];
    for (int index = 0; index < fields.length; index++) {
      entries[index * 2] = Keyword.create(fields[index]);
      entries[index * 2 + 1] = orderedValues[index];
    }
    return hara.lang.data.OrderedMap.Standard.from(metadata, entries);
  }

  public Object read(String field) throws UnknownIdentifierException {
    int index = type.fieldIndex(field);
    if (index < 0) {
      throw UnknownIdentifierException.create(field);
    }
    return values.lookup(Keyword.create(type.fields()[index]));
  }

  public HaraType type() {
    return type;
  }

  Object[] orderedValues() {
    String[] fields = type.fields();
    Object[] ordered = new Object[fields.length];
    for (int index = 0; index < fields.length; index++) {
      ordered[index] = values.lookup(Keyword.create(fields[index]));
    }
    return ordered;
  }

  hara.lang.data.OrderedMap.Standard<Object, Object> asMap() {
    return values;
  }

  @Override
  public IMetadata meta() {
    return values.meta();
  }

  @Override
  public HaraStruct withMeta(IMetadata metadata) {
    hara.lang.data.OrderedMap.Standard<Object, Object> updated = values.withMeta(metadata);
    return updated == values ? this : new HaraStruct(type, updated);
  }

  @Override
  public long hashCalc(Constant.HashType hashType) {
    long hash = 31L * System.identityHashCode(type);
    for (Object value : orderedValues()) {
      hash = 31L * hash + G.hashFn(hashType).apply(value);
    }
    return hash;
  }

  @Override
  public String display() {
    return toString();
  }

  @ExportMessage
  boolean hasMembers() {
    return true;
  }

  @ExportMessage
  Object getMembers(boolean includeInternal) {
    return new HaraMemberNames(type.fields());
  }

  @ExportMessage
  boolean isMemberReadable(String member) {
    return type.fieldIndex(member) >= 0;
  }

  @ExportMessage
  Object readMember(String member) throws UnknownIdentifierException {
    return HaraBox.export(read(member));
  }

  @Override
  public boolean equals(Object other) {
    if (!(other instanceof HaraStruct struct) || type != struct.type) {
      return false;
    }
    Object[] left = orderedValues();
    Object[] right = struct.orderedValues();
    for (int index = 0; index < left.length; index++) {
      if (!Eq.eq(left[index], right[index])) {
        return false;
      }
    }
    return true;
  }

  @Override
  public int hashCode() {
    return Long.hashCode(hashCalc(Constant.HashType.RAPID));
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
          .append(values.lookup(Keyword.create(fields[index])));
    }
    return result.append(">").toString();
  }

  @Override
  public Map.Entry<Object, Object> find(Object key) {
    Keyword canonical = canonicalKey(key);
    if (canonical == null) {
      return null;
    }
    return new hara.lang.data.MapEntry<>(null, canonical, values.lookup(canonical));
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
    String[] fields = type.fields();
    return new Iterator<Object>() {
      private int index;

      @Override
      public boolean hasNext() {
        return index < fields.length;
      }

      @Override
      public Object next() {
        return values.lookup(Keyword.create(fields[index++]));
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
        return index < fields.length;
      }

      @Override
      public Map.Entry<Object, Object> next() {
        Keyword key = Keyword.create(fields[index++]);
        return new hara.lang.data.MapEntry<>(null, key, values.lookup(key));
      }
    };
  }

  @Override
  public HaraStruct assoc(Object key, Object value) {
    Keyword canonical = canonicalKey(key);
    if (canonical == null) {
      throw new HaraException("unknown struct field: " + fieldName(key));
    }
    return new HaraStruct(type, values.assoc(canonical, value));
  }

  @Override
  public IDissoc<Object> dissoc(Object key) {
    Keyword canonical = canonicalKey(key);
    return canonical == null ? this : values.dissoc(canonical);
  }

  @Override
  public HaraStruct empty() {
    return new HaraStruct(type, new Object[type.arity()]).withMeta(meta());
  }

  @Override
  public long count() {
    return type.arity();
  }

  private Keyword canonicalKey(Object key) {
    int index = indexOfKey(key);
    return index < 0 ? null : Keyword.create(type.fields()[index]);
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

  private static String fieldName(Object key) {
    if (key instanceof Keyword keyword) {
      return keyword.getName();
    }
    if (key instanceof Symbol symbol) {
      return symbol.getName();
    }
    return String.valueOf(key);
  }

  @ExportLibrary(InteropLibrary.class)
  static final class HaraMemberNames implements TruffleObject {
    private final String[] names;

    HaraMemberNames(String[] names) {
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
