package hara.truffle.node;

import com.oracle.truffle.api.CompilerDirectives;
import com.oracle.truffle.api.CompilerDirectives.TruffleBoundary;
import com.oracle.truffle.api.RootCallTarget;
import com.oracle.truffle.api.frame.MaterializedFrame;
import com.oracle.truffle.api.frame.VirtualFrame;
import com.oracle.truffle.api.interop.ArityException;
import com.oracle.truffle.api.interop.InteropLibrary;
import com.oracle.truffle.api.interop.UnknownIdentifierException;
import com.oracle.truffle.api.interop.UnsupportedMessageException;
import com.oracle.truffle.api.interop.UnsupportedTypeException;
import com.oracle.truffle.api.nodes.ControlFlowException;
import com.oracle.truffle.api.nodes.DirectCallNode;
import com.oracle.truffle.api.nodes.IndirectCallNode;
import com.oracle.truffle.api.nodes.LoopNode;
import com.oracle.truffle.api.source.SourceSection;
import hara.kernel.builtin.BuiltinStruct;
import hara.lang.base.Eq;
import hara.lang.base.Ex;
import hara.lang.base.Iter;
import hara.lang.base.primitive.Cast;
import hara.lang.base.primitive.Num;
import hara.lang.data.Symbol;
import hara.lang.data.Keyword;
import hara.lang.data.HaraCharacter;
import hara.lang.protocol.ILinearType;
import hara.lang.protocol.IMapType;
import hara.lang.protocol.ISetType;
import hara.lang.protocol.IExInfo;
import hara.lang.protocol.IFn;
import hara.lang.protocol.IMetadata;
import hara.lang.protocol.IObjType;
import hara.truffle.HaraBox;
import hara.truffle.HaraBuiltinFunction;
import hara.truffle.HaraContext;
import hara.truffle.HaraException;
import hara.truffle.HaraFunction;
import hara.truffle.HaraLanguage;
import hara.truffle.HalcSchema;
import hara.truffle.HaraMultiFunction;
import hara.truffle.HaraMutable;
import hara.truffle.HaraProtocol;
import hara.truffle.HaraProtocolImplementation;
import hara.truffle.HaraStruct;
import hara.truffle.HaraType;
import hara.truffle.HaraVar;
import hara.truffle.HbcMachine;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.Iterator;
import java.util.LinkedHashMap;

public final class HaraNodes {
  private HaraNodes() {}

  private static Object constructNamedValue(HaraType type, Object[] values) {
    try {
      return type.construct(values);
    } catch (ArityException impossible) {
      throw new IllegalStateException("named value arity was checked before construction", impossible);
    }
  }

  public static final class RecurTarget {
    private final int[] slots;
    private final int[] scratchSlots;
    private final RecurException signal;

    public RecurTarget(int[] slots, int[] scratchSlots) {
      this.slots = slots;
      this.scratchSlots = scratchSlots;
      this.signal = new RecurException(this);
    }

    public int arity() {
      return slots.length;
    }

    public int[] slots() {
      return slots;
    }

    /**
     * Per-loop staging slots for recurrence values: {@link Recur} evaluates into these before
     * copying into the binding slots, so no array is allocated per iteration.
     */
    public int[] scratchSlots() {
      return scratchSlots;
    }

    /**
     * The single recurrence signal for this target. The exception carries no per-iteration
     * state (values travel through frame slots), so one stackless instance serves every
     * recurrence and is safe to throw concurrently from multiple frames.
     */
    RecurException signal() {
      return signal;
    }
  }

  /**
   * Lightweight, stackless recurrence signal. Graal treats {@link ControlFlowException} as a
   * canonical control-transfer mechanism, so a thrown recur compiles to a plain loop back-edge
   * once the loop body is inlined; no stack trace is ever captured. Each {@link RecurTarget}
   * owns exactly one instance, thrown on every recurrence of that loop.
   */
  @SuppressWarnings("serial")
  private static final class RecurException extends ControlFlowException {
    private final RecurTarget target;

    private RecurException(RecurTarget target) {
      this.target = target;
    }
  }

  private static final class ThrownValue extends RuntimeException {
    private final Object value;

    private ThrownValue(Object value) {
      super(hara.lang.base.G.display(value));
      this.value = value;
    }
  }

  public static final class Literal extends HaraExpressionNode {
    private final Object value;

    public Literal(Object value) {
      this.value = value;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      return value;
    }
  }

  /** Builds syntax-quoted Hara data while evaluating only explicit unquotes. */
  public static final class SyntaxQuote extends HaraExpressionNode {
    public static final class Unquote {
      private final int index;
      private final boolean splice;

      public Unquote(int index, boolean splice) {
        this.index = index;
        this.splice = splice;
      }
    }

    public static final class AutoGensym {
      private final int index;
      private final String prefix;

      public AutoGensym(int index, String prefix) {
        this.index = index;
        this.prefix = prefix;
      }
    }

    private final Object template;
    @Children private final HaraExpressionNode[] unquotes;

    public SyntaxQuote(Object template, HaraExpressionNode[] unquotes) {
      this.template = template;
      this.unquotes = unquotes;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      Object[] values = new Object[unquotes.length];
      for (int index = 0; index < unquotes.length; index++) {
        values[index] = unquotes[index].execute(frame);
      }
      return materializeTemplate(template, values);
    }

    @TruffleBoundary
    private Object materializeTemplate(Object template, Object[] values) {
      return materialize(template, values, new HashMap<>());
    }

    @SuppressWarnings({"rawtypes", "unchecked"})
    @TruffleBoundary
    private Object materialize(
        Object value, Object[] values, java.util.Map<Integer, Symbol> gensyms) {
      if (value instanceof Unquote unquote) {
        if (unquote.splice) {
          throw new HaraException("unquote-splicing is only valid inside a collection", this);
        }
        return values[unquote.index];
      }
      if (value instanceof AutoGensym auto) {
        return gensyms.computeIfAbsent(
            auto.index, ignored -> HaraLanguage.currentContext(this).gensym(auto.prefix));
      }
      if (value instanceof hara.lang.data.List<?> list) {
        ArrayList<Object> output = new ArrayList<>();
        for (Object item : list) append(output, item, values, gensyms);
        return hara.lang.data.List.Standard.from(metadata(list), output.toArray());
      }
      if (value instanceof ILinearType<?> vector
          && !(value instanceof hara.lang.data.List)
          && "[".equals(vector.startString())) {
        ArrayList<Object> output = new ArrayList<>();
        for (Object item : vector) append(output, item, values, gensyms);
        Object sequence =
            output.size() <= 8
                ? BuiltinStruct.tuple(output.toArray())
                : hara.lang.data.Vector.Standard.from(null, output.toArray());
        return ((IObjType) sequence).withMeta(metadata(vector));
      }
      if (value instanceof ISetType<?> set) {
        ArrayList<Object> output = new ArrayList<>();
        for (Object item : set) append(output, item, values, gensyms);
        return hara.lang.data.Set.Standard.from(metadata(set), output.toArray());
      }
      if (value instanceof IMapType<?, ?> map) {
        ArrayList<Object> output = new ArrayList<>();
        for (Object entryValue : map) {
          java.util.Map.Entry<?, ?> entry = (java.util.Map.Entry<?, ?>) entryValue;
          output.add(materializeMapEntry(entry.getKey(), values, gensyms));
          output.add(materializeMapEntry(entry.getValue(), values, gensyms));
        }
        if (value instanceof hara.lang.data.OrderedMap) {
          return hara.lang.data.OrderedMap.Standard.from(metadata(map), output.toArray());
        }
        return hara.lang.data.Map.Standard.from(metadata(map), output.toArray());
      }
      return value;
    }

    private Object materializeMapEntry(
        Object value, Object[] values, java.util.Map<Integer, Symbol> gensyms) {
      if (value instanceof Unquote unquote && unquote.splice) {
        throw new HaraException("unquote-splicing is not valid in a map entry", this);
      }
      return materialize(value, values, gensyms);
    }

    private void append(
        ArrayList<Object> output,
        Object value,
        Object[] values,
        java.util.Map<Integer, Symbol> gensyms) {
      if (value instanceof Unquote unquote && unquote.splice) {
        Object expanded = values[unquote.index];
        if (!(expanded instanceof ILinearType<?> sequence)) {
          throw new HaraException("unquote-splicing expects a sequential value", this);
        }
        for (Object item : sequence) output.add(item);
        return;
      }
      output.add(materialize(value, values, gensyms));
    }

    private IMetadata metadata(Object value) {
      return value instanceof IObjType object ? object.meta() : null;
    }
  }

  /** Evaluates the contents of a reader collection and rebuilds its Java-backed value. */
  public static final class CollectionLiteral extends HaraExpressionNode {
    public enum Kind {
      TUPLE,
      VECTOR,
      QUEUE,
      MUTABLE_ARRAY,
      MUTABLE_OBJECT,
      MAP,
      ORDERED_MAP,
      SORTED_MAP,
      SET,
      ORDERED_SET,
      SORTED_SET
    }

    private final Kind kind;
    @Children private final HaraExpressionNode[] elements;

    public CollectionLiteral(Kind kind, HaraExpressionNode[] elements) {
      this.kind = kind;
      this.elements = elements;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      Object[] values = new Object[elements.length];
      for (int i = 0; i < elements.length; i++) {
        values[i] = elements[i].execute(frame);
      }
      return construct(kind, values);
    }

    @TruffleBoundary
    @SuppressWarnings({"rawtypes", "unchecked"})
    private static Object construct(Kind kind, Object[] values) {
      switch (kind) {
        case TUPLE:
          return BuiltinStruct.tuple(values);
        case VECTOR:
          return BuiltinStruct.vector(values);
        case QUEUE:
          return BuiltinStruct.queue(values);
        case MUTABLE_ARRAY:
          return new ArrayList<>(java.util.Arrays.asList(values));
        case MUTABLE_OBJECT:
          if ((values.length & 1) != 0) {
            throw new HaraException("x:object expects an even number of key/value forms");
          }
          LinkedHashMap<Object, Object> object = new LinkedHashMap<>();
          for (int i = 0; i < values.length; i += 2) {
            object.put(values[i], values[i + 1]);
          }
          return object;
        case MAP:
          return BuiltinStruct.hashMap(values);
        case ORDERED_MAP:
          return BuiltinStruct.orderedMap(values);
        case SORTED_MAP:
          return BuiltinStruct.sortedMap(values);
        case SET:
          return BuiltinStruct.hashSet(values);
        case ORDERED_SET:
          return BuiltinStruct.orderedSet(values);
        case SORTED_SET:
          return BuiltinStruct.sortedSet(values);
        default:
          throw new IllegalStateException("Unknown collection literal: " + kind);
      }
    }
  }

