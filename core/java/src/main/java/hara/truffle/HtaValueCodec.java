package hara.truffle;

import hara.lang.data.Keyword;
import hara.lang.data.MapEntry;
import hara.lang.data.Symbol;
import hara.lang.data.HaraCharacter;
import hara.lang.data.Tuple;
import hara.lang.base.NumUtils;
import hara.lang.protocol.ILinearType;
import hara.lang.protocol.IMapType;
import hara.lang.protocol.ISetType;
import java.io.ByteArrayOutputStream;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.math.BigInteger;
import java.util.regex.Pattern;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collection;
import java.util.Iterator;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;

/** Canonical, dependency-free value encoding used by HTA v1. */
public final class HtaValueCodec {
  private static final byte[] MAGIC = {'H', 'T', 'A', '0'};
  private static final int MAX_FRAME_BYTES = 64 * 1024 * 1024;
  private static final int MAX_NESTING_DEPTH = 256;
  private static final int NIL = 0;
  private static final int FALSE = 1;
  private static final int TRUE = 2;
  private static final int I64 = 3;
  private static final int STRING = 4;
  private static final int BYTES = 5;
  private static final int KEYWORD = 6;
  private static final int SYMBOL = 7;
  private static final int LIST = 8;
  private static final int VECTOR = 9;
  private static final int SET = 10;
  private static final int MAP = 11;
  private static final int HANDLE = 12;
  private static final int F64 = 15;
  private static final int CHARACTER = 19;
  private static final int BIG_INTEGER = 20;
  private static final int REGEX = 22;
  private static final int TUPLE = 23;
  private static final int CONS = 24;
  private static final int QUEUE = 25;
  private static final int ORDERED_MAP = 26;
  private static final int SORTED_MAP = 27;
  private static final int TRIE = 28;
  private static final int ORDERED_SET = 29;
  private static final int SORTED_SET = 30;
  private static final int TAGGED = 31;
  private static final int EXCEPTION_INFO = 32;
  private static final int STRUCT = 33;
  private static final int POINTER = 34;
  private static final int VAR_REF = 35;
  private static final int DEQUE = 36;
  private static final int PRIORITY_MAP = 37;
  private static final int MAP_ENTRY = 38;
  private static final String RESULT_STRUCT_NAME = "std.native/Result";
  private static final String[] RESULT_STRUCT_FIELDS = {"status", "data", "error", "context"};

  private HtaValueCodec() {}

  public static byte[] encode(Object value) {
    ByteArrayOutputStream output = new ByteArrayOutputStream();
    output.writeBytes(MAGIC);
    write(output, HaraBox.unwrap(value), 0);
    return output.toByteArray();
  }

  public static Object decode(byte[] bytes) {
    return decode(bytes, false);
  }

  /**
   * Decodes a canonical frame and materializes Hara persistent collections for
   * HBC0 constants.
   *
   * <p>The ordinary decoder remains permissive for trusted legacy state. Wire
   * and artifact boundaries must use this method so alternate BigInteger text,
   * map/set ordering, and other noncanonical representations are rejected.
   */
  public static Object decodeCanonical(byte[] bytes) {
    Object value = decode(bytes, true);
    if (!Arrays.equals(bytes, encode(value))) {
      throw noncanonical();
    }
    return value;
  }

  private static Object decode(byte[] bytes, boolean canonicalCollections) {
    if (bytes.length > MAX_FRAME_BYTES) throw malformed("frame too large");
    if (bytes.length < MAGIC.length) throw malformed("missing HTA0 header");
    for (int i = 0; i < MAGIC.length; i++) {
      if (bytes[i] != MAGIC[i]) throw malformed("invalid HTA0 header");
    }
    Reader reader = new Reader(bytes, MAGIC.length, canonicalCollections);
    Object value = reader.read(0);
    if (reader.remaining() != 0) throw malformed("trailing bytes");
    return value;
  }

