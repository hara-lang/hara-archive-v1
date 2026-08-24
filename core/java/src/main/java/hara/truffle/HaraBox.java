package hara.truffle;

import com.oracle.truffle.api.CompilerDirectives.TruffleBoundary;
import com.oracle.truffle.api.interop.ArityException;
import com.oracle.truffle.api.interop.InteropLibrary;
import com.oracle.truffle.api.interop.InvalidArrayIndexException;
import com.oracle.truffle.api.interop.StopIterationException;
import com.oracle.truffle.api.interop.TruffleObject;
import com.oracle.truffle.api.interop.UnknownKeyException;
import com.oracle.truffle.api.interop.UnsupportedMessageException;
import com.oracle.truffle.api.interop.UnsupportedTypeException;
import com.oracle.truffle.api.library.ExportLibrary;
import com.oracle.truffle.api.library.ExportMessage;
import hara.lang.protocol.ILinearType;
import hara.lang.protocol.ISetType;
import hara.lang.base.Eq;
import hara.lang.base.NumUtils;
import hara.lang.base.primitive.Cast;
import hara.lang.protocol.ICount;
import hara.lang.protocol.IFind;
import hara.lang.protocol.IFn;
import hara.lang.protocol.ILookup;
import hara.lang.protocol.IDisplay;
import hara.lang.protocol.IEquality;
import java.lang.reflect.Array;
import java.math.BigDecimal;
import java.math.BigInteger;
import java.util.Iterator;
import java.util.List;
import java.util.Map;
import java.util.function.Function;

/** Truffle boundary value for Java-backed Hara values. */
@ExportLibrary(InteropLibrary.class)
public final class HaraBox implements TruffleObject, IEquality {
  private final Object value;
  private final String display;

  public HaraBox(Object value) {
    this.value = canonicalValue(value);
    this.display = display(this.value);
  }

  public static Object unwrap(Object value) {
    if (value instanceof HaraBox) {
      return ((HaraBox) value).value;
    }
    if (value instanceof HaraBigInteger) {
      return ((HaraBigInteger) value).value();
    }
    return value;
  }

  public static boolean isNil(Object value) {
    Object unwrapped = unwrap(value);
    return unwrapped == null || unwrapped == HaraNull.SINGLETON;
  }

  public static Object export(Object value) {
    if (value == null) {
      return HaraNull.SINGLETON;
    }
    if (value instanceof BigDecimal) {
      return exportBigDecimal((BigDecimal) value);
    }
    if (value instanceof BigInteger) {
      return exportBigInteger((BigInteger) value);
    }
    if (value instanceof Byte
        || value instanceof Short
        || value instanceof Integer
        || value instanceof Long) {
      return Long.valueOf(((Number) value).longValue());
    }
    if (value instanceof Double
        || value instanceof Float
        || value instanceof Boolean
        || value instanceof String
        || value instanceof TruffleObject) {
      if (value instanceof Double || value instanceof Float) {
        HaraNumericConversions.requireFinite(((Number) value).doubleValue());
      }
      return value;
    }
    return new HaraBox(value);
  }

  @Override
  public boolean equality(Object other) {
    return Eq.eq(value, unwrap(other));
  }

  @TruffleBoundary
  private static Object exportBigDecimal(BigDecimal value) {
    double converted = value.doubleValue();
    if (!Double.isFinite(converted)) {
      throw new IllegalArgumentException(
          "BigDecimal value is outside the finite Hara Float range");
    }
    return converted;
  }

  @TruffleBoundary
  private static Object exportBigInteger(BigInteger value) {
    return NumUtils.normalizeInteger(value) instanceof Long normalized
        ? normalized
        : new HaraBigInteger(value);
  }

  private static Object canonicalValue(Object value) {
    if (value instanceof Byte
        || value instanceof Short
        || value instanceof Integer
        || value instanceof Long
        || value instanceof BigInteger) {
      return NumUtils.normalizeInteger(NumUtils.toBigInteger(value));
    }
    if (value instanceof HaraBigInteger) {
      return ((HaraBigInteger) value).value();
    }
    if (value instanceof Double || value instanceof Float) {
      HaraNumericConversions.requireFinite(((Number) value).doubleValue());
    }
    return value;
  }

  @ExportMessage
  Object toDisplayString(boolean allowSideEffects) {
    return display;
  }

  @ExportMessage
  boolean hasArrayElements() {
    return arraySize(value) >= 0;
  }

  @ExportMessage
  long getArraySize() throws UnsupportedMessageException {
    long size = arraySize(value);
    if (size < 0) {
      throw UnsupportedMessageException.create();
    }
    return size;
  }

  @ExportMessage
  boolean isArrayElementReadable(long index) {
    return index >= 0 && index < arraySize(value);
  }

  @ExportMessage
  Object readArrayElement(long index) throws InvalidArrayIndexException {
    if (!isArrayElementReadable(index)) {
      throw InvalidArrayIndexException.create(index);
    }
    return export(arrayElement(value, index));
  }