  /** Constructs an ordinary mutable byte value from evaluated numeric values. */
  public static final class Bytes extends HaraExpressionNode {
    @Children private final HaraExpressionNode[] elements;

    public Bytes(HaraExpressionNode[] elements) {
      this.elements = elements;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      byte[] result = new byte[elements.length];
      for (int i = 0; i < elements.length; i++) {
        Object value = elements[i].execute(frame);
        try {
          result[i] = byteCast(value);
        } catch (IllegalArgumentException error) {
          throw new HaraException("bytes expects values in the byte range", this);
        }
      }
      return result;
    }

    @TruffleBoundary
    private static byte byteCast(Object value) {
      return Cast.byteCast(value);
    }
  }

  public static final class ByteValue extends HaraExpressionNode {
    public enum Operator {
      SIGNED,
      UNSIGNED
    }

    private final Operator operator;
    @Child private HaraExpressionNode value;

    public ByteValue(Operator operator, HaraExpressionNode value) {
      this.operator = operator;
      this.value = value;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      Object input = HaraBox.unwrap(value.execute(frame));
      long number;
      try {
        number = longCast(input);
      } catch (RuntimeException error) {
        throw new HaraException("byte conversion expects an integral numeric value", this);
      }
      if (number < Byte.MIN_VALUE || number > 0xffL) {
        throw new HaraException("byte conversion expects a value in the range -128..255", this);
      }
      if (operator == Operator.UNSIGNED) return number < 0 ? number + 256 : number;
      return number > Byte.MAX_VALUE ? number - 256 : number;
    }

    @TruffleBoundary
    private static long longCast(Object value) {
      return Cast.longCast(value);
    }
  }

  public static final class ByteCopy extends HaraExpressionNode {
    @Child private HaraExpressionNode value;

    public ByteCopy(HaraExpressionNode value) {
      this.value = value;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      Object bytes = HaraBox.unwrap(value.execute(frame));
      if (!(bytes instanceof byte[])) {
        throw new HaraException("byte-copy expects bytes", this);
      }
      return ((byte[]) bytes).clone();
    }
  }

  public static final class ByteSlice extends HaraExpressionNode {
    @Child private HaraExpressionNode value;
    @Child private HaraExpressionNode start;
    @Child private HaraExpressionNode end;

    public ByteSlice(HaraExpressionNode value, HaraExpressionNode start, HaraExpressionNode end) {
      this.value = value;
      this.start = start;
      this.end = end;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      Object bytes = HaraBox.unwrap(value.execute(frame));
      Object startValue = start.execute(frame);
      Object endValue = end.execute(frame);
      if (!(bytes instanceof byte[])) {
        throw new HaraException("byte-slice expects bytes", this);
      }
      if (!(startValue instanceof Number) || !(endValue instanceof Number)) {
        throw new HaraException("byte-slice indexes must be numeric", this);
      }
      return sliceBytes((byte[]) bytes, startValue, endValue);
    }

    @TruffleBoundary
    private Object sliceBytes(byte[] array, Object startValue, Object endValue) {
      long startIndex = ((Number) startValue).longValue();
      long endIndex = ((Number) endValue).longValue();
      if (startIndex < 0 || endIndex < startIndex || endIndex > array.length) {
        throw new HaraException(
            "byte-slice range is out of bounds: " + startIndex + ".." + endIndex, this);
      }
      return java.util.Arrays.copyOfRange(array, (int) startIndex, (int) endIndex);
    }
  }

  public static final class MutableOperation extends HaraExpressionNode {
    public enum Operator {
      LENGTH,
      GET,
      SET,
      DELETE,
      APPEND,
      INSERT,
      REMOVE,
      CLONE,
      SLICE,
      BYTE_LENGTH,
      BYTE_GET,
      BYTE_SET
    }

    @Children private final HaraExpressionNode[] arguments;
    private final Operator operator;

    public MutableOperation(Operator operator, HaraExpressionNode[] arguments) {
      this.operator = operator;
      this.arguments = arguments;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      Object[] values = new Object[arguments.length];
      for (int i = 0; i < arguments.length; i++) {
        values[i] = HaraBox.unwrap(arguments[i].execute(frame));
      }
      return executeOperation(values);
    }

    @TruffleBoundary
    private Object executeOperation(Object[] values) {
      switch (operator) {
        case LENGTH:
          return length(values[0]);
        case GET:
          return get(values[0], values[1], values.length == 3 ? values[2] : null);
        case SET:
          set(values[0], values[1], values[2]);
          return values[0];
        case DELETE:
          delete(values[0], values[1]);
          return values[0];
        case APPEND:
          append(values[0], values[1]);
          return values[0];
        case INSERT:
          insert(values[0], values[1], values[2]);
          return values[0];
        case REMOVE:
          remove(values[0], values[1]);
          return values[0];
        case CLONE:
          return clone(values[0]);
        case SLICE:
          return slice(values[0], values[1], values.length == 3 ? values[2] : length(values[0]));
        case BYTE_LENGTH:
          return byteLength(values[0]);
        case BYTE_GET:
          return byteGet(
              values[0], values[1], values.length == 3 ? values[2] : null, values.length == 3);
        case BYTE_SET:
          byteSet(values[0], values[1], values[2]);
          return values[0];
        default:
          throw unsupportedOperator(operator);
      }
    }

    private static long byteLength(Object value) {
      if (value instanceof byte[]) return ((byte[]) value).length;
      throw new HaraException("byte-count expects bytes");
    }

    private static Object byteGet(Object target, Object key, Object fallback, boolean hasFallback) {
      if (!(target instanceof byte[])) {
        throw new HaraException("byte-get expects bytes");
      }
      int index;
      try {
        index = index(key, target);
      } catch (HaraException | IndexOutOfBoundsException error) {
        if (hasFallback) return fallback;
        throw new HaraException("byte-get index out of bounds: " + key);
      }
      return ((byte[]) target)[index];
    }

    private static void byteSet(Object target, Object key, Object value) {
      if (!(target instanceof byte[])) {
        throw new HaraException("byte-set expects bytes");
      }
      int index;
      try {
        index = index(key, target);
      } catch (HaraException | IndexOutOfBoundsException error) {
        throw new HaraException("byte-set index out of bounds: " + key);
      }
      try {
        ((byte[]) target)[index] = Cast.byteCast(value);
      } catch (IllegalArgumentException error) {
        throw new HaraException("byte-set expects a value in the byte range");
      }
    }

    private static long length(Object value) {
      if (value instanceof hara.lang.protocol.ILinearType<?>) {
        return ((hara.lang.protocol.ILinearType<?>) value).count();
      }
      if (value instanceof java.util.Map<?, ?>) return ((java.util.Map<?, ?>) value).size();
      if (value instanceof java.util.List<?>) return ((java.util.List<?>) value).size();
      if (value instanceof String) return ((String) value).codePointCount(0, ((String) value).length());
      if (value != null && value.getClass().isArray()) {
        return java.lang.reflect.Array.getLength(value);
      }
      throw new HaraException("x:len does not support value: " + value);
    }

    private static Object get(Object target, Object key, Object fallback) {
      try {
        if (target instanceof java.util.Map<?, ?>) {
          java.util.Map<?, ?> map = (java.util.Map<?, ?>) target;
          return map.containsKey(key) ? map.get(key) : fallback;
        }
        if (target instanceof java.util.List<?>) {
          return ((java.util.List<?>) target).get(index(key, target));
        }
        if (target instanceof byte[]) return ((byte[]) target)[index(key, target)];
        if (target instanceof hara.lang.protocol.ILinearType<?>) {
          return ((hara.lang.protocol.ILinearType<?>) target).nth(indexLong(key));
        }
        if (target instanceof String) {
          String string = (String) target;
          int index = index(key, target);
          return HaraCharacter.of(string.codePointAt(string.offsetByCodePoints(0, index)));
        }
        if (target != null && target.getClass().isArray()) {
          return java.lang.reflect.Array.get(target, index(key, target));
        }
      } catch (IndexOutOfBoundsException error) {
        return fallback;
      }
      throw new HaraException("x:get does not support target: " + target, null);
    }

    private static void set(Object target, Object key, Object value) {
      try {
        if (target instanceof java.util.Map<?, ?>) {
          ((java.util.Map<Object, Object>) target).put(key, value);
          return;
        }
        int index = index(key, target);
        if (target instanceof java.util.List<?>) {
          ((java.util.List<Object>) target).set(index, value);
        } else if (target instanceof byte[]) {
          try {
            ((byte[]) target)[index] = Cast.byteCast(value);
          } catch (IllegalArgumentException error) {
            throw new HaraException("x:set expects a value in the byte range");
          }
        } else if (target != null && target.getClass().isArray()) {
          java.lang.reflect.Array.set(target, index, value);
        } else {
          throw new HaraException("x:set does not support target: " + target);
        }
      } catch (IndexOutOfBoundsException error) {
        throw new HaraException("x:set index out of bounds: " + key);
      }
    }

    private static void delete(Object target, Object key) {
      try {
        if (target instanceof java.util.Map<?, ?>) {
          ((java.util.Map<?, ?>) target).remove(key);
        } else if (target instanceof java.util.List<?>) {
          ((java.util.List<?>) target).remove(index(key, target));
        } else {
          throw new HaraException("x:delete does not support target: " + target);
        }
      } catch (IndexOutOfBoundsException error) {
        throw new HaraException("x:delete index out of bounds: " + key);
      }
    }

    private static void append(Object target, Object value) {
      if (target instanceof java.util.List<?>) {
        ((java.util.List<Object>) target).add(value);
        return;
      }
      throw new HaraException("x:append does not support target: " + target);
    }

    private static void insert(Object target, Object key, Object value) {
      if (target instanceof java.util.List<?>) {
        java.util.List<Object> list = (java.util.List<Object>) target;
        long index = indexLong(key);
        if (index < 0 || index > list.size()) {
          throw new HaraException("x:insert index out of bounds: " + index);
        }
        list.add((int) index, value);
        return;
      }
      throw new HaraException("x:insert does not support target: " + target);
    }

    private static void remove(Object target, Object key) {
      try {
        if (target instanceof java.util.List<?>) {
          ((java.util.List<?>) target).remove(index(key, target));
          return;
        }
        if (target instanceof java.util.Map<?, ?>) {
          ((java.util.Map<?, ?>) target).remove(key);
          return;
        }
      } catch (IndexOutOfBoundsException error) {
        throw new HaraException("x:remove index out of bounds: " + key);
      }
      throw new HaraException("x:remove does not support target: " + target);
    }

    private static Object clone(Object target) {
      if (target instanceof byte[]) return ((byte[]) target).clone();
      if (target instanceof java.util.List<?>) {
        return new java.util.ArrayList<>((java.util.List<?>) target);
      }
      if (target instanceof java.util.Map<?, ?>) {
        return new java.util.LinkedHashMap<>((java.util.Map<?, ?>) target);
      }
      throw new HaraException("x:clone does not support target: " + target);
    }