  private static void write(ByteArrayOutputStream output, Object value, int depth) {
    if (depth > MAX_NESTING_DEPTH) throw malformed("nesting depth exceeded");
    if (value == null || value == HaraNull.SINGLETON) {
      output.write(NIL);
    } else if (value instanceof Boolean) {
      output.write((Boolean) value ? TRUE : FALSE);
    } else if (value instanceof Byte
        || value instanceof Short
        || value instanceof Integer
        || value instanceof Long) {
      output.write(I64);
      writeLong(output, ((Number) value).longValue());
    } else if (value instanceof Float || value instanceof Double) {
      HaraNumericConversions.requireFinite(((Number) value).doubleValue());
      output.write(F64);
      writeLong(output, Double.doubleToRawLongBits(((Number) value).doubleValue()));
    } else if (value instanceof HaraCharacter character) {
      output.write(CHARACTER);
      writeInt(output, character.codePoint());
    } else if (value instanceof Character) {
      output.write(CHARACTER);
      writeInt(output, (Character) value);
    } else if (value instanceof BigInteger) {
      Number normalized = NumUtils.normalizeInteger((BigInteger) value);
      if (normalized instanceof Long integer) {
        output.write(I64);
        writeLong(output, integer);
      } else {
        output.write(BIG_INTEGER);
        writeText(output, normalized.toString());
      }
    } else if (value instanceof Pattern) {
      output.write(REGEX);
      writeText(output, ((Pattern) value).pattern());
    } else if (value instanceof String) {
      output.write(STRING);
      writeBytes(output, ((String) value).getBytes(StandardCharsets.UTF_8));
    } else if (value instanceof byte[]) {
      output.write(BYTES);
      writeBytes(output, (byte[]) value);
    } else if (value instanceof Keyword) {
      Keyword keyword = (Keyword) value;
      output.write(KEYWORD);
      writeText(output, qualified(keyword.getNamespace(), keyword.getName()));
    } else if (value instanceof Symbol) {
      Symbol symbol = (Symbol) value;
      output.write(SYMBOL);
      writeText(output, qualified(symbol.getNamespace(), symbol.getName()));
    } else if (value instanceof HaraVar variable) {
      output.write(VAR_REF);
      write(
          output,
          Symbol.create(variable.namespaceName(), variable.symbolName()),
          depth + 1);
    } else if (value instanceof HtaHandle) {
      HtaHandle handle = (HtaHandle) value;
      if (handle.released()) throw new HaraException("hta/handle-released: " + handle);
      output.write(HANDLE);
      writeText(output, handle.owner());
      writeText(output, handle.type());
      writeLong(output, handle.id());
    } else if (value instanceof HaraResult result) {
      output.write(STRUCT);
      write(output, RESULT_STRUCT_NAME, depth + 1);
      output.write(VECTOR);
      writeCollection(output, java.util.List.of(RESULT_STRUCT_FIELDS), depth + 1);
      output.write(VECTOR);
      writeCollection(
          output,
          java.util.Arrays.asList(
              result.status(), result.data(), result.errorValue(), result.transportContext()),
          depth + 1);
    } else if (value instanceof HaraMutable || value instanceof HaraMutableType) {
      throw new HaraException(
          "hta/value-unsupported: mutable values are not serializable; use (into {} value)");
    } else if (value instanceof HaraStruct struct) {
      output.write(STRUCT);
      write(output, struct.type().name(), depth + 1);
      output.write(VECTOR);
      writeCollection(output, java.util.Arrays.asList(struct.type().fields()), depth + 1);
      output.write(VECTOR);
      writeCollection(output, java.util.Arrays.asList(struct.orderedValues()), depth + 1);
    } else if (value instanceof hara.lang.data.Pointer pointer) {
      output.write(POINTER);
      write(output, pointer.context(), depth + 1);
      writeMap(output, pointer.values().entrySet().iterator(), depth + 1);
    } else if (value instanceof hara.lang.data.TaggedLiteral tagged) {
      output.write(TAGGED);
      write(output, tagged.tag(), depth + 1);
      write(output, tagged.form(), depth + 1);
    } else if (value instanceof MapEntry<?, ?> entry) {
      output.write(MAP_ENTRY);
      writeCollection(output, entry, depth);
    } else if (value instanceof hara.lang.base.Ex.Info info) {
      output.write(EXCEPTION_INFO);
      write(output, info.getMessage(), depth + 1);
      write(output, info.getData(), depth + 1);
      write(output, info.getCause(), depth + 1);
      write(output, exceptionProvenance(info), depth + 1);
    } else if (value instanceof hara.lang.data.PriorityMap<?, ?>) {
      writeMap(output, ((IMapType<?, ?>) value).iterator(), depth, PRIORITY_MAP, false);
    } else if (value instanceof hara.lang.data.OrderedMap<?, ?>) {
      writeMap(output, ((IMapType<?, ?>) value).iterator(), depth, ORDERED_MAP, false);
    } else if (value instanceof hara.lang.data.SortedMap<?, ?>) {
      writeMap(output, ((IMapType<?, ?>) value).iterator(), depth, SORTED_MAP, false);
    } else if (value instanceof hara.lang.data.Trie<?>) {
      writeMap(output, ((IMapType<?, ?>) value).iterator(), depth, TRIE, false);
    } else if (value instanceof IMapType<?, ?>) {
      writeMap(output, ((IMapType<?, ?>) value).iterator(), depth);
    } else if (value instanceof Map<?, ?>) {
      writeMap(output, ((Map<?, ?>) value).entrySet().iterator(), depth);
    } else if (value instanceof hara.lang.data.OrderedSet<?>) {
      writeSet(output, ((ISetType<?>) value).iterator(), depth, ORDERED_SET, false);
    } else if (value instanceof hara.lang.data.SortedSet<?>) {
      writeSet(output, ((ISetType<?>) value).iterator(), depth, SORTED_SET, true);
    } else if (value instanceof ISetType<?>) {
      writeSet(output, ((ISetType<?>) value).iterator(), depth);
    } else if (value instanceof java.util.Set<?>) {
      writeSet(output, ((java.util.Set<?>) value).iterator(), depth);
    } else if (value instanceof hara.lang.data.List<?>) {
      output.write(LIST);
      writeCollection(output, (ILinearType<?>) value, depth);
    } else if (Tuple.isCompact(value)) {
      output.write(VECTOR);
      writeCollection(output, (ILinearType<?>) value, depth);
    } else if (value instanceof hara.lang.data.Cons<?>) {
      output.write(CONS);
      writeCollection(output, (ILinearType<?>) value, depth);
    } else if (value instanceof hara.lang.data.Queue<?>) {
      output.write(QUEUE);
      writeCollection(output, (ILinearType<?>) value, depth);
    } else if (value instanceof hara.lang.data.Deque<?>) {
      output.write(DEQUE);
      writeCollection(output, (ILinearType<?>) value, depth);
    } else if (value instanceof ILinearType<?>) {
      output.write(VECTOR);
      writeCollection(output, (ILinearType<?>) value, depth);
    } else if (value instanceof List<?>) {
      output.write(VECTOR);
      writeCollection(output, (List<?>) value, depth);
    } else if (value instanceof Collection<?>) {
      output.write(LIST);
      writeCollection(output, (Collection<?>) value, depth);
    } else {
      throw new HaraException("hta/value-unsupported: " + value.getClass().getName());
    }
  }