  @ExportMessage
  @TruffleBoundary
  boolean isArrayElementModifiable(long index) {
    return (value instanceof byte[] && index >= 0 && index < ((byte[]) value).length)
        || (value instanceof List<?> && index >= 0 && index < ((List<?>) value).size());
  }

  @ExportMessage
  boolean isArrayElementInsertable(long index) {
    return false;
  }

  @ExportMessage
  @TruffleBoundary
  void writeArrayElement(long index, Object newValue)
      throws InvalidArrayIndexException, UnsupportedTypeException {
    if (!isArrayElementModifiable(index)) {
      throw InvalidArrayIndexException.create(index);
    }
    try {
      if (value instanceof byte[]) {
        ((byte[]) value)[(int) index] = Cast.byteCast(unwrap(newValue));
      } else {
        ((List<Object>) value).set((int) index, unwrap(newValue));
      }
    } catch (IllegalArgumentException error) {
      throw UnsupportedTypeException.create(new Object[] {newValue}, "expected a signed byte");
    }
  }

  @ExportMessage
  boolean isExecutable() {
    return value instanceof IFn;
  }

  @ExportMessage
  @TruffleBoundary
  Object execute(Object[] arguments)
      throws UnsupportedTypeException, ArityException, UnsupportedMessageException {
    if (!(value instanceof IFn)) {
      throw UnsupportedMessageException.create();
    }
    return export(HaraFunctionDispatch.invoke(value, arguments));
  }

  @ExportMessage
  boolean hasIterator() {
    return value instanceof Iterator || value instanceof Iterable;
  }

  @ExportMessage
  @TruffleBoundary
  Object getIterator() throws UnsupportedMessageException {
    if (value instanceof Iterator) {
      return new HaraIterator((Iterator<?>) value, Function.identity());
    }
    if (value instanceof Iterable) {
      return new HaraIterator(((Iterable<?>) value).iterator(), Function.identity());
    }
    throw UnsupportedMessageException.create();
  }

  @ExportMessage
  boolean hasHashEntries() {
    return value instanceof Map || value instanceof ILookup || value instanceof ISetType;
  }

  @ExportMessage
  @TruffleBoundary
  long getHashSize() throws UnsupportedMessageException {
    if (value instanceof Map) {
      return ((Map<?, ?>) value).size();
    }
    if (value instanceof ICount) {
      return ((ICount) value).count();
    }
    throw UnsupportedMessageException.create();
  }

  @ExportMessage
  @TruffleBoundary
  boolean isHashEntryReadable(Object key) {
    if (value instanceof Map) {
      return ((Map<?, ?>) value).containsKey(key);
    }
    return hasHashEntry(value, key);
  }

  @ExportMessage
  @TruffleBoundary
  Object readHashValue(Object key) throws UnknownKeyException, UnsupportedMessageException {
    if (!isHashEntryReadable(key)) {
      if (!hasHashEntries()) {
        throw UnsupportedMessageException.create();
      }
      throw UnknownKeyException.create(key);
    }
    if (value instanceof Map) {
      return export(((Map<?, ?>) value).get(key));
    }
    if (value instanceof ISetType) {
      return export(key);
    }
    return export(lookupValue((ILookup<?, ?>) value, key));
  }

  @ExportMessage
  @TruffleBoundary
  boolean isHashEntryModifiable(Object key) {
    return value instanceof Map && ((Map<?, ?>) value).containsKey(key);
  }

  @ExportMessage
  @TruffleBoundary
  boolean isHashEntryInsertable(Object key) {
    return value instanceof Map && !((Map<?, ?>) value).containsKey(key);
  }

  @ExportMessage
  @TruffleBoundary
  boolean isHashEntryRemovable(Object key) {
    return value instanceof Map && ((Map<?, ?>) value).containsKey(key);
  }

  @ExportMessage
  @TruffleBoundary
  void writeHashEntry(Object key, Object newValue) throws UnsupportedMessageException {
    if (!(value instanceof Map)) {
      throw UnsupportedMessageException.create();
    }
    ((Map<Object, Object>) value).put(key, unwrap(newValue));
  }

  @ExportMessage
  @TruffleBoundary
  void removeHashEntry(Object key) throws UnknownKeyException, UnsupportedMessageException {
    if (!(value instanceof Map)) {
      throw UnsupportedMessageException.create();
    }
    if (!((Map<?, ?>) value).containsKey(key)) {
      throw UnknownKeyException.create(key);
    }
    ((Map<?, ?>) value).remove(key);
  }