    private static Object slice(Object target, Object startValue, Object endValue) {
      long start = indexLong(startValue);
      long end = indexLong(endValue);
      if (start < 0 || end < start || end > length(target)) {
        throw new HaraException("x:slice range is out of bounds");
      }
      if (target instanceof byte[]) {
        return java.util.Arrays.copyOfRange((byte[]) target, (int) start, (int) end);
      }
      if (target instanceof String) {
        String string = (String) target;
        int startOffset = string.offsetByCodePoints(0, (int) start);
        int endOffset = string.offsetByCodePoints(0, (int) end);
        return string.substring(startOffset, endOffset);
      }
      if (target instanceof java.util.List<?>) {
        return new java.util.ArrayList<>(
            ((java.util.List<?>) target).subList((int) start, (int) end));
      }
      if (target instanceof hara.lang.protocol.ILinearType<?>) {
        Object[] values = new Object[(int) (end - start)];
        for (int i = 0; i < values.length; i++) {
          values[i] = ((hara.lang.protocol.ILinearType<?>) target).nth(start + i);
        }
        return BuiltinStruct.vector(values);
      }
      throw new HaraException("x:slice does not support target: " + target);
    }

    private static int index(Object key, Object target) {
      long index = indexLong(key);
      if (index < 0 || index >= length(target)) {
        throw new IndexOutOfBoundsException("index: " + index);
      }
      return (int) index;
    }

    private static long indexLong(Object key) {
      if (!(key instanceof Number)) throw new HaraException("index must be numeric: " + key);
      return ((Number) key).longValue();
    }
  }

  public static final class ReadLocal extends HaraExpressionNode {
    private final int slot;

    public ReadLocal(int slot) {
      this.slot = slot;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      return frame.getValue(slot);
    }
  }

  public static final class Lookup extends HaraExpressionNode {
    @Child private HaraExpressionNode target;
    private final Object key;

    public Lookup(HaraExpressionNode target, long index) {
      this.target = target;
      this.key = index;
    }

    public Lookup(HaraExpressionNode target, Object key) {
      this.target = target;
      this.key = key;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      Object value = target.execute(frame);
      if (value == null) return null;
      if (key instanceof Number && value instanceof hara.lang.protocol.ILinearType<?>) {
        long index = keyIndex();
        hara.lang.protocol.ILinearType<?> linear = (hara.lang.protocol.ILinearType<?>) value;
        return index < 0 || index >= linear.count() ? null : linear.nth(index);
      }
      if (key instanceof Number && value instanceof hara.lang.data.types.ILinkedType<?>) {
        long index = keyIndex();
        if (index < 0) return null;
        java.util.Iterator<?> values = ((hara.lang.data.types.ILinkedType<?>) value).iterator();
        for (long position = 0; values.hasNext(); position++) {
          Object current = values.next();
          if (position == index) return current;
        }
        return null;
      }
      if (value instanceof String) {
        String string = (String) value;
        long index = keyIndex();
        return index < 0 || index >= string.codePointCount(0, string.length())
            ? null
            : HaraCharacter.of(
                string.codePointAt(string.offsetByCodePoints(0, Math.toIntExact(index))));
      }
      return lookupGeneric(value);
    }

    @TruffleBoundary
    private long keyIndex() {
      return ((Number) key).longValue();
    }

    @TruffleBoundary
    private Object lookupGeneric(Object value) {
      if (value instanceof java.util.List<?>) {
        int index = ((Number) key).intValue();
        return index < 0 || index >= ((java.util.List<?>) value).size()
            ? null
            : ((java.util.List<?>) value).get(index);
      }
      if (value != null && value.getClass().isArray()) {
        int index = ((Number) key).intValue();
        return index < 0 || index >= java.lang.reflect.Array.getLength(value)
            ? null
            : java.lang.reflect.Array.get(value, index);
      }
      if (value instanceof hara.lang.protocol.ILookup<?, ?>) {
        return ((hara.lang.protocol.ILookup<Object, Object>) value).lookup(key);
      }
      if (value instanceof java.util.Map<?, ?>) {
        return ((java.util.Map<?, ?>) value).get(key);
      }
      throw new HaraException("Cannot destructure value: " + value, this);
    }
  }

  public static final class DefaultValue extends HaraExpressionNode {
    @Child private HaraExpressionNode value;
    @Child private HaraExpressionNode fallback;

    public DefaultValue(HaraExpressionNode value, HaraExpressionNode fallback) {
      this.value = value;
      this.fallback = fallback;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      Object result = value.execute(frame);
      return result == null ? fallback.execute(frame) : result;
    }
  }

  public static final class Rest extends HaraExpressionNode {
    @Child private HaraExpressionNode target;
    private final long start;

    public Rest(HaraExpressionNode target, long start) {
      this.target = target;
      this.start = start;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      Object value = target.execute(frame);
      if (value == null) return null;
      if (value instanceof hara.lang.protocol.ILinearType<?>) {
        hara.lang.protocol.ILinearType<?> linear = (hara.lang.protocol.ILinearType<?>) value;
        if (start > linear.count()) {
          throw new HaraException("Destructuring rest index is out of bounds", this);
        }
        return restLinear(linear);
      }
      if (value instanceof hara.lang.data.types.ILinkedType<?>) {
        return restLinked((hara.lang.data.types.ILinkedType<?>) value);
      }
      if (value instanceof String) {
        return restString((String) value);
      }
      throw cannotRest(value);
    }

    @TruffleBoundary
    private HaraException cannotRest(Object value) {
      return new HaraException("Cannot destructure rest from value: " + value, this);
    }

    @TruffleBoundary
    private Object restLinear(hara.lang.protocol.ILinearType<?> linear) {
      Object[] values = new Object[(int) linear.count() - (int) start];
      for (int i = 0; i < values.length; i++) {
        values[i] = linear.nth(start + i);
      }
      return BuiltinStruct.vector(values);
    }

    @TruffleBoundary
    private Object restLinked(hara.lang.data.types.ILinkedType<?> linked) {
      java.util.Iterator<?> iterator = linked.iterator();
      java.util.ArrayList<Object> values = new java.util.ArrayList<>();
      long position = 0;
      while (iterator.hasNext()) {
        Object value = iterator.next();
        if (position >= start) values.add(value);
        position++;
      }
      return BuiltinStruct.vector(values.toArray());
    }

    @TruffleBoundary
    private Object restString(String value) {
      int length = value.codePointCount(0, value.length());
      if (start < 0 || start > length) {
        throw new HaraException("Destructuring rest index is out of bounds");
      }
      int offset = value.offsetByCodePoints(0, (int) start);
      Object[] values = new Object[length - (int) start];
      for (int index = 0; index < values.length; index++) {
        int codePoint = value.codePointAt(offset);
        values[index] = HaraCharacter.of(codePoint);
        offset += Character.charCount(codePoint);
      }
      return BuiltinStruct.vector(values);
    }
  }

  public static final class ReadGlobal extends HaraExpressionNode {
    private final Symbol symbol;
    private final Symbol displaySymbol;

    public ReadGlobal(Symbol symbol) {
      this(symbol, symbol);
    }

    public ReadGlobal(Symbol symbol, Symbol displaySymbol) {
      this.symbol = symbol;
      this.displaySymbol = displaySymbol;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      return readGlobal(HaraLanguage.currentContext(this));
    }

    @TruffleBoundary
    private Object readGlobal(HaraContext context) {
      if (context.hasNativeSymbol(symbol)) return context.resolveNativeSymbol(symbol);
      HaraVar var = context.resolve(symbol);
      if (var == null) {
        Object namespace = context.resolveNamespaceValue(displaySymbol);
        if (namespace != null) return namespace;
        throw unboundError("symbol");
      }
      return var.deref();
    }

    @TruffleBoundary
    private HaraException unboundError(String kind) {
      return new HaraException("Unbound " + kind + ": " + displaySymbol.display(), this);
    }
  }

  public static final class VarReference extends HaraExpressionNode {
    private final Symbol symbol;

    public VarReference(Symbol symbol) {
      this.symbol = symbol;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      HaraVar var = HaraLanguage.currentContext(this).resolve(symbol);
      if (var == null) {
        throw unboundError("var");
      }
      return var;
    }

    @TruffleBoundary
    private HaraException unboundError(String kind) {
      return new HaraException("Unbound " + kind + ": " + symbol.display(), this);
    }
  }

  public static final class SetVar extends HaraExpressionNode {
    private final Symbol symbol;
    @Child private HaraExpressionNode value;

    public SetVar(Symbol symbol, HaraExpressionNode value) {
      this.symbol = symbol;
      this.value = value;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      HaraVar var = HaraLanguage.currentContext(this).resolve(symbol);
      if (var == null) {
        throw unboundError("var");
      }
      return var.reset(value.execute(frame));
    }

    @TruffleBoundary
    private HaraException unboundError(String kind) {
      return new HaraException("Unbound " + kind + ": " + symbol.display(), this);
    }
  }

  public static final class SetField extends HaraExpressionNode {
    @Child private HaraExpressionNode target;
    @Child private HaraExpressionNode replacement;
    private final String field;

    public SetField(
        HaraExpressionNode target, String field, HaraExpressionNode replacement) {
      this.target = target;
      this.field = field;
      this.replacement = replacement;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      Object receiver = target.execute(frame);
      Object value = replacement.execute(frame);
      if (!(receiver instanceof HaraMutable mutable)) {
        throw new HaraException(
            "set! field expects a mutable value: "
                + field
                + " on "
                + (receiver == null ? "nil" : receiver.getClass().getName()),
            this);
      }
      try {
        return writeMutableField(mutable, value);
      } catch (UnknownIdentifierException exception) {
        throw unknownMutableFieldError();
      }
    }

    @TruffleBoundary
    private Object writeMutableField(HaraMutable mutable, Object value)
        throws UnknownIdentifierException {
      return mutable.write(field, value);
    }

    @TruffleBoundary
    private HaraException unknownMutableFieldError() {
      return new HaraException("Unknown mutable field: " + field, this);
    }
  }

  public static final class Do extends HaraExpressionNode {
    @Children private final HaraExpressionNode[] expressions;

    public Do(HaraExpressionNode[] expressions) {
      this.expressions = expressions;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      Object result = null;
      for (HaraExpressionNode expression : expressions) {
        result = expression.execute(frame);
      }
      return result;
    }
  }

  public static final class If extends HaraExpressionNode {
    @Child private HaraExpressionNode condition;
    @Child private HaraExpressionNode consequent;
    @Child private HaraExpressionNode alternative;