  private static void writeSet(ByteArrayOutputStream output, Iterator<?> iterator, int depth) {
    writeSet(output, iterator, depth, SET, true);
  }

  private static void writeSet(
      ByteArrayOutputStream output, Iterator<?> iterator, int depth, int tag, boolean sort) {
    ArrayList<byte[]> encoded = new ArrayList<>();
    iterator.forEachRemaining(value -> encoded.add(encodeBare(value, depth + 1)));
    if (sort) encoded.sort(HtaValueCodec::compareUnsigned);
    output.write(tag);
    writeInt(output, encoded.size());
    encoded.forEach(value -> writeRaw(output, value));
  }

  private static void writeMap(ByteArrayOutputStream output, Iterator<?> iterator, int depth) {
    writeMap(output, iterator, depth, MAP, true);
  }

  private static void writeMap(
      ByteArrayOutputStream output, Iterator<?> iterator, int depth, int tag, boolean sort) {
    ArrayList<Map.Entry<byte[], byte[]>> encoded = new ArrayList<>();
    iterator.forEachRemaining(
        item -> {
          Map.Entry<?, ?> entry = (Map.Entry<?, ?>) item;
          encoded.add(
              Map.entry(encodeBare(entry.getKey(), depth + 1), encodeBare(entry.getValue(), depth + 1)));
        });
    if (sort) encoded.sort((left, right) -> compareUnsigned(left.getKey(), right.getKey()));
    output.write(tag);
    writeInt(output, encoded.size());
    encoded.forEach(
        entry -> {
          writeRaw(output, entry.getKey());
          writeRaw(output, entry.getValue());
        });
  }