  @ExportMessage
  @TruffleBoundary
  Object getHashEntriesIterator() throws UnsupportedMessageException {
    if (value instanceof Map) {
      Iterator<?> entries = ((Map<?, ?>) value).entrySet().iterator();
      return new HaraIterator(
          entries,
          entry -> {
            Map.Entry<?, ?> pair = (Map.Entry<?, ?>) entry;
            return new HaraHashEntry(pair.getKey(), pair.getValue());
          });
    }
    if (value instanceof ILookup) {
      ILookup<?, ?> lookup = (ILookup<?, ?>) value;
      return new HaraIterator(
          lookup.keys(), key -> new HaraHashEntry(key, lookupValue(lookup, key)));
    }
    if (value instanceof ISetType) {
      return new HaraIterator(((ISetType<?>) value).iterator(), key -> new HaraHashEntry(key, key));
    }
    throw UnsupportedMessageException.create();
  }

  @ExportMessage
  @TruffleBoundary
  Object getHashKeysIterator() throws UnsupportedMessageException {
    if (value instanceof Map) {
      return new HaraIterator(((Map<?, ?>) value).keySet().iterator(), Function.identity());
    }
    if (value instanceof ILookup) {
      return new HaraIterator(((ILookup<?, ?>) value).keys(), Function.identity());
    }
    if (value instanceof ISetType) {
      return new HaraIterator(((ISetType<?>) value).iterator(), Function.identity());
    }
    throw UnsupportedMessageException.create();
  }

  @ExportMessage
  @TruffleBoundary
  Object getHashValuesIterator() throws UnsupportedMessageException {
    if (value instanceof Map) {
      return new HaraIterator(((Map<?, ?>) value).values().iterator(), Function.identity());
    }
    if (value instanceof ILookup) {
      return new HaraIterator(((ILookup<?, ?>) value).vals(), Function.identity());
    }
    if (value instanceof ISetType) {
      return new HaraIterator(((ISetType<?>) value).iterator(), Function.identity());
    }
    throw UnsupportedMessageException.create();
  }

  @Override
  public String toString() {
    return display;
  }

  @TruffleBoundary
  private static String display(Object value) {
    if (value instanceof byte[]) return hara.lang.base.G.displayBytes((byte[]) value);
    if (value instanceof Character || value instanceof java.util.regex.Pattern) {
      return hara.lang.base.G.display(value);
    }
    if (value instanceof IDisplay) return ((IDisplay) value).display();
    if (value instanceof Iterator) return "#<lazy-iterator>";
    return String.valueOf(value);
  }

  @TruffleBoundary
  private static long arraySize(Object value) {
    if (value instanceof ILinearType) {
      return ((ILinearType<?>) value).count();
    }
    if (value instanceof List) {
      return ((List<?>) value).size();
    }
    return value != null && value.getClass().isArray() ? Array.getLength(value) : -1;
  }

  @TruffleBoundary
  private static Object arrayElement(Object value, long index) {
    if (value instanceof ILinearType) {
      return ((ILinearType<?>) value).nth(index);
    }
    if (value instanceof List) {
      return ((List<?>) value).get((int) index);
    }
    return Array.get(value, (int) index);
  }

  @TruffleBoundary
  @SuppressWarnings({"rawtypes", "unchecked"})
  private static Object lookupValue(ILookup<?, ?> lookup, Object key) {
    return ((ILookup) lookup).lookup(key);
  }

  @TruffleBoundary
  @SuppressWarnings({"rawtypes", "unchecked"})
  private static boolean hasHashEntry(Object value, Object key) {
    if (value instanceof ISetType) {
      return ((ISetType) value).find(key) != null;
    }
    if (value instanceof IFind) {
      return ((IFind) value).find(key) != null;
    }
    return false;
  }

  @ExportLibrary(InteropLibrary.class)
  static final class HaraIterator implements TruffleObject {
    private final Iterator<?> iterator;
    private final Function<Object, Object> mapper;

    HaraIterator(Iterator<?> iterator, Function<Object, Object> mapper) {
      this.iterator = iterator;
      this.mapper = mapper;
    }

    @ExportMessage
    boolean isIterator() {
      return true;
    }

    @ExportMessage
    @TruffleBoundary
    boolean hasIteratorNextElement() {
      return iterator.hasNext();
    }

    @ExportMessage
    @TruffleBoundary
    Object getIteratorNextElement() throws StopIterationException {
      if (!iterator.hasNext()) {
        throw StopIterationException.create();
      }
      return export(mapper.apply(iterator.next()));
    }
  }

  @ExportLibrary(InteropLibrary.class)
  static final class HaraHashEntry implements TruffleObject {
    private final Object key;
    private final Object value;

    HaraHashEntry(Object key, Object value) {
      this.key = key;
      this.value = value;
    }

    @ExportMessage
    boolean hasArrayElements() {
      return true;
    }

    @ExportMessage
    long getArraySize() {
      return 2;
    }

    @ExportMessage
    boolean isArrayElementReadable(long index) {
      return index == 0 || index == 1;
    }

    @ExportMessage
    Object readArrayElement(long index) throws InvalidArrayIndexException {
      if (index == 0) {
        return export(key);
      }
      if (index == 1) {
        return export(value);
      }
      throw InvalidArrayIndexException.create(index);
    }
  }
}