    public If(
        HaraExpressionNode condition,
        HaraExpressionNode consequent,
        HaraExpressionNode alternative) {
      this.condition = condition;
      this.consequent = consequent;
      this.alternative = alternative;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      Object value = condition.execute(frame);
      return HaraBox.isNil(value) || Boolean.FALSE.equals(value)
          ? alternative.execute(frame)
          : consequent.execute(frame);
    }
  }

  public static final class ShortCircuit extends HaraExpressionNode {
    private final boolean all;
    @Children private final HaraExpressionNode[] expressions;

    public ShortCircuit(boolean all, HaraExpressionNode[] expressions) {
      this.all = all;
      this.expressions = expressions;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      Object result = all ? Boolean.TRUE : null;
      for (HaraExpressionNode expression : expressions) {
        result = expression.execute(frame);
        boolean truthy = !HaraBox.isNil(result) && !Boolean.FALSE.equals(result);
        if (all ? !truthy : truthy) return result;
      }
      return result;
    }
  }

  public static final class Let extends HaraExpressionNode {
    private final int[] slots;
    @Children private final HaraExpressionNode[] initializers;
    @Child private HaraExpressionNode body;

    public Let(int[] slots, HaraExpressionNode[] initializers, HaraExpressionNode body) {
      this.slots = slots;
      this.initializers = initializers;
      this.body = body;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      Object[] values = new Object[initializers.length];
      for (int i = 0; i < initializers.length; i++) {
        values[i] = initializers[i].execute(frame);
      }
      for (int i = 0; i < slots.length; i++) {
        frame.setObject(slots[i], values[i]);
      }
      return body.execute(frame);
    }
  }

  public static final class LetFn extends HaraExpressionNode {
    private final int[] slots;
    @Children private final HaraExpressionNode[] functions;
    @Child private HaraExpressionNode body;

    public LetFn(int[] slots, HaraExpressionNode[] functions, HaraExpressionNode body) {
      this.slots = slots;
      this.functions = functions;
      this.body = body;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      for (int i = 0; i < functions.length; i++) {
        frame.setObject(slots[i], functions[i].execute(frame));
      }
      return body.execute(frame);
    }
  }

  public static final class Binding extends HaraExpressionNode {
    private final Symbol[] symbols;
    @Children private final HaraExpressionNode[] initializers;
    @Child private HaraExpressionNode body;

    public Binding(Symbol[] symbols, HaraExpressionNode[] initializers, HaraExpressionNode body) {
      this.symbols = symbols;
      this.initializers = initializers;
      this.body = body;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      Object[] values = new Object[initializers.length];
      for (int i = 0; i < initializers.length; i++) {
        values[i] = initializers[i].execute(frame);
      }
      HaraVar[] vars = new HaraVar[symbols.length];
      int bound = 0;
      try {
        for (int i = 0; i < symbols.length; i++) {
          HaraVar var = HaraLanguage.currentContext(this).resolve(symbols[i]);
          if (var == null) {
            throw bindingError("Unbound dynamic var: ", symbols[i]);
          }
          if (!var.isDynamic()) {
            throw bindingError("binding requires a dynamic Var: ", symbols[i]);
          }
          vars[i] = var;
          var.bind(values[i]);
          bound++;
        }
        return body.execute(frame);
      } finally {
        for (int i = bound - 1; i >= 0; i--) vars[i].unbind();
      }
    }

    @TruffleBoundary
    private HaraException bindingError(String message, Symbol symbol) {
      return new HaraException(message + symbol.display(), this);
    }
  }

  public static final class Loop extends HaraExpressionNode {
    private final RecurTarget target;
    private final int[] slots;
    @Children private final HaraExpressionNode[] initializers;
    @Child private HaraExpressionNode body;

    public Loop(
        RecurTarget target,
        int[] slots,
        HaraExpressionNode[] initializers,
        HaraExpressionNode body) {
      this.target = target;
      this.slots = slots;
      this.initializers = initializers;
      this.body = body;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      for (int i = 0; i < initializers.length; i++) {
        frame.setObject(slots[i], initializers[i].execute(frame));
      }
      int recurrences = 0;
      while (true) {
        try {
          return body.execute(frame);
        } catch (RecurException recur) {
          if (recur.target != target) {
            throw recur;
          }
          recurrences++;
          LoopNode.reportLoopCount(this, recurrences);
        }
      }
    }
  }

  public static final class Recur extends HaraExpressionNode {
    private final RecurTarget target;
    @Children private final HaraExpressionNode[] values;

    public Recur(RecurTarget target, HaraExpressionNode[] values) {
      this.target = target;
      this.values = values;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      // Evaluate every recurrence value into the loop's scratch slots before touching the
      // binding slots, so expressions such as (recur (+ i 1) (+ acc i)) observe the current
      // bindings; no per-iteration array or exception state is allocated.
      int[] scratchSlots = target.scratchSlots();
      for (int i = 0; i < values.length; i++) {
        frame.setObject(scratchSlots[i], values[i].execute(frame));
      }
      int[] slots = target.slots();
      for (int i = 0; i < slots.length; i++) {
        frame.copy(scratchSlots[i], slots[i]);
      }
      throw target.signal();
    }
  }

  public static final class Throw extends HaraExpressionNode {
    @Child private HaraExpressionNode value;

    public Throw(HaraExpressionNode value) {
      this.value = value;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      Object error = value.execute(frame);
      if (error instanceof IExInfo) {
        if (error instanceof Ex.Info info) {
          com.oracle.truffle.api.source.SourceSection source = getSourceSection();
          info.recordThrow(
              new Ex.Info.Site(
                  HaraLanguage.currentContext(this).currentNamespaceName(),
                  source == null ? null : source.getSource().getName(),
                  source == null ? 0 : source.getStartLine(),
                  source == null ? 0 : source.getStartColumn()));
        }
        throw new ThrownValue(error);
      }
      if (error instanceof RuntimeException runtime) {
        throw runtime;
      }
      if (error instanceof Error fatal) {
        throw fatal;
      }
      throw new HaraException("throw expects an Exception value created by ex", this);
    }
  }

  public static final class Try extends HaraExpressionNode {
    @Child private HaraExpressionNode body;
    @Children private final CatchClause[] catches;
    @Child private HaraExpressionNode finallyBody;

    public static final class CatchClause extends HaraExpressionNode {
      private final Object selector;
      private final int catchSlot;
      @Child private HaraExpressionNode body;

      public CatchClause(Object selector, int catchSlot, HaraExpressionNode body) {
        this.selector = selector;
        this.catchSlot = catchSlot;
        this.body = body;
      }

      private boolean matches(Object value) {
        if (selector == null) return true;
        if (selector instanceof Keyword keyword) {
          return keyword.equals(errorCode(value));
        }
        if (selector instanceof ILinearType selectors) {
          Object code = errorCode(value);
          for (int index = 0; index < selectors.count(); index++) {
            if (selectors.nth(index).equals(code)) return true;
          }
          return false;
        }
        Symbol type = (Symbol) selector;
        String typeName = type.getName();
        if ("Object".equals(typeName)
            || "Exception".equals(typeName)
            || "Throwable".equals(typeName)) {
          return true;
        }
        if (value instanceof HaraStruct struct) {
          String actual = struct.type().name();
          return typeName.equals(actual) || actual.endsWith("/" + typeName);
        }
        if (value instanceof HaraMutable mutable) {
          String actual = mutable.type().name();
          return typeName.equals(actual) || actual.endsWith("/" + typeName);
        }
        if ("Number".equals(typeName)) return value instanceof Number;
        if ("String".equals(typeName)) return value instanceof String;
        if ("Boolean".equals(typeName)) return value instanceof Boolean;
        if ("Long".equals(typeName)) return value instanceof Long;
        if ("Integer".equals(typeName)) return value instanceof Integer;
        if ("Double".equals(typeName)) return value instanceof Double;
        if ("BigInteger".equals(typeName)) return value instanceof java.math.BigInteger;
        if (value instanceof Throwable) {
          return HaraLanguage.currentContext(this).matchesNativeThrowable(type, (Throwable) value);
        }
        return false;
      }

      private static Object errorCode(Object value) {
        Object data = value instanceof IExInfo info ? info.getData() : null;
        return data instanceof IMapType map
            ? map.lookup(Keyword.create("ex", "code"))
            : null;
      }

      @Override
      public Object execute(VirtualFrame frame) {
        return body.execute(frame);
      }

      private Object executeCatch(VirtualFrame frame, Object value) {
        frame.setObject(catchSlot, value);
        return body.execute(frame);
      }
    }

    public Try(HaraExpressionNode body, CatchClause[] catches, HaraExpressionNode finallyBody) {
      this.body = body;
      this.catches = catches;
      this.finallyBody = finallyBody;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      try {
        return body.execute(frame);
      } catch (ThrownValue thrown) {
        for (CatchClause clause : catches) {
          if (clause.matches(thrown.value)) {
            return clause.executeCatch(frame, thrown.value);
          }
        }
        throw thrown;
      } catch (hara.kernel.flavor.NativeFlavorException failure) {
        Throwable cause = failure.getCause() == null ? failure : failure.getCause();
        for (CatchClause clause : catches) {
          if (clause.matches(cause)) {
            return clause.executeCatch(frame, cause);
          }
        }
        throw failure;
      } catch (HaraException failure) {
        for (CatchClause clause : catches) {
          if (clause.matches(failure)) {
            return clause.executeCatch(frame, failure);
          }
        }
        throw failure;
      } catch (RuntimeException failure) {
        if (failure instanceof RecurException) throw failure;
        Object guestFailure = HbcMachine.guestThrownValue(failure);
        if (guestFailure != failure) {
          for (CatchClause clause : catches) {
            if (clause.matches(guestFailure)) {
              return clause.executeCatch(frame, guestFailure);
            }
          }
          throw failure;
        }
        for (CatchClause clause : catches) {
          if (clause.matches(failure)) {
            return clause.executeCatch(frame, failure);
          }
        }
        throw failure;
      } finally {
        if (finallyBody != null) {
          finallyBody.execute(frame);
        }
      }
    }
  }

  public static final class DefineGlobal extends HaraExpressionNode {
    private final Symbol symbol;
    @Child private HaraExpressionNode initializer;

    public DefineGlobal(Symbol symbol, HaraExpressionNode initializer) {
      this.symbol = symbol;
      this.initializer = initializer;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      return HaraLanguage.currentContext(this).define(symbol, initializer.execute(frame));
    }
  }

  public static final class DefineNamedType extends HaraExpressionNode {
    private final Symbol symbol;
    private final HalcSchema.NamedField[] fields;
    private final boolean mutable;
    @Children private final HaraExpressionNode[] extensions;