  private static byte[] encodeBare(Object value, int depth) {
    ByteArrayOutputStream output = new ByteArrayOutputStream();
    write(output, HaraBox.unwrap(value), depth);
    return output.toByteArray();
  }

  private static void writeCollection(ByteArrayOutputStream output, Iterable<?> values, int depth) {
    ArrayList<Object> copy = new ArrayList<>();
    values.forEach(copy::add);
    writeInt(output, copy.size());
    copy.forEach(value -> write(output, HaraBox.unwrap(value), depth + 1));
  }

  private static int compareUnsigned(byte[] left, byte[] right) {
    return java.util.Arrays.compareUnsigned(left, right);
  }

  private static String qualified(String namespace, String name) {
    return namespace == null ? name : namespace + "/" + name;
  }

  private static void writeText(ByteArrayOutputStream output, String value) {
    writeBytes(output, value.getBytes(StandardCharsets.UTF_8));
  }

  private static void writeBytes(ByteArrayOutputStream output, byte[] value) {
    if (value.length > MAX_FRAME_BYTES - output.size() - Integer.BYTES) {
      throw malformed("frame too large");
    }
    writeInt(output, value.length);
    output.writeBytes(value);
  }

  private static void writeRaw(ByteArrayOutputStream output, byte[] value) {
    if (value.length > MAX_FRAME_BYTES - output.size()) throw malformed("frame too large");
    output.writeBytes(value);
  }

  private static void writeInt(ByteArrayOutputStream output, int value) {
    output.write((value >>> 24) & 0xff);
    output.write((value >>> 16) & 0xff);
    output.write((value >>> 8) & 0xff);
    output.write(value & 0xff);
  }

  private static void writeLong(ByteArrayOutputStream output, long value) {
    for (int shift = 56; shift >= 0; shift -= 8) output.write((int) (value >>> shift) & 0xff);
  }

  private static HaraException malformed(String message) {
    return new HaraException("hta/value-malformed: " + message);
  }

  private static HaraException noncanonical() {
    return new HaraException("hta/value-noncanonical: frame bytes are not canonical");
  }

  private static Map<Object, Object> exceptionProvenance(hara.lang.base.Ex.Info info) {
    Map<Object, Object> provenance = new LinkedHashMap<>();
    provenance.put(Keyword.create("ex/created-at"), siteValue(info.createdAt()));
    List<Object> throwsAt = new ArrayList<>();
    for (hara.lang.base.Ex.Info.Site site : info.throwsAt()) throwsAt.add(siteValue(site));
    provenance.put(Keyword.create("ex/throws"), throwsAt);
    return provenance;
  }

  private static Object siteValue(hara.lang.base.Ex.Info.Site site) {
    if (site == null) return null;
    Map<Object, Object> value = new LinkedHashMap<>();
    value.put(Keyword.create("namespace"), site.namespace());
    value.put(Keyword.create("resource"), site.resource());
    value.put(Keyword.create("line"), site.line());
    value.put(Keyword.create("column"), site.column());
    return value;
  }

  private static final class Reader {
    private final ByteBuffer input;
    private final boolean canonicalCollections;

    private Reader(byte[] bytes, int offset, boolean canonicalCollections) {
      input =
          ByteBuffer.wrap(bytes, offset, bytes.length - offset).slice().order(ByteOrder.BIG_ENDIAN);
      this.canonicalCollections = canonicalCollections;
    }