    public DefineNamedType(
        Symbol symbol,
        HalcSchema.NamedField[] fields,
        boolean mutable,
        HaraExpressionNode[] extensions) {
      this.symbol = symbol;
      this.fields = fields.clone();
      this.mutable = mutable;
      this.extensions = extensions;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      HaraContext context = HaraLanguage.currentContext(this);
      return context.withDeclarationTransaction(
          () -> {
            Object result = context.defineNamedType(symbol, fields, mutable);
            for (HaraExpressionNode extension : extensions) {
              result = extension.execute(frame);
            }
            return result;
          });
    }
  }

  public static final class MacroExpand extends HaraExpressionNode {
    @Child private HaraExpressionNode form;
    private final boolean recursive;

    public MacroExpand(HaraExpressionNode form, boolean recursive) {
      this.form = form;
      this.recursive = recursive;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      return HaraLanguage.currentContext(this).macroExpand(form.execute(frame), recursive);
    }
  }

  public static final class Require extends HaraExpressionNode {
    @Child private HaraExpressionNode path;
    @Child private HaraExpressionNode options;

    public Require(HaraExpressionNode path, HaraExpressionNode options) {
      this.path = path;
      this.options = options;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      Object pathValue = path.execute(frame);
      Object optionsValue = options.execute(frame);
      return HaraLanguage.currentContext(this)
          .requireModule(
              optionsValue == null
                  ? new Object[] {pathValue}
                  : new Object[] {pathValue, optionsValue});
    }
  }

  public static final class Declare extends HaraExpressionNode {
    private final Symbol[] symbols;

    public Declare(Symbol[] symbols) {
      this.symbols = symbols;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      HaraContext context = HaraLanguage.currentContext(this);
      for (Symbol symbol : symbols) context.define(symbol, null);
      return null;
    }
  }

  public static final class DefineMulti extends HaraExpressionNode {
    private final Symbol symbol;
    @Child private HaraExpressionNode dispatch;

    public DefineMulti(Symbol symbol, HaraExpressionNode dispatch) {
      this.symbol = symbol;
      this.dispatch = dispatch;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      Object value = dispatch.execute(frame);
      if (!(value instanceof HaraFunction)
          && !InteropLibrary.getUncached(value).isExecutable(value)) {
        throw new HaraException("defmulti dispatch function must be a function", this);
      }
      HaraContext context = HaraLanguage.currentContext(this);
      return context.define(symbol, new HaraMultiFunction(context, value));
    }
  }

  public static final class DefineMethod extends HaraExpressionNode {
    private final Symbol symbol;
    @Child private HaraExpressionNode dispatchValue;
    @Child private HaraExpressionNode function;

    public DefineMethod(
        Symbol symbol, HaraExpressionNode dispatchValue, HaraExpressionNode function) {
      this.symbol = symbol;
      this.dispatchValue = dispatchValue;
      this.function = function;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      HaraContext context = HaraLanguage.currentContext(this);
      HaraVar var = context.resolve(symbol);
      if (var == null || !(var.get() instanceof HaraMultiFunction)) {
        throw defmultiError();
      }
      Object method = function.execute(frame);
      if (!(method instanceof HaraFunction)
          && !InteropLibrary.getUncached(method).isExecutable(method)) {
        throw new HaraException("defmethod body did not produce a function", this);
      }
      ((HaraMultiFunction) var.get()).addMethod(dispatchValue.execute(frame), method);
      return var;
    }

    @TruffleBoundary
    private HaraException defmultiError() {
      return new HaraException(
          "defmethod requires an existing defmulti: " + symbol.getName(), this);
    }
  }

  public static final class SetNamespace extends HaraExpressionNode {
    private final Symbol symbol;
    private final Object[] clauses;

    public SetNamespace(Symbol symbol, Object[] clauses) {
      this.symbol = symbol;
      this.clauses = clauses;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      HaraLanguage.currentContext(this).setCurrentNamespace(symbol, clauses);
      return symbol;
    }
  }

  public static final class SetAnonymousNamespace extends HaraExpressionNode {
    private final Symbol symbol;
    private final Object[] clauses;

    public SetAnonymousNamespace(Symbol symbol, Object[] clauses) {
      this.symbol = symbol;
      this.clauses = clauses;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      HaraLanguage.currentContext(this).setCurrentNamespace(symbol, clauses);
      return null;
    }
  }

  public static final class DefineAlias extends HaraExpressionNode {
    private final Symbol alias;
    private final Symbol target;

    public DefineAlias(Symbol alias, Symbol target) {
      this.alias = alias;
      this.target = target;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      HaraLanguage.currentContext(this).defineAlias(alias, target);
      return alias;
    }
  }

  public static final class DefineProtocol extends HaraExpressionNode {
    private final Symbol symbol;
    private final HaraProtocol protocol;

    public DefineProtocol(Symbol symbol, HaraProtocol protocol) {
      this.symbol = symbol;
      this.protocol = protocol;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      return HaraLanguage.currentContext(this).defineLanguageProtocol(symbol, protocol);
    }
  }

  public static final class ProtocolMethodImplementation {
    private final String name;
    private final HaraExpressionNode function;

    public ProtocolMethodImplementation(String name, HaraExpressionNode function) {
      this.name = name;
      this.function = function;
    }
  }

  public static final class ExtendType extends HaraExpressionNode {
    @Child private HaraExpressionNode type;
    @Child private HaraExpressionNode protocol;
    @Children private final HaraExpressionNode[] functions;
    private final String[] names;

    public ExtendType(
        HaraExpressionNode type,
        HaraExpressionNode protocol,
        ProtocolMethodImplementation[] implementations) {
      this.type = type;
      this.protocol = protocol;
      this.functions = new HaraExpressionNode[implementations.length];
      this.names = new String[implementations.length];
      for (int i = 0; i < implementations.length; i++) {
        functions[i] = implementations[i].function;
        names[i] = implementations[i].name;
      }
    }

    @Override
    public Object execute(VirtualFrame frame) {
      Object typeValue = type.execute(frame);
      Object protocolValue = protocol.execute(frame);
      if (!(typeValue instanceof HaraType)) {
        throw new HaraException("extend-type expects a named value type", this);
      }
      if (!(protocolValue instanceof HaraProtocol)) {
        throw new HaraException("extend-type expects a protocol", this);
      }
      HaraFunction[] methodFunctions = new HaraFunction[functions.length];
      for (int i = 0; i < functions.length; i++) {
        Object functionValue = functions[i].execute(frame);
        if (!(functionValue instanceof HaraFunction)) {
          throw new HaraException("protocol implementation must be a function", this);
        }
        methodFunctions[i] = (HaraFunction) functionValue;
      }
      HaraProtocol haraProtocol = (HaraProtocol) protocolValue;
      for (int i = 0; i < names.length; i++) {
        haraProtocol.extend((HaraType) typeValue, names[i], methodFunctions[i]);
      }
      return haraProtocol;
    }
  }

  public static final class ReadField extends HaraExpressionNode {
    @Child private HaraExpressionNode target;
    private final String field;

    public ReadField(HaraExpressionNode target, String field) {
      this.target = target;
      this.field = field;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      Object value = target.execute(frame);
      if (!(value instanceof HaraMutable mutable)) {
        throw new HaraException(
            "field expects a mutable value: "
                + field
                + " on "
                + (value == null ? "nil" : value.getClass().getName()),
            this);
      }
      try {
        return readMutableField(mutable);
      } catch (com.oracle.truffle.api.interop.UnknownIdentifierException exception) {
        throw unknownFieldError();
      }
    }

    @TruffleBoundary
    private HaraException unknownFieldError() {
      return new HaraException("Unknown mutable field: " + field, this);
    }

    @TruffleBoundary
    private Object readMutableField(HaraMutable mutable)
        throws com.oracle.truffle.api.interop.UnknownIdentifierException {
      return mutable.read(field);
    }
  }

  public static final class HostSymbol extends HaraExpressionNode {
    private final String name;

    public HostSymbol(String name) {
      this.name = name;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      requireHostInterop(this);
      try {
        return HaraLanguage.currentContext(this).lookupHostSymbol(name);
      } catch (RuntimeException exception) {
        throw hostSymbolError();
      }
    }

    @TruffleBoundary
    private HaraException hostSymbolError() {
      return new HaraException("Unable to resolve host symbol " + name, this);
    }
  }

  public static final class HostGet extends HaraExpressionNode {
    @Child private HaraExpressionNode target;
    private final String member;

    public HostGet(HaraExpressionNode target, String member) {
      this.target = target;
      this.member = member;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      requireHostInterop(this);
      Object targetValue = target.execute(frame);
      try {
        return HaraLanguage.currentContext(this)
            .asGuestValue(InteropLibrary.getUncached().readMember(targetValue, member));
      } catch (UnsupportedMessageException | UnknownIdentifierException exception) {
        throw hostGetError();
      }
    }

    @TruffleBoundary
    private HaraException hostGetError() {
      return new HaraException("Unable to read host member " + member, this);
    }
  }

  public static final class HostCall extends HaraExpressionNode {
    @Child private HaraExpressionNode target;
    @Children private final HaraExpressionNode[] arguments;
    private final String member;

    public HostCall(HaraExpressionNode target, String member, HaraExpressionNode[] arguments) {
      this.target = target;
      this.member = member;
      this.arguments = arguments;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      requireHostInterop(this);
      Object targetValue = target.execute(frame);
      Object[] values = new Object[arguments.length];
      for (int i = 0; i < arguments.length; i++) {
        values[i] = arguments[i].execute(frame);
      }
      try {
        return HaraLanguage.currentContext(this)
            .asGuestValue(InteropLibrary.getUncached().invokeMember(targetValue, member, values));
      } catch (UnsupportedMessageException
          | UnknownIdentifierException
          | UnsupportedTypeException
          | ArityException exception) {
        throw hostCallError();
      }
    }

    @TruffleBoundary
    private HaraException hostCallError() {
      return new HaraException("Unable to call host member " + member, this);
    }
  }

  public static final class NativeConstruct extends HaraExpressionNode {
    @Child private HaraExpressionNode type;
    @Children private final HaraExpressionNode[] arguments;

    public NativeConstruct(HaraExpressionNode type, HaraExpressionNode[] arguments) {
      this.type = type;
      this.arguments = arguments;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      Object[] values = new Object[arguments.length];
      for (int i = 0; i < arguments.length; i++) values[i] = arguments[i].execute(frame);
      return HaraLanguage.currentContext(this).constructNative(type.execute(frame), values);
    }
  }

  public static final class NativeReadMember extends HaraExpressionNode {
    @Child private HaraExpressionNode receiver;
    private final String member;

    public NativeReadMember(HaraExpressionNode receiver, String member) {
      this.receiver = receiver;
      this.member = member;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      return HaraLanguage.currentContext(this).readNativeMember(receiver.execute(frame), member);
    }
  }

  public static final class NativeIndex extends HaraExpressionNode {
    @Child private HaraExpressionNode receiver;
    @Child private HaraExpressionNode index;

    public NativeIndex(HaraExpressionNode receiver, HaraExpressionNode index) {
      this.receiver = receiver;
      this.index = index;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      return HaraLanguage.currentContext(this)
          .indexNative(receiver.execute(frame), index.execute(frame));
    }
  }

  public static final class MarkerCall extends HaraExpressionNode {
    @Child private HaraExpressionNode receiver;
    @Children private final HaraExpressionNode[] arguments;
    private final String method;

    public MarkerCall(HaraExpressionNode receiver, String method, HaraExpressionNode[] arguments) {
      this.receiver = receiver;
      this.method = method;
      this.arguments = arguments;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      Object target = receiver.execute(frame);
      Object[] values = new Object[arguments.length];
      for (int i = 0; i < arguments.length; i++) values[i] = arguments[i].execute(frame);
      return HaraLanguage.currentContext(this).invokeMarkerMethod(target, method, values);
    }
  }

  private static void requireHostInterop(HaraExpressionNode location) {
    if (!HaraLanguage.currentContext(location).hostInteropAllowed()) {
      throw new HaraException("Host interop is disabled for this Hara context", location);
    }
  }

  public static final class Add extends HaraExpressionNode {
    @Child private HaraExpressionNode left;
    @Child private HaraExpressionNode right;

    public Add(HaraExpressionNode left, HaraExpressionNode right) {
      this.left = left;
      this.right = right;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      Object leftValue = left.execute(frame);
      Object rightValue = right.execute(frame);
      if (isLongLike(leftValue) && isLongLike(rightValue)) {
        try {
          return Math.addExact(asLong(leftValue), asLong(rightValue));
        } catch (ArithmeticException overflow) {
          CompilerDirectives.transferToInterpreterAndInvalidate();
          return addPromoted(leftValue, rightValue);
        }
      }
      if (leftValue instanceof Number && rightValue instanceof Number) {
        CompilerDirectives.transferToInterpreterAndInvalidate();
        return addGeneric(leftValue, rightValue);
      }
      throw new HaraException("+ expects two numbers", this);
    }

    private static boolean isLongLike(Object value) {
      return value instanceof Byte
          || value instanceof Short
          || value instanceof Integer
          || value instanceof Long;
    }

    private static long asLong(Object value) {
      if (value instanceof Long) return (Long) value;
      if (value instanceof Integer) return (Integer) value;
      if (value instanceof Short) return (Short) value;
      return (Byte) value;
    }

    @TruffleBoundary
    private static Number addPromoted(Object left, Object right) {
      return Num.addP(left, right);
    }

    @TruffleBoundary
    private static Number addGeneric(Object left, Object right) {
      return Num.addP(left, right);
    }

  }

  public static final class Numeric extends HaraExpressionNode {
    public enum Operator {
      SUBTRACT,
      MULTIPLY,
      DIVIDE,
      REMAINDER,
      MODULO;

      private String symbol() {
        switch (this) {
          case SUBTRACT:
            return "-";
          case MULTIPLY:
            return "*";
          case DIVIDE:
            return "/";
          case REMAINDER:
            return "rem";
          case MODULO:
            return "mod";
          default:
            throw unsupportedOperator(this);
        }
      }

      private Number applyLong(long left, long right) {
        switch (this) {
          case SUBTRACT:
            try {
              return Math.subtractExact(left, right);
            } catch (ArithmeticException overflow) {
              return subtractPromoted(left, right);
            }
          case MULTIPLY:
            try {
              return Math.multiplyExact(left, right);
            } catch (ArithmeticException overflow) {
              return multiplyPromoted(left, right);
            }
          case DIVIDE:
            return divideLongs(left, right);
          case REMAINDER:
            return remainderLongs(left, right);
          case MODULO:
            return moduloLongs(left, right);
          default:
            throw unsupportedOperator(this);
        }
      }

      @TruffleBoundary
      private Number subtractPromoted(long left, long right) {
        return Num.minusP(left, right);
      }

      @TruffleBoundary
      private Number multiplyPromoted(long left, long right) {
        return Num.multiplyP(left, right);
      }

      @TruffleBoundary
      private Number divideLongs(long left, long right) {
        return Num.divide(left, right);
      }

      @TruffleBoundary
      private Number remainderLongs(long left, long right) {
        return Num.remainder(left, right);
      }

      @TruffleBoundary
      private Number moduloLongs(long left, long right) {
        return Num.mod(left, right);
      }

      @TruffleBoundary
      private Number applyGeneric(Object left, Object right) {
        switch (this) {
          case SUBTRACT:
            return Num.minusP(left, right);
          case MULTIPLY:
            return Num.multiplyP(left, right);
          case DIVIDE:
            return Num.divide(left, right);
          case REMAINDER:
            return Num.remainder(left, right);
          case MODULO:
            return Num.mod(left, right);
          default:
            throw unsupportedOperator(this);
        }
      }
    }

    @Child private HaraExpressionNode left;
    @Child private HaraExpressionNode right;
    private final Operator operator;

    public Numeric(Operator operator, HaraExpressionNode left, HaraExpressionNode right) {
      this.operator = operator;
      this.left = left;
      this.right = right;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      Object leftValue = left.execute(frame);
      Object rightValue = right.execute(frame);
      if (!(leftValue instanceof Number) || !(rightValue instanceof Number)) {
        throw numericTypeError();
      }
      if (isLongLike(leftValue) && isLongLike(rightValue)) {
        long leftLong = asLong(leftValue);
        long rightLong = asLong(rightValue);
        return operator.applyLong(leftLong, rightLong);
      }
      CompilerDirectives.transferToInterpreterAndInvalidate();
      return operator.applyGeneric(leftValue, rightValue);
    }

    @TruffleBoundary
    private HaraException numericTypeError() {
      return new HaraException(operator.symbol() + " expects two numbers", this);
    }

    private static boolean isLongLike(Object value) {
      return value instanceof Byte
          || value instanceof Short
          || value instanceof Integer
          || value instanceof Long;
    }

    private static long asLong(Object value) {
      if (value instanceof Long) return (Long) value;
      if (value instanceof Integer) return (Integer) value;
      if (value instanceof Short) return (Short) value;
      return (Byte) value;
    }
  }

  @TruffleBoundary
  private static boolean eqValues(Object left, Object right) {
    return Eq.eq(HaraBox.unwrap(left), HaraBox.unwrap(right));
  }

  @TruffleBoundary
  private static AssertionError unsupportedOperator(Object operator) {
    return new AssertionError(operator);
  }

  public static final class Compare extends HaraExpressionNode {
    public enum Operator {
      LESS,
      LESS_OR_EQUAL,
      GREATER,
      GREATER_OR_EQUAL,
      EQUAL,
      NOT_EQUAL
    }

    @Child private HaraExpressionNode left;
    @Child private HaraExpressionNode right;
    private final Operator operator;

    public Compare(Operator operator, HaraExpressionNode left, HaraExpressionNode right) {
      this.operator = operator;
      this.left = left;
      this.right = right;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      Object leftValue = left.execute(frame);
      Object rightValue = right.execute(frame);
      if (operator == Operator.EQUAL || operator == Operator.NOT_EQUAL) {
        boolean equal = eqValues(leftValue, rightValue);
        return operator == Operator.EQUAL ? equal : !equal;
      }
      if (!(leftValue instanceof Number) || !(rightValue instanceof Number)) {
        throw new HaraException("comparison expects two numbers", this);
      }
      int comparison = compareNumbers((Number) leftValue, (Number) rightValue);
      switch (operator) {
        case LESS:
          return comparison < 0;
        case LESS_OR_EQUAL:
          return comparison <= 0;
        case GREATER:
          return comparison > 0;
        case GREATER_OR_EQUAL:
          return comparison >= 0;
        default:
          throw unsupportedOperator(operator);
      }
    }

    static int compareNumbers(Number left, Number right) {
      if (isLongLike(left) && isLongLike(right)) {
        return Long.compare(asLong(left), asLong(right));
      }
      return compareGeneric(left, right);
    }

    private static boolean isLongLike(Object value) {
      return value instanceof Byte
          || value instanceof Short
          || value instanceof Integer
          || value instanceof Long;
    }

    private static long asLong(Object value) {
      if (value instanceof Long) return (Long) value;
      if (value instanceof Integer) return (Integer) value;
      if (value instanceof Short) return (Short) value;
      return (Byte) value;
    }

    @TruffleBoundary
    private static int compareGeneric(Number left, Number right) {
      return Num.compare(left, right);
    }
  }

  public static final class CompareChain extends HaraExpressionNode {
    @Children private final HaraExpressionNode[] values;
    private final Compare.Operator operator;

    public CompareChain(Compare.Operator operator, HaraExpressionNode[] values) {
      this.operator = operator;
      this.values = values;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      Object previous = values[0].execute(frame);
      for (int i = 1; i < values.length; i++) {
        Object current = values[i].execute(frame);
        boolean matches;
        if (operator == Compare.Operator.EQUAL || operator == Compare.Operator.NOT_EQUAL) {
          boolean equal = eqValues(previous, current);
          matches = operator == Compare.Operator.EQUAL ? equal : !equal;
        } else {
          if (!(previous instanceof Number) || !(current instanceof Number)) {
            throw new HaraException("comparison expects two numbers", this);
          }
          int comparison = Compare.compareNumbers((Number) previous, (Number) current);
          switch (operator) {
            case LESS:
              matches = comparison < 0;
              break;
            case LESS_OR_EQUAL:
              matches = comparison <= 0;
              break;
            case GREATER:
              matches = comparison > 0;
              break;
            case GREATER_OR_EQUAL:
              matches = comparison >= 0;
              break;
            default:
              throw unsupportedOperator(operator);
          }
        }
        if (!matches) return operator == Compare.Operator.NOT_EQUAL;
        previous = current;
      }
      return operator != Compare.Operator.NOT_EQUAL;
    }
  }

  public static final class FunctionLiteral extends HaraExpressionNode {
    private final RootCallTarget callTarget;
    private final int minimumArity;
    private final boolean variadic;
    private final boolean captures;

    @CompilerDirectives.CompilationFinal private HaraFunction closureFreeInstance;

    public FunctionLiteral(
        RootCallTarget callTarget, int minimumArity, boolean variadic, boolean captures) {
      this.callTarget = callTarget;
      this.minimumArity = minimumArity;
      this.variadic = variadic;
      this.captures = captures;
    }

    public HaraFunction instantiateWithoutClosure() {
      if (captures) {
        throw new HaraException("Top-level compiled function unexpectedly captures lexical state");
      }
      return new HaraFunction(callTarget, minimumArity, variadic, null);
    }