    private int remaining() {
      return input.remaining();
    }

    private Object read(int depth) {
      if (depth > MAX_NESTING_DEPTH) throw malformed("nesting depth exceeded");
      require(1);
      int tag = Byte.toUnsignedInt(input.get());
      switch (tag) {
        case NIL:
          return HaraNull.SINGLETON;
        case FALSE:
          return false;
        case TRUE:
          return true;
        case I64:
          require(8);
          return input.getLong();
        case F64:
          require(8);
          return HaraNumericConversions.requireFinite(Double.longBitsToDouble(input.getLong()));
        case CHARACTER:
          require(4);
          int codePoint = input.getInt();
          if (!Character.isValidCodePoint(codePoint)
              || (codePoint >= Character.MIN_SURROGATE && codePoint <= Character.MAX_SURROGATE)) {
            throw malformed("invalid character scalar");
          }
          return HaraCharacter.of(codePoint);
        case BIG_INTEGER:
          return NumUtils.normalizeInteger(new BigInteger(text()));
        case REGEX:
          return Pattern.compile(text());
        case STRING:
          return text();
        case BYTES:
          return bytes();
        case KEYWORD:
          return Keyword.create(text());
        case SYMBOL:
          return Symbol.create(text());
        case LIST:
          return sequence(depth + 1, false);
        case VECTOR:
          return sequence(depth + 1, true);
        case TUPLE:
          return tuple(depth + 1);
        case MAP_ENTRY:
          return mapEntry(depth + 1);
        case CONS:
          return cons(depth + 1);
        case QUEUE:
          return hara.lang.data.Queue.Standard.from(null, sequenceArray(depth + 1, "queue"));
        case DEQUE:
          return hara.lang.data.Deque.Standard.from(null, sequenceArray(depth + 1, "deque"));
        case SET:
          return set(depth + 1);
        case ORDERED_SET:
          return hara.lang.data.OrderedSet.Standard.from(
              null, sequenceArray(depth + 1, "ordered set"));
        case SORTED_SET:
          return hara.lang.data.SortedSet.Standard.from(
              null, sequenceArray(depth + 1, "sorted set"));
        case MAP:
          return map(depth + 1);
        case ORDERED_MAP:
          return hara.lang.data.OrderedMap.Standard.from(
              null, mapArray(depth + 1, "ordered map", false));
        case SORTED_MAP:
          return hara.lang.data.SortedMap.Standard.from(
              null, mapArray(depth + 1, "sorted map", false));
        case PRIORITY_MAP:
          return hara.lang.data.PriorityMap.Standard.from(
              null, mapArray(depth + 1, "priority map", false));
        case TRIE:
          return trie(depth + 1);
        case HANDLE:
          String owner = text();
          String type = text();
          require(8);
          return new HtaHandle(owner, type, input.getLong());
        case STRUCT:
          return struct(depth + 1);
        case TAGGED:
          Object tagValue = read(depth + 1);
          if (!(tagValue instanceof Symbol)) throw malformed("invalid tagged literal tag");
          return new hara.lang.data.TaggedLiteral((Symbol) tagValue, read(depth + 1));
        case EXCEPTION_INFO:
          return exceptionInfo(depth + 1);
        case POINTER:
          return pointer(depth + 1);
        case VAR_REF:
          return varReference(depth + 1);
        default:
          throw malformed("unknown value tag " + tag);
      }
    }

    private Object varReference(int depth) {
      Object value = read(depth);
      if (!(value instanceof Symbol symbol) || symbol.getNamespace() == null) {
        throw malformed("invalid Var reference");
      }
      return new HaraVar(symbol.getNamespace(), symbol.getName(), null);
    }