    @Override
    public Object execute(VirtualFrame frame) {
      if (!captures) {
        // Closure-free functions are immutable, so one instance serves every execution
        // and call sites keep a stable wrapper for their direct-call caches.
        HaraFunction instance = closureFreeInstance;
        if (instance == null) {
          CompilerDirectives.transferToInterpreterAndInvalidate();
          instance = new HaraFunction(callTarget, minimumArity, variadic, null);
          closureFreeInstance = instance;
        }
        return instance;
      }
      return new HaraFunction(callTarget, minimumArity, variadic, frame.materialize());
    }
  }

  public static final class MultiFunction extends HaraExpressionNode {
    @Children private final HaraExpressionNode[] alternatives;

    public MultiFunction(HaraExpressionNode[] alternatives) {
      this.alternatives = alternatives;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      HaraFunction[] functions = new HaraFunction[alternatives.length];
      for (int i = 0; i < alternatives.length; i++) {
        Object value = alternatives[i].execute(frame);
        if (!(value instanceof HaraFunction)) {
          throw new HaraException("Multi-arity clause did not produce a function", this);
        }
        functions[i] = (HaraFunction) value;
      }
      return new HaraFunction(functions);
    }
  }

  public static final class Invoke extends HaraExpressionNode {
    @Child private HaraExpressionNode function;
    @Children private final HaraExpressionNode[] arguments;
    @Child private DirectCallNode directCall;
    @Child private IndirectCallNode indirectCall = IndirectCallNode.create();

    @CompilerDirectives.CompilationFinal private RootCallTarget cachedCallTarget;

    public Invoke(HaraExpressionNode function, HaraExpressionNode[] arguments) {
      this.function = function;
      this.arguments = arguments;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      Object target = function.execute(frame);
      if (target instanceof HaraVar variable) target = variable.deref();
      if (target instanceof HaraMultiFunction) {
        return invokeMultiFunction((HaraMultiFunction) target, evaluateArguments(frame));
      }
      if (target instanceof HaraStruct || target instanceof HaraMutable) {
        return invokeViaProtocol(target, evaluateArguments(frame));
      }
      if (target instanceof HaraBuiltinFunction) {
        // Builtins never implement ILookup/ISequentialLookupType/ISetType, so the IFn
        // protocol invoker always degrades to IFn.applyAsArray; calling it directly is
        // exactly equivalent and skips the boundary plus the dispatch round trip.
        return invokeBuiltin((HaraBuiltinFunction) target, evaluateArguments(frame));
      }
      if (target instanceof IFn) {
        return invokeViaProtocol(target, evaluateArguments(frame));
      }
      if (target instanceof HaraType) {
        HaraType haraType = (HaraType) target;
        if (arguments.length != haraType.arity()) {
          throw arityError(haraType.arity(), arguments.length, false);
        }
        Object[] values = evaluateArguments(frame);
        return constructNamedValue(haraType, values);
      }
      if (!(target instanceof HaraFunction)
          && HaraLanguage.currentContext(this).isFunctionValue(target)) {
        return HaraLanguage.currentContext(this).invokeCallable(target, evaluateArguments(frame));
      }
      if (!(target instanceof HaraFunction)) {
        throw notCallable(target);
      }
      HaraFunction haraFunction = (HaraFunction) target;
      HaraFunction selectedFunction = haraFunction.resolveArity(arguments.length);
      if (selectedFunction == null) {
        throw arityError(haraFunction.arity(), arguments.length, haraFunction.variadic());
      }

      Object[] values = evaluateArguments(frame);

      RootCallTarget selectedTarget = selectedFunction.callTarget();
      if (selectedTarget == cachedCallTarget) {
        // The current closure travels with the call arguments, so closures created
        // from the same literal still hit the direct-call cache.
        return directCall.call(selectedFunction.callArguments(values));
      }
      if (cachedCallTarget == null) {
        CompilerDirectives.transferToInterpreterAndInvalidate();
        cachedCallTarget = selectedTarget;
        directCall = insert(DirectCallNode.create(selectedTarget));
        return directCall.call(selectedFunction.callArguments(values));
      }
      return indirectCall.call(selectedTarget, selectedFunction.callArguments(values));
    }

    @TruffleBoundary
    private Object invokeBuiltin(HaraBuiltinFunction target, Object[] values) {
      try {
        Object result = target.apply(values);
        if (target.recordsExceptionCreation() && result instanceof Ex.Info info) {
          SourceSection source = getSourceSection();
          info.recordCreation(
              new Ex.Info.Site(
                  HaraLanguage.currentContext(this).currentNamespaceName(),
                  source == null ? null : source.getSource().getName(),
                  source == null ? 0 : source.getStartLine(),
                  source == null ? 0 : source.getStartColumn()));
        }
        return result;
      } catch (HaraException error) {
        if (error.haraLocation() != null) throw error;
        throw new HaraException(error.getMessage(), this);
      } catch (ClassCastException error) {
        throw new HaraException(error.getMessage(), this);
      }
    }

    @TruffleBoundary
    private Object invokeMultiFunction(HaraMultiFunction target, Object[] values) {
      return HaraBox.export(target.invoke(values));
    }

    @TruffleBoundary
    private Object invokeViaProtocol(Object target, Object[] values) {
      try {
        return HaraLanguage.currentContext(this).ifnProtocol().invoke("invoke", target, values);
      } catch (HaraException error) {
        if (error.haraLocation() != null) throw error;
        throw new HaraException(error.getMessage(), this);
      }
    }

    private Object[] evaluateArguments(VirtualFrame frame) {
      Object[] values = new Object[arguments.length];
      for (int i = 0; i < arguments.length; i++) {
        values[i] = arguments[i].execute(frame);
      }
      return values;
    }

    @TruffleBoundary
    private HaraException notCallable(Object value) {
      return new HaraException("Value is not callable: " + value, this);
    }

    @TruffleBoundary
    private HaraException arityError(int expected, int actual, boolean variadic) {
      String expectedText = variadic ? "at least " + expected : Integer.toString(expected);
      return new HaraException("Expected " + expectedText + " arguments, received " + actual, this);
    }
  }

  /**
   * Specialized node for the hot sequence operations {@code first} and {@code rest}. Emitted only
   * when the operator is not lexically shadowed and the call arity is one; on every execution the
   * operator var's current value is compared against the canonical std.foundation function
   * captured by the context at namespace (re)load, so redefining the var transparently reverts
   * the call site to a fully generic invocation. The fast path reproduces the foundation
   * definitions verbatim ({@code first} coerces the receiver through iter and takes or skips the
   * head; {@code rest} is exactly {@code (seq (iter-drop 1 value))}); receivers Iter cannot coerce
   * fall back to a plain invocation, which reproduces the exact unsupported-receiver error.
   */
  public static final class FirstRest extends HaraExpressionNode {
    public enum Kind {
      FIRST("first"),
      REST("rest");

      private final String functionName;

      Kind(String functionName) {
        this.functionName = functionName;
      }
    }

    private final Kind kind;
    private final Symbol symbol;
    @Child private HaraExpressionNode argument;
    @Child private DirectCallNode directCall;
    @Child private IndirectCallNode indirectCall = IndirectCallNode.create();

    @CompilerDirectives.CompilationFinal private RootCallTarget cachedCallTarget;

    public FirstRest(Kind kind, Symbol symbol, HaraExpressionNode argument) {
      this.kind = kind;
      this.symbol = symbol;
      this.argument = argument;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      HaraContext context = HaraLanguage.currentContext(this);
      Object value = argument.execute(frame);
      Object target = readOperator(context);
      Object canonical = context.intrinsicSequenceFunction(kind.functionName);
      if (canonical == null || target != canonical) {
        return invokeGeneric(target, new Object[] {value});
      }
      Object receiver = HaraBox.unwrap(value);
      Iterator<?> source;
      try {
        source = iteratorFor(receiver);
      } catch (RuntimeException error) {
        // Iter declined the receiver; dispatching through a plain invocation reproduces the
        // exact unsupported-receiver error the generic path would raise.
        return invokeGeneric(target, new Object[] {value});
      }
      if (kind == Kind.FIRST) {
        return source.hasNext() ? source.next() : null;
      }
      return context.restSequence(source);
    }

    /** Mirrors HaraContext.iterValue for the nil and string receivers it special-cases. */
    private static Iterator<?> iteratorFor(Object receiver) {
      if (receiver == null) return Iter.emptyIterator();
      if (receiver instanceof String) return Iter.codePoints((String) receiver);
      return Iter.iter(receiver);
    }

    private Object readOperator(HaraContext context) {
      if (context.hasNativeSymbol(symbol)) {
        return context.resolveNativeSymbol(symbol);
      }
      HaraVar var = context.resolve(symbol);
      if (var == null) {
        throw unboundOperator();
      }
      return var.deref();
    }

    @TruffleBoundary
    private HaraException unboundOperator() {
      return new HaraException("Unbound symbol: " + symbol.display(), this);
    }

    /** Mirrors {@link Invoke} for call sites whose operator no longer holds the canonical function. */
    private Object invokeGeneric(Object target, Object[] values) {
      if (target instanceof HaraMultiFunction) {
        return invokeMultiFunction((HaraMultiFunction) target, values);
      }
      if (target instanceof HaraStruct || target instanceof HaraMutable) {
        return invokeViaProtocol(target, values);
      }
      if (target instanceof HaraBuiltinFunction) {
        return ((HaraBuiltinFunction) target).apply(values);
      }
      if (target instanceof IFn) {
        return invokeViaProtocol(target, values);
      }
      if (target instanceof HaraType) {
        HaraType haraType = (HaraType) target;
        if (values.length != haraType.arity()) {
          throw arityError(haraType.arity(), values.length, false);
        }
        return constructNamedValue(haraType, values);
      }
      if (!(target instanceof HaraFunction)) {
        throw notCallable(target);
      }
      HaraFunction haraFunction = (HaraFunction) target;
      HaraFunction selectedFunction = haraFunction.resolveArity(values.length);
      if (selectedFunction == null) {
        throw arityError(haraFunction.arity(), values.length, haraFunction.variadic());
      }
      RootCallTarget selectedTarget = selectedFunction.callTarget();
      if (selectedTarget == cachedCallTarget) {
        return directCall.call(selectedFunction.callArguments(values));
      }
      if (cachedCallTarget == null) {
        CompilerDirectives.transferToInterpreterAndInvalidate();
        cachedCallTarget = selectedTarget;
        directCall = insert(DirectCallNode.create(selectedTarget));
        return directCall.call(selectedFunction.callArguments(values));
      }
      return indirectCall.call(selectedTarget, selectedFunction.callArguments(values));
    }

    @TruffleBoundary
    private Object invokeMultiFunction(HaraMultiFunction target, Object[] values) {
      return HaraBox.export(target.invoke(values));
    }

    @TruffleBoundary
    private Object invokeViaProtocol(Object target, Object[] values) {
      try {
        return HaraLanguage.currentContext(this).ifnProtocol().invoke("invoke", target, values);
      } catch (HaraException error) {
        if (error.haraLocation() != null) throw error;
        throw new HaraException(error.getMessage(), this);
      }
    }

    @TruffleBoundary
    private HaraException notCallable(Object value) {
      return new HaraException("Value is not callable: " + value, this);
    }

    @TruffleBoundary
    private HaraException arityError(int expected, int actual, boolean variadic) {
      String expectedText = variadic ? "at least " + expected : Integer.toString(expected);
      return new HaraException("Expected " + expectedText + " arguments, received " + actual, this);
    }
  }

  /**
   * Specialized node for the hot persistent-collection operations {@code get} and {@code nth}.
   * Emitted only when the operator is not lexically shadowed and the call arity
   * matches the protocol method; on every execution the operator var's current value is compared
   * against the canonical builtin installed by the context, so redefining the var transparently
   * reverts the call site to a fully generic invocation. The fast path applies only to the
   * runtime's own intrinsic protocol implementations (built-in persistent collections, byte
   * arrays, and nil); every other receiver dispatches through the protocol exactly as a plain
   * invocation would, preserving extend-type semantics and invalidation.
   */
  public static final class CollectionOp extends HaraExpressionNode {
    public enum Kind {
      GET("ILookup", "lookup"),
      NTH("INth", "nth"),
      ;

      private final String protocolName;
      private final String methodName;

      Kind(String protocolName, String methodName) {
        this.protocolName = protocolName;
        this.methodName = methodName;
      }
    }

    private final Kind kind;
    private final Symbol symbol;
    private final Symbol protocolSymbol;
    @Children private final HaraExpressionNode[] arguments;
    @Child private DirectCallNode directCall;
    @Child private IndirectCallNode indirectCall = IndirectCallNode.create();

    @CompilerDirectives.CompilationFinal private RootCallTarget cachedCallTarget;

    public CollectionOp(Kind kind, Symbol symbol, HaraExpressionNode[] arguments) {
      this.kind = kind;
      this.symbol = symbol;
      this.protocolSymbol = Symbol.create(kind.protocolName);
      this.arguments = arguments;
    }

    @Override
    public Object execute(VirtualFrame frame) {
      HaraContext context = HaraLanguage.currentContext(this);
      Object target = readOperator(context);
      Object canonical = context.intrinsicCollectionBuiltin(symbol.getName());
      if (canonical == null || target != canonical) {
        return invokeGeneric(target, evaluateArguments(frame));
      }
      Object[] values = evaluateArguments(frame);
      Object receiver = HaraBox.unwrap(values[0]);
      if (context.isHostObject(receiver)) {
        receiver = context.asHostObject(receiver);
      }
      HaraProtocol protocol = resolveProtocol(context);
      HaraProtocolImplementation implementation = protocol.implementation(receiver, kind.methodName);
      if (implementation != null && implementation.intrinsic() && intrinsicApplies(receiver)) {
        try {
          return invokeIntrinsic(receiver, values);
        } catch (Ex.Unsupported error) {
          // The intrinsic receiver declined the operation; dispatching through the protocol
          // reproduces the exact unsupported-receiver error a plain invocation would raise.
          return invokeProtocolGeneric(protocol, receiver, values);
        }
      }
      return invokeProtocolGeneric(protocol, receiver, values);
    }

    private boolean intrinsicApplies(Object receiver) {
      switch (kind) {
        case GET:
          return receiver == null
              || receiver instanceof hara.lang.protocol.ILookup
              || receiver instanceof hara.lang.protocol.ISetType
              || receiver instanceof byte[];
        case NTH:
          return receiver instanceof hara.lang.protocol.INth || receiver instanceof byte[];
        default:
          throw new AssertionError(kind);
      }
    }

    private Object invokeIntrinsic(Object receiver, Object[] values) {
      switch (kind) {
        case GET:
          return intrinsicGet(receiver, values);
        case NTH:
          return intrinsicNth(receiver, values);
        default:
          throw new AssertionError(kind);
      }
    }

    @SuppressWarnings("unchecked")
    private Object intrinsicGet(Object receiver, Object[] values) {
      if (receiver == null) {
        return values.length == 3 ? values[2] : null;
      }
      if (receiver instanceof hara.lang.protocol.ILookup) {
        if (receiver instanceof hara.lang.data.types.ISequentialLookupType<?> sequential) {
          long index = sequentialIndex(values[1]);
          if (index >= sequential.count()) return values.length == 3 ? values[2] : null;
          return sequential.nth(index);
        }
        hara.lang.protocol.ILookup<Object, Object> lookup =
            (hara.lang.protocol.ILookup<Object, Object>) receiver;
        return values.length == 3 ? lookup.lookup(values[1], values[2]) : lookup.lookup(values[1]);
      }
      if (receiver instanceof hara.lang.protocol.ISetType<?> set) {
        Object found =
            ((hara.lang.protocol.IFind<Object, Object>) set).find(values[1]);
        return found == null && values.length == 3 ? values[2] : found;
      }
      byte[] bytes = (byte[]) receiver;
      if (!(values[1] instanceof Number)) {
        throw new HaraException(
            "ILookup/lookup on bytes expects an index and optional default", this);
      }
      long index = ((Number) values[1]).longValue();
      if (index < 0 || index >= bytes.length) {
        return values.length == 3 ? values[2] : null;
      }
      return bytes[(int) index];
    }

    private Object intrinsicNth(Object receiver, Object[] values) {
      long index = sequentialIndex(values[1]);
      if (receiver instanceof hara.lang.protocol.INth) {
        return ((hara.lang.protocol.INth<?>) receiver).nth(index);
      }
      byte[] bytes = (byte[]) receiver;
      if (index < 0 || index >= bytes.length) {
        throw new HaraException("byte index out of bounds: " + index, this);
      }
      return bytes[(int) index];
    }

    private long sequentialIndex(Object value) {
      Object index = HaraBox.unwrap(value);
      long exact;
      try {
        exact = hara.lang.base.NumUtils.toBigInteger(index).longValueExact();
      } catch (RuntimeException error) {
        throw new HaraException(
            "sequential lookup expects a non-negative integer index, received "
                + hara.lang.base.G.display(index),
            this);
      }
      if (exact < 0) {
        throw new HaraException(
            "sequential lookup expects a non-negative integer index, received "
                + hara.lang.base.G.display(index),
            this);
      }
      return exact;
    }


    private Object readOperator(HaraContext context) {
      if (context.hasNativeSymbol(symbol)) {
        return context.resolveNativeSymbol(symbol);
      }
      HaraVar var = context.resolve(symbol);
      if (var == null) {
        throw unboundOperator();
      }
      return var.deref();
    }

    @TruffleBoundary
    private HaraException unboundOperator() {
      return new HaraException("Unbound symbol: " + symbol.display(), this);
    }

    private HaraProtocol resolveProtocol(HaraContext context) {
      HaraVar variable = context.resolve(protocolSymbol);
      Object value = variable == null ? null : variable.get();
      if (!(value instanceof HaraProtocol)) {
        throw missingProtocol();
      }
      return (HaraProtocol) value;
    }

    @TruffleBoundary
    private HaraException missingProtocol() {
      return new HaraException("Missing protocol: " + kind.protocolName, this);
    }

    @TruffleBoundary
    private Object invokeProtocolGeneric(HaraProtocol protocol, Object receiver, Object[] values) {
      Object[] protocolArguments = new Object[values.length - 1];
      System.arraycopy(values, 1, protocolArguments, 0, protocolArguments.length);
      try {
        return protocol.invoke(kind.methodName, receiver, protocolArguments);
      } catch (HaraException error) {
        if (error.haraLocation() != null) throw error;
        throw new HaraException(error.getMessage(), this);
      }
    }

    /** Mirrors {@link Invoke} for call sites whose operator no longer holds the canonical builtin. */
    private Object invokeGeneric(Object target, Object[] values) {
      if (target instanceof HaraMultiFunction) {
        return invokeMultiFunction((HaraMultiFunction) target, values);
      }
      if (target instanceof HaraStruct || target instanceof HaraMutable) {
        return invokeViaProtocol(target, values);
      }
      if (target instanceof HaraBuiltinFunction) {
        return ((HaraBuiltinFunction) target).apply(values);
      }
      if (target instanceof IFn) {
        return invokeViaProtocol(target, values);
      }
      if (target instanceof HaraType) {
        HaraType haraType = (HaraType) target;
        if (values.length != haraType.arity()) {
          throw arityError(haraType.arity(), values.length, false);
        }
        return constructNamedValue(haraType, values);
      }
      if (!(target instanceof HaraFunction)) {
        throw notCallable(target);
      }
      HaraFunction haraFunction = (HaraFunction) target;
      HaraFunction selectedFunction = haraFunction.resolveArity(values.length);
      if (selectedFunction == null) {
        throw arityError(haraFunction.arity(), values.length, haraFunction.variadic());
      }
      RootCallTarget selectedTarget = selectedFunction.callTarget();
      if (selectedTarget == cachedCallTarget) {
        return directCall.call(selectedFunction.callArguments(values));
      }
      if (cachedCallTarget == null) {
        CompilerDirectives.transferToInterpreterAndInvalidate();
        cachedCallTarget = selectedTarget;
        directCall = insert(DirectCallNode.create(selectedTarget));
        return directCall.call(selectedFunction.callArguments(values));
      }
      return indirectCall.call(selectedTarget, selectedFunction.callArguments(values));
    }

    @TruffleBoundary
    private Object invokeMultiFunction(HaraMultiFunction target, Object[] values) {
      return HaraBox.export(target.invoke(values));
    }

    @TruffleBoundary
    private Object invokeViaProtocol(Object target, Object[] values) {
      try {
        return HaraLanguage.currentContext(this).ifnProtocol().invoke("invoke", target, values);
      } catch (HaraException error) {
        if (error.haraLocation() != null) throw error;
        throw new HaraException(error.getMessage(), this);
      }
    }

    private Object[] evaluateArguments(VirtualFrame frame) {
      Object[] values = new Object[arguments.length];
      for (int i = 0; i < arguments.length; i++) {
        values[i] = arguments[i].execute(frame);
      }
      return values;
    }

    @TruffleBoundary
    private HaraException notCallable(Object value) {
      return new HaraException("Value is not callable: " + value, this);
    }

    @TruffleBoundary
    private HaraException arityError(int expected, int actual, boolean variadic) {
      String expectedText = variadic ? "at least " + expected : Integer.toString(expected);
      return new HaraException("Expected " + expectedText + " arguments, received " + actual, this);
    }
  }

}