    private Object struct(int depth) {
      Object nameValue = read(depth);
      Object fieldValue = read(depth);
      Object valuesValue = read(depth);
      if (!(nameValue instanceof String)) {
        throw malformed("invalid struct type name");
      }
      Object[] fieldObjects = sequenceValues(fieldValue, "struct fields");
      Object[] members = sequenceValues(valuesValue, "struct values");
      if (fieldObjects.length != members.length) {
        throw malformed("struct field/value arity mismatch");
      }
      String[] fields = new String[fieldObjects.length];
      for (int index = 0; index < fields.length; index++) {
        if (!(fieldObjects[index] instanceof String)) {
          throw malformed("invalid struct field name");
        }
        fields[index] = (String) fieldObjects[index];
      }
      if (RESULT_STRUCT_NAME.equals(nameValue)
          && java.util.Arrays.equals(fields, RESULT_STRUCT_FIELDS)) {
        return result(members);
      }
      return new HaraStruct(new HaraType((String) nameValue, fields), members);
    }

    private Object result(Object[] members) {
      Object status = HaraBox.unwrap(members[0]);
      Object data = nullable(members[1]);
      Object error = nullable(members[2]);
      Object context = HaraPersistentValues.normalize(members[3]);
      if (Keyword.create("success").equals(status)) {
        if (error != null) throw malformed("success Result contains an error");
        return HaraResult.success(data, context);
      }
      if (Keyword.create("error").equals(status)) {
        if (data != null) throw malformed("error Result contains success data");
        if (!(error instanceof hara.lang.base.Ex.Info)) {
          throw malformed("error Result lacks a native Error");
        }
        return HaraResult.error(error, context);
      }
      throw malformed("invalid Result status");
    }

    private Object nullable(Object value) {
      Object raw = HaraBox.unwrap(value);
      return raw == HaraNull.SINGLETON ? null : raw;
    }

    private Object[] sequenceValues(Object value, String kind) {
      if (value instanceof ILinearType<?> sequence) {
        Object[] result = new Object[(int) sequence.count()];
        for (int index = 0; index < result.length; index++) result[index] = sequence.nth(index);
        return result;
      }
      if (value instanceof List<?> sequence) {
        return sequence.toArray();
      }
      throw malformed("invalid " + kind);
    }

    private Object sequence(int depth, boolean vector) {
      int size = size();
      requireContainerItems(size, 1, "sequence");
      ArrayList<Object> result = new ArrayList<>(size);
      for (int i = 0; i < size; i++) result.add(read(depth));
      if (!canonicalCollections) return result;
      Object[] values = result.toArray();
      return vector
          ? hara.lang.data.Vector.Standard.from(null, values)
          : hara.lang.data.List.Standard.from(null, values);
    }

    private Object tuple(int depth) {
      Object[] values = sequenceArray(depth, "tuple");
      int size = values.length;
      return switch (size) {
        case 0 -> hara.lang.data.Tuple.Tup0.EMPTY;
        case 1 -> new hara.lang.data.Tuple.Tup1.L<>(null, values[0]);
        case 2 -> new hara.lang.data.Tuple.Tup2.L<>(null, values[0], values[1]);
        case 3 -> new hara.lang.data.Tuple.Tup3.L<>(null, values[0], values[1], values[2]);
        case 4 -> new hara.lang.data.Tuple.Tup4.L<>(null, values[0], values[1], values[2], values[3]);
        case 5 -> new hara.lang.data.Tuple.Tup5.L<>(null, values[0], values[1], values[2], values[3], values[4]);
        case 6 -> new hara.lang.data.Tuple.Tup6.L<>(null, values[0], values[1], values[2], values[3], values[4], values[5]);
        case 7 -> new hara.lang.data.Tuple.Tup7.L<>(null, values[0], values[1], values[2], values[3], values[4], values[5], values[6]);
        case 8 -> new hara.lang.data.Tuple.Tup8.L<>(null, values[0], values[1], values[2], values[3], values[4], values[5], values[6], values[7]);
        default -> throw malformed("tuple arity exceeds Java runtime maximum");
      };
    }

    private Object mapEntry(int depth) {
      Object[] values = sequenceArray(depth, "map entry");
      if (values.length != 2) throw malformed("map entry must contain two values");
      return new MapEntry<>(null, values[0], values[1]);
    }

    private Object cons(int depth) {
      Object[] values = sequenceArray(depth, "cons");
      if (values.length == 0) throw malformed("empty cons");
      hara.lang.data.types.ILinkedType<Object> result = null;
      for (int index = values.length - 1; index >= 0; index--) {
        result = new hara.lang.data.Cons<>(null, values[index], result);
      }
      return result;
    }

    private Object[] sequenceArray(int depth, String kind) {
      int size = size();
      requireContainerItems(size, 1, kind);
      Object[] values = new Object[size];
      for (int index = 0; index < size; index++) values[index] = read(depth);
      return values;
    }

    private Object[] mapArray(int depth, String kind, boolean requireStringKeys) {
      int size = size();
      requireContainerItems(size, 2, kind);
      Object[] values = new Object[size * 2];
      for (int index = 0; index < size; index++) {
        Object key = read(depth);
        if (requireStringKeys && !(key instanceof String)) {
          throw malformed("invalid trie key");
        }
        values[index * 2] = key;
        values[index * 2 + 1] = read(depth);
      }
      return values;
    }

    private Object trie(int depth) {
      Object[] entries = mapArray(depth, "trie", true);
      hara.lang.data.Trie<Object> result = new hara.lang.data.Trie.Standard<>();
      for (int index = 0; index < entries.length; index += 2) {
        result = result.assoc((String) entries[index], entries[index + 1]);
      }
      return result;
    }

    private Object pointer(int depth) {
      Object context = read(depth);
      if (!(context instanceof Keyword)) throw malformed("invalid pointer context");
      Object fields = read(depth);
      java.util.Map<Object, Object> descriptor = new LinkedHashMap<>();
      if (fields instanceof java.util.Map<?, ?> map) {
        map.forEach(descriptor::put);
      } else if (fields instanceof IMapType<?, ?> map) {
        Iterator<?> entries = map.iterator();
        while (entries.hasNext()) {
          Object item = entries.next();
          if (!(item instanceof java.util.Map.Entry<?, ?> entry)) {
            throw malformed("invalid pointer field entry");
          }
          descriptor.put(entry.getKey(), entry.getValue());
        }
      } else {
        throw malformed("invalid pointer fields");
      }
      return new hara.lang.data.Pointer(context, descriptor);
    }

    private Object exceptionInfo(int depth) {
      Object message = read(depth);
      Object data = read(depth);
      Object cause = read(depth);
      Object provenance = read(depth);
      if (!(message instanceof String)) throw malformed("invalid exception message");
      if (!(data instanceof hara.lang.protocol.IMetadata)) {
        throw malformed("invalid exception data");
      }
      Throwable throwable = cause instanceof Throwable ? (Throwable) cause : null;
      if (cause != HaraNull.SINGLETON && throwable == null) {
        throw malformed("invalid exception cause");
      }
      Map<Object, Object> fields = mapEntries(provenance);
      if (fields.size() != 2
          || !hasField(fields, "ex/created-at")
          || !hasField(fields, "ex/throws")) {
        throw malformed("invalid exception provenance fields");
      }
      hara.lang.base.Ex.Info.Site createdAt = null;
      Object created = field(fields, "ex/created-at");
      if (created != null && created != HaraNull.SINGLETON) createdAt = site(created);
      List<hara.lang.base.Ex.Info.Site> throwsAt = new ArrayList<>();
      Object thrown = field(fields, "ex/throws");
      if (thrown instanceof List<?> list) {
        for (Object value : list) throwsAt.add(site(value));
      } else if (thrown instanceof ILinearType<?> linear) {
        Iterator<?> iterator = linear.iterator();
        while (iterator.hasNext()) throwsAt.add(site(iterator.next()));
      } else {
        throw malformed("invalid exception throws provenance");
      }
      return new hara.lang.base.Ex.Info(
          (String) message, (hara.lang.protocol.IMetadata) data, throwable, createdAt, throwsAt);
    }

    private Map<Object, Object> mapEntries(Object value) {
      Map<Object, Object> entries = new LinkedHashMap<>();
      if (value instanceof Map<?, ?> map) {
        map.forEach(entries::put);
      } else if (value instanceof IMapType<?, ?> map) {
        Iterator<?> iterator = map.iterator();
        while (iterator.hasNext()) {
          Object item = iterator.next();
          if (!(item instanceof Map.Entry<?, ?> entry)) throw malformed("invalid map entry");
          entries.put(entry.getKey(), entry.getValue());
        }
      } else {
        throw malformed("invalid exception provenance");
      }
      return entries;
    }

    private hara.lang.base.Ex.Info.Site site(Object value) {
      Map<Object, Object> fields = mapEntries(value);
      if (fields.size() != 4
          || !hasField(fields, "namespace")
          || !hasField(fields, "resource")
          || !hasField(fields, "line")
          || !hasField(fields, "column")) {
        throw malformed("invalid exception provenance site");
      }
      Object namespace = field(fields, "namespace");
      Object resource = field(fields, "resource");
      Object line = field(fields, "line");
      Object column = field(fields, "column");
      if ((namespace != null && namespace != HaraNull.SINGLETON && !(namespace instanceof String))
          || (resource != null && resource != HaraNull.SINGLETON && !(resource instanceof String))
          || !(line instanceof Long)
          || !(column instanceof Long)
          || (line instanceof Long l && l < 0)
          || (column instanceof Long l && l < 0)) {
        throw malformed("invalid exception provenance site");
      }
      return new hara.lang.base.Ex.Info.Site(
          namespace instanceof String ? (String) namespace : null,
          resource instanceof String ? (String) resource : null,
          (Long) line,
          (Long) column);
    }

    private boolean hasField(Map<Object, Object> fields, String name) {
      return fields.keySet().stream()
          .anyMatch(
              key ->
                  (key instanceof Keyword && name.equals(keywordName((Keyword) key)))
                      || (key instanceof String && name.equals(key)));
    }

    private Object field(Map<Object, Object> fields, String name) {
      return fields.entrySet().stream()
          .filter(
              entry ->
                  (entry.getKey() instanceof Keyword
                          && name.equals(keywordName((Keyword) entry.getKey())))
                      || (entry.getKey() instanceof String && name.equals(entry.getKey())))
          .map(Map.Entry::getValue)
          .findFirst()
          .orElse(null);
    }

    private String keywordName(Keyword keyword) {
      return keyword.getNamespace() == null
          ? keyword.getName()
          : keyword.getNamespace() + "/" + keyword.getName();
    }

    private Object set(int depth) {
      int size = size();
      requireContainerItems(size, 1, "set");
      LinkedHashSet<Object> result = new LinkedHashSet<>();
      for (int i = 0; i < size; i++) result.add(read(depth));
      return canonicalCollections
          ? hara.lang.data.Set.Standard.from(null, result.toArray())
          : result;
    }

    private Object map(int depth) {
      int size = size();
      requireContainerItems(size, 2, "map");
      LinkedHashMap<Object, Object> result = new LinkedHashMap<>();
      for (int i = 0; i < size; i++) result.put(read(depth), read(depth));
      if (!canonicalCollections) return result;
      Object[] entries = new Object[result.size() * 2];
      int index = 0;
      for (java.util.Map.Entry<Object, Object> entry : result.entrySet()) {
        entries[index++] = entry.getKey();
        entries[index++] = entry.getValue();
      }
      return hara.lang.data.Map.Standard.from(null, entries);
    }

    private String text() {
      return new String(bytes(), StandardCharsets.UTF_8);
    }

    private byte[] bytes() {
      int size = size();
      require(size);
      byte[] result = new byte[size];
      input.get(result);
      return result;
    }

    private int size() {
      require(4);
      int size = input.getInt();
      if (size < 0) throw malformed("negative length");
      return size;
    }

    private void requireContainerItems(int count, int minimumBytes, String kind) {
      if (count > input.remaining() / minimumBytes) {
        throw malformed("impossible " + kind + " length");
      }
    }

    private void require(int amount) {
      if (amount < 0 || input.remaining() < amount) throw malformed("truncated value");
    }
  }
}
