package hara.truffle;

import hara.lang.base.Reduced;
import hara.lang.data.types.ISequentialLookupType;
import hara.lang.protocol.ILinearType;
import hara.lang.protocol.ISetType;
import hara.lang.protocol.IMapType;
import hara.lang.data.types.IVectorType;
import hara.lang.data.Keyword;
import hara.lang.data.List;
import hara.lang.data.Symbol;
import hara.lang.data.HaraCharacter;
import hara.lang.data.MapEntry;
import hara.lang.data.TaggedLiteral;
import hara.lang.data.Tuple;
import hara.lang.protocol.*;
import hara.lang.declaration.HaraProtocolExtension;
import hara.lang.declaration.HaraProtocolTarget;
import java.util.Arrays;
import java.util.Iterator;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.function.Consumer;

/** Annotated built-in protocol implementations and their semantic helpers. */
final class HaraProtocolExtensions {
  private HaraProtocolExtensions() {}

  @HaraProtocolExtension(protocol = IFn.class, method = "invoke", receiver = IFn.class)
  static Object invokeFunction(Object receiver, Object[] arguments) {
    return HaraFunctionDispatch.invoke(receiver, arguments);
  }

  @HaraProtocolExtension(protocol = IFn.class, method = "invoke", receiver = HaraFunction.class)
  @HaraProtocolExtension(
      protocol = IFn.class, method = "invoke", receiver = HaraMultiFunction.class)
  @HaraProtocolExtension(protocol = IFn.class, method = "invoke", receiver = HaraType.class)
  @HaraProtocolExtension(
      protocol = IFn.class, method = "invoke", receiver = hara.lang.data.Pointer.class)
  @HaraProtocolExtension(
      protocol = IFn.class, method = "invoke", receiver = HbcMachine.HbcClosure.class)
  @HaraProtocolExtension(
      protocol = IFn.class, method = "invoke", receiver = HbcMachine.HbcMultiArity.class)
  @HaraProtocolExtension(
      protocol = IFn.class, method = "invoke", receiver = HbcMachine.HbcNativeCallable.class)
  static Object invokeHaraCallable(
      HaraContext context, Object receiver, Object[] arguments) {
    return context.invokeCallable(
        receiver,
        Arrays.stream(arguments)
            .map(HaraProtocolExtensions::unwrapArgument)
            .toArray(Object[]::new));
  }

  @HaraProtocolExtension(
      protocol = IDisplay.class, method = "display", target = HaraProtocolTarget.STRING)
  static Object displayString(Object receiver, Object[] arguments) {
    return hara.lang.base.G.display((String) receiver);
  }

  @HaraProtocolExtension(
      protocol = IDisplay.class, method = "display", target = HaraProtocolTarget.CHARACTER)
  static Object displayCharacter(Object receiver, Object[] arguments) {
    return hara.lang.base.G.displayCharacter(receiver);
  }

  @HaraProtocolExtension(
      protocol = IDisplay.class, method = "display", target = HaraProtocolTarget.NUMBER)
  static Object displayNumber(Object receiver, Object[] arguments) {
    return hara.lang.base.G.display((Number) receiver);
  }

  @HaraProtocolExtension(
      protocol = IDisplay.class, method = "display", target = HaraProtocolTarget.BOOLEAN)
  static Object displayBoolean(Object receiver, Object[] arguments) {
    return hara.lang.base.G.display(receiver);
  }

  @HaraProtocolExtension(
      protocol = IDisplay.class, method = "display", target = HaraProtocolTarget.FOREIGN)
  static Object displayForeign(Object receiver, Object[] arguments) {
    return hara.lang.base.G.display(receiver);
  }

  static Object unwrapArgument(Object value) {
    Object unwrapped = HaraBox.unwrap(value);
    return HaraBox.isNil(unwrapped) ? null : unwrapped;
  }

  @HaraProtocolExtension(
      protocol = ILookup.class,
      method = "lookup",
      receiver = ILookup.class,
      intrinsic = true)
  static Object lookupProtocol(Object receiver, Object[] arguments) {
    return lookupValue((ILookup<?, ?>) receiver, arguments);
  }

  @HaraProtocolExtension(
      protocol = ILookup.class,
      method = "lookup",
      receiver = Tuple.Tup0.class,
      intrinsic = true)
  @HaraProtocolExtension(
      protocol = ILookup.class,
      method = "lookup",
      receiver = Tuple.Tup1.class,
      intrinsic = true)
  static Object lookupTupleProtocol(Object receiver, Object[] arguments) {
    return lookupTuple(receiver, arguments);
  }

  @HaraProtocolExtension(
      protocol = ILookup.class,
      method = "lookup",
      receiver = MapEntry.class,
      intrinsic = true)
  static Object lookupMapEntryProtocol(Object receiver, Object[] arguments) {
    return lookupMapEntry((MapEntry<?, ?>) receiver, arguments);
  }

  @HaraProtocolExtension(
      protocol = ILookup.class,
      method = "lookup",
      receiver = byte[].class,
      intrinsic = true)
  static Object lookupBytesProtocol(Object receiver, Object[] arguments) {
    return lookupBytes(receiver, arguments);
  }

  @HaraProtocolExtension(
      protocol = ILookup.class,
      method = "lookup",
      receiver = String.class,
      intrinsic = true)
  static Object lookupStringProtocol(Object receiver, Object[] arguments) {
    if (arguments.length < 1 || arguments.length > 2) {
      throw new HaraException("ILookup/lookup on strings expects an index and optional default");
    }
    long index = HaraNumericConversions.toLong(arguments[0], "ILookup/lookup on strings");
    String value = (String) receiver;
    if (index < 0 || index >= value.codePointCount(0, value.length())) {
      return arguments.length == 2 ? arguments[1] : null;
    }
    return HaraCharacter.of(value.codePointAt(value.offsetByCodePoints(0, Math.toIntExact(index))));
  }

  @HaraProtocolExtension(
      protocol = ILookup.class,
      method = "lookup",
      receiver = ISetType.class,
      intrinsic = true)
  static Object lookupSetProtocol(Object receiver, Object[] arguments) {
    return setValue((ISetType<?>) receiver, arguments);
  }

  @HaraProtocolExtension(
      protocol = ILookup.class,
      method = "lookup",
      target = HaraProtocolTarget.NIL,
      intrinsic = true)
  static Object lookupNil(Object receiver, Object[] arguments) {
    if (arguments.length < 1 || arguments.length > 2) {
      throw new HaraException("ILookup/lookup expects one or two arguments");
    }
    return arguments.length == 2 ? arguments[1] : null;
  }

  @HaraProtocolExtension(
      protocol = IAssoc.class,
      method = "assoc",
      receiver = IAssoc.class,
      intrinsic = true)
  static Object assocProtocol(Object receiver, Object[] arguments) {
    return assocValue((IAssoc<?, ?>) receiver, arguments);
  }

  @HaraProtocolExtension(
      protocol = IAssoc.class,
      method = "assoc",
      receiver = Tuple.Tup0.class,
      intrinsic = true)
  @HaraProtocolExtension(
      protocol = IAssoc.class,
      method = "assoc",
      receiver = Tuple.Tup1.class,
      intrinsic = true)
  static Object assocTupleProtocol(Object receiver, Object[] arguments) {
    return assocTuple(receiver, arguments);
  }

  @HaraProtocolExtension(
      protocol = IObjType.class,
      method = "with-meta",
      receiver = IObjType.class,
      intrinsic = true)
  static Object withMetaProtocol(Object receiver, Object[] arguments) {
    if (arguments.length != 1 || !(arguments[0] instanceof IMetadata metadata)) {
      throw new HaraException("IObjType/with-meta expects a metadata value");
    }
    return ((IObjType) receiver).withMeta(metadata);
  }

  @HaraProtocolExtension(
      protocol = IStringLike.class, method = "to-string", receiver = Keyword.class)
  static Object keywordToString(Object receiver, Object[] arguments) {
    Keyword keyword = (Keyword) receiver;
    return keyword.getNamespace() == null
        ? keyword.getName()
        : keyword.getNamespace() + "/" + keyword.getName();
  }

  @HaraProtocolExtension(
      protocol = IStringLike.class, method = "from-string", receiver = Keyword.class)
  static Object keywordFromString(Object receiver, Object[] arguments) {
    return Keyword.create(String.valueOf(arguments[0]));
  }

  @HaraProtocolExtension(
      protocol = IStringLike.class, method = "to-string", receiver = Symbol.class)
  static Object symbolToString(Object receiver, Object[] arguments) {
    return ((Symbol) receiver).pathString();
  }

  @HaraProtocolExtension(
      protocol = IStringLike.class, method = "from-string", receiver = Symbol.class)
  static Object symbolFromString(Object receiver, Object[] arguments) {
    return Symbol.create(String.valueOf(arguments[0]));
  }

  @HaraProtocolExtension(protocol = ICount.class, method = "count", receiver = String.class)
  static Object countString(Object receiver, Object[] arguments) {
    String value = (String) receiver;
    return (long) value.codePointCount(0, value.length());
  }

  @HaraProtocolExtension(protocol = ICount.class, method = "count", receiver = byte[].class)
  static Object countBytes(Object receiver, Object[] arguments) {
    return (long) ((byte[]) receiver).length;
  }

  @HaraProtocolExtension(
      protocol = ICount.class,
      method = "count",
      target = HaraProtocolTarget.NIL)
  static Object countNil(Object receiver, Object[] arguments) {
    return 0L;
  }

  @HaraProtocolExtension(protocol = IConj.class, method = "conj", receiver = IConj.class)
  static Object conjProtocol(Object receiver, Object[] arguments) {
    return conjValue((IConj<?>) receiver, arguments[0]);
  }

  @HaraProtocolExtension(
      protocol = IConj.class,
      method = "conj",
      target = HaraProtocolTarget.NIL)
  static Object conjNil(Object receiver, Object[] arguments) {
    return List.Standard.from(null, arguments[0]);
  }

  @HaraProtocolExtension(protocol = IFind.class, method = "find", receiver = IFind.class)
  static Object findProtocol(Object receiver, Object[] arguments) {
    return findValue((IFind<?, ?>) receiver, arguments[0]);
  }

  @HaraProtocolExtension(
      protocol = IFind.class,
      method = "find",
      receiver = Tuple.Tup0.class,
      intrinsic = true)
  @HaraProtocolExtension(
      protocol = IFind.class,
      method = "find",
      receiver = Tuple.Tup1.class,
      intrinsic = true)
  static Object findTupleProtocol(Object receiver, Object[] arguments) {
    return findTuple(receiver, arguments);
  }

  @HaraProtocolExtension(protocol = IEquality.class, method = "equality", receiver = byte[].class)
  static Object equalityBytes(Object receiver, Object[] arguments) {
    return arguments.length == 1
        && arguments[0] instanceof byte[]
        && Arrays.equals((byte[]) receiver, (byte[]) arguments[0]);
  }

  @HaraProtocolExtension(protocol = IHash.class, method = "hash", receiver = byte[].class)
  static Object hashBytes(Object receiver, Object[] arguments) {
    return (long) Arrays.hashCode((byte[]) receiver);
  }

  @HaraProtocolExtension(
      protocol = IDerefTimeout.class, method = "deref-timeout", receiver = IDerefTimeout.class)
  static Object derefTimeoutProtocol(Object receiver, Object[] arguments) {
    return derefTimeoutValue((IDerefTimeout<?>) receiver, arguments[0], arguments[1]);
  }

  @HaraProtocolExtension(
      protocol = INth.class,
      method = "nth",
      receiver = INth.class,
      intrinsic = true)
  static Object nthProtocol(Object receiver, Object[] arguments) {
    long index = HaraNumericConversions.toLong(arguments[0], "INth/nth");
    try {
      return ((INth<?>) receiver).nth(index);
    } catch (IndexOutOfBoundsException | java.util.NoSuchElementException error) {
      throw new HaraException("nth index out of bounds: " + index);
    }
  }

  @HaraProtocolExtension(
      protocol = INth.class,
      method = "nth",
      receiver = byte[].class,
      intrinsic = true)
  static Object nthBytes(Object receiver, Object[] arguments) {
    long index = HaraNumericConversions.toLong(arguments[0], "INth/nth");
    byte[] bytes = (byte[]) receiver;
    if (index < 0 || index >= bytes.length) {
      throw new HaraException("byte index out of bounds: " + index);
    }
    return bytes[(int) index];
  }

  @HaraProtocolExtension(protocol = INth.class, method = "nth", receiver = java.util.List.class)
  static Object nthList(Object receiver, Object[] arguments) {
    long index = HaraNumericConversions.toLong(arguments[0], "INth/nth");
    java.util.List<?> list = (java.util.List<?>) receiver;
    try {
      return list.get(Math.toIntExact(index));
    } catch (IndexOutOfBoundsException | ArithmeticException error) {
      throw new HaraException("nth index out of bounds: " + index);
    }
  }

  @HaraProtocolExtension(protocol = INth.class, method = "nth", receiver = Tuple.Tup0.class)
  @HaraProtocolExtension(protocol = INth.class, method = "nth", receiver = Tuple.Tup1.class)
  static Object nthTupleProtocol(Object receiver, Object[] arguments) {
    return nthTuple(receiver, arguments);
  }

  @HaraProtocolExtension(protocol = IEmpty.class, method = "empty", target = HaraProtocolTarget.NIL)
  static Object emptyNil(Object receiver, Object[] arguments) {
    return null;
  }

  @HaraProtocolExtension(
      protocol = IEncodable.class,
      method = "encode-with",
      target = HaraProtocolTarget.NIL)
  static Object encodeNil(HaraContext context, Object receiver, Object[] arguments) {
    return context.invokeProtocol("IEncodeVisitor", "visit-nil", arguments[0]);
  }

  @HaraProtocolExtension(
      protocol = IEncodable.class,
      method = "encode-with",
      target = HaraProtocolTarget.DEFAULT)
  static Object encodeDefault(HaraContext context, Object receiver, Object[] arguments) {
    Object visitor = arguments[0];
    if (receiver instanceof TaggedLiteral tagged) {
      return context.invokeProtocol(
          "IEncodeVisitor", "visit-tagged", visitor, tagged.tag(), tagged.form());
    }
    String method =
        receiver instanceof Boolean
            ? "visit-boolean"
            : receiver instanceof Number
                ? "visit-number"
                : receiver instanceof HaraCharacter || receiver instanceof Character
                    ? "visit-character"
                    : receiver instanceof String
                        ? "visit-string"
                        : receiver instanceof Keyword
                            ? "visit-keyword"
                            : receiver instanceof Symbol
                                ? "visit-symbol"
                                : receiver instanceof MapEntry
                                    ? "visit-unknown"
                                    : receiver instanceof IVectorType<?>
                                        || Tuple.isCompact(receiver)
                                    ? "visit-vector"
                                    : receiver instanceof IMapType<?, ?>
                                        ? "visit-map"
                                        : receiver instanceof ISetType<?>
                                            ? "visit-set"
                                            : receiver instanceof ISequential<?>
                                                ? "visit-seq"
                                                : "visit-unknown";
    return context.invokeProtocol("IEncodeVisitor", method, visitor, receiver);
  }

  @HaraProtocolExtension(protocol = ICons.class, method = "cons", receiver = hara.lang.data.Seq.class)
  @HaraProtocolExtension(protocol = ICons.class, method = "cons", receiver = hara.lang.data.Cons.class)
  @HaraProtocolExtension(protocol = ICons.class, method = "cons", receiver = hara.lang.data.Deque.class)
  @HaraProtocolExtension(protocol = ICons.class, method = "cons", receiver = hara.lang.data.Queue.class)
  @HaraProtocolExtension(protocol = ICons.class, method = "cons", receiver = List.class)
  @HaraProtocolExtension(protocol = ICons.class, method = "cons", receiver = hara.lang.data.Vector.class)
  @HaraProtocolExtension(protocol = ICons.class, method = "cons", receiver = Tuple.Tup0.class)
  @HaraProtocolExtension(protocol = ICons.class, method = "cons", receiver = Tuple.Tup1.class)
  @HaraProtocolExtension(protocol = ICons.class, method = "cons", receiver = ISequential.class)
  @HaraProtocolExtension(protocol = ICons.class, method = "cons", receiver = ICons.class)
  static Object consProtocol(Object receiver, Object[] arguments) {
    return receiver instanceof ISequential
        ? consSequential((ISequential<?>) receiver, arguments[0])
        : consValue((ICons<?>) receiver, arguments[0]);
  }

  @HaraProtocolExtension(
      protocol = ICons.class,
      method = "cons",
      target = HaraProtocolTarget.NIL)
  static Object consNil(Object receiver, Object[] arguments) {
    return new hara.lang.data.Cons<>(null, arguments[0], null);
  }

  @HaraProtocolExtension(protocol = IIter.class, method = "iter", receiver = Iterable.class)
  static Object iterIterable(Object receiver, Object[] arguments) {
    return ((Iterable<?>) receiver).iterator();
  }

  @HaraProtocolExtension(protocol = IIter.class, method = "iter", receiver = Iterator.class)
  static Object iterIterator(Object receiver, Object[] arguments) {
    return receiver;
  }

  @HaraProtocolExtension(protocol = IIter.class, method = "iter", receiver = String.class)
  static Object iterString(Object receiver, Object[] arguments) {
    return hara.lang.base.Iter.codePoints((String) receiver);
  }

  @HaraProtocolExtension(protocol = INth.class, method = "nth", receiver = String.class)
  static Object nthString(Object receiver, Object[] arguments) {
    long index = HaraNumericConversions.toLong(arguments[0], "INth/nth");
    String value = (String) receiver;
    if (index < 0 || index >= value.codePointCount(0, value.length())) {
      throw new HaraException("nth index out of bounds: " + index);
    }
    return HaraCharacter.of(value.codePointAt(value.offsetByCodePoints(0, Math.toIntExact(index))));
  }

  @HaraProtocolExtension(protocol = IIter.class, method = "iter", receiver = java.util.Map.class)
  static Object iterMap(Object receiver, Object[] arguments) {
    return ((java.util.Map<?, ?>) receiver).entrySet().iterator();
  }

  @HaraProtocolExtension(
      protocol = IIter.class,
      method = "iter",
      receiver = java.util.Map.Entry.class)
  static Object iterEntry(Object receiver, Object[] arguments) {
    java.util.Map.Entry<?, ?> entry = (java.util.Map.Entry<?, ?>) receiver;
    return hara.lang.base.Iter.objects(entry.getKey(), entry.getValue());
  }

  @HaraProtocolExtension(protocol = IIter.class, method = "iter", target = HaraProtocolTarget.NIL)
  static Object iterNil(Object receiver, Object[] arguments) {
    return hara.lang.base.Iter.emptyIterator();
  }

  @HaraProtocolExtension(protocol = IIter.class, method = "iter", receiver = Object[].class)
  @HaraProtocolExtension(protocol = IIter.class, method = "iter", receiver = boolean[].class)
  @HaraProtocolExtension(protocol = IIter.class, method = "iter", receiver = byte[].class)
  @HaraProtocolExtension(protocol = IIter.class, method = "iter", receiver = char[].class)
  @HaraProtocolExtension(protocol = IIter.class, method = "iter", receiver = short[].class)
  @HaraProtocolExtension(protocol = IIter.class, method = "iter", receiver = int[].class)
  @HaraProtocolExtension(protocol = IIter.class, method = "iter", receiver = long[].class)
  @HaraProtocolExtension(protocol = IIter.class, method = "iter", receiver = float[].class)
  @HaraProtocolExtension(protocol = IIter.class, method = "iter", receiver = double[].class)
  static Object iterArray(Object receiver, Object[] arguments) {
    return hara.lang.base.Iter.iter(receiver);
  }

  @HaraProtocolExtension(protocol = IIterator.class, method = "iter-next?", receiver = Iterator.class)
  static Object iteratorHasNext(Object receiver, Object[] arguments) {
    return ((Iterator<?>) receiver).hasNext();
  }

  @HaraProtocolExtension(protocol = IIterator.class, method = "iter-next", receiver = Iterator.class)
  static Object iteratorNext(Object receiver, Object[] arguments) {
    Iterator<?> iterator = (Iterator<?>) receiver;
    if (!iterator.hasNext()) {
      throw new HaraException("iter-next reached the end of the iterator");
    }
    return iterator.next();
  }

  @HaraProtocolExtension(protocol = IClose.class, method = "close", receiver = Iterator.class)
  static Object closeIterator(Object receiver, Object[] arguments) {
    hara.lang.base.Iter.close((Iterator<?>) receiver);
    return null;
  }

  @HaraProtocolExtension(protocol = IClose.class, method = "close", receiver = AutoCloseable.class)
  static Object closeAutoCloseable(Object receiver, Object[] arguments) {
    try {
      ((AutoCloseable) receiver).close();
      return receiver;
    } catch (Exception error) {
      throw new HaraException("close failed: " + error.getMessage());
    }
  }

  @HaraProtocolExtension(protocol = ICas.class, method = "cas", receiver = ICas.class)
  static Object casProtocol(Object receiver, Object[] arguments) {
    Object oldValue = arguments[0];
    Object newValue = arguments[1];
    if (receiver instanceof hara.lang.data.Atom.Swap swap) {
      swap.validate(newValue);
      boolean changed = swap.cas(oldValue, newValue);
      if (changed) swap.notifyWatches(oldValue, newValue);
      return changed;
    }
    return ((ICas<Object>) receiver).cas(oldValue, newValue);
  }

  @HaraProtocolExtension(protocol = IReduce.class, method = "reduce", receiver = IReduce.class)
  static Object reduceProtocol(Object receiver, Object[] arguments) {
    Object result;
    if (arguments.length == 1) {
      result = ((IReduce) receiver).reduce(arguments[0]);
    } else if (arguments.length == 2) {
      result = ((IReduce) receiver).reduce(arguments[0], arguments[1]);
    } else {
      throw new HaraException("IReduce/reduce expects a function and optional initial value");
    }
    result = HaraBox.unwrap(result);
    return Reduced.isReduced(result) ? Reduced.unreduced(result) : result;
  }

  @HaraProtocolExtension(protocol = IReduce.class, method = "reduce", receiver = Iterable.class)
  @HaraProtocolExtension(protocol = IReduce.class, method = "reduce", receiver = Iterator.class)
  @HaraProtocolExtension(protocol = IReduce.class, method = "reduce", receiver = String.class)
  @HaraProtocolExtension(protocol = IReduce.class, method = "reduce", receiver = java.util.Map.class)
  @HaraProtocolExtension(
      protocol = IReduce.class,
      method = "reduce",
      receiver = java.util.Map.Entry.class)
  @HaraProtocolExtension(
      protocol = IReduce.class,
      method = "reduce",
      target = HaraProtocolTarget.NIL)
  @HaraProtocolExtension(protocol = IReduce.class, method = "reduce", receiver = Object[].class)
  @HaraProtocolExtension(protocol = IReduce.class, method = "reduce", receiver = boolean[].class)
  @HaraProtocolExtension(protocol = IReduce.class, method = "reduce", receiver = byte[].class)
  @HaraProtocolExtension(protocol = IReduce.class, method = "reduce", receiver = char[].class)
  @HaraProtocolExtension(protocol = IReduce.class, method = "reduce", receiver = short[].class)
  @HaraProtocolExtension(protocol = IReduce.class, method = "reduce", receiver = int[].class)
  @HaraProtocolExtension(protocol = IReduce.class, method = "reduce", receiver = long[].class)
  @HaraProtocolExtension(protocol = IReduce.class, method = "reduce", receiver = float[].class)
  @HaraProtocolExtension(protocol = IReduce.class, method = "reduce", receiver = double[].class)
  static Object reduceFallback(HaraContext context, Object receiver, Object[] arguments) {
    if (arguments.length < 1 || arguments.length > 2) {
      throw new HaraException("IReduce/reduce expects a function and optional initial value");
    }
    Iterator<?> iterator =
        receiver instanceof String
            ? hara.lang.base.Iter.codePoints((String) receiver)
            : hara.lang.base.Iter.iter(receiver);
    try {
      Object accumulator;
      if (arguments.length == 2) {
        accumulator = arguments[1];
      } else {
        if (!iterator.hasNext()) {
          throw new HaraException("IReduce/reduce cannot reduce an empty value without init");
        }
        accumulator = iterator.next();
      }
      while (iterator.hasNext()) {
        accumulator =
            HaraBox.unwrap(
                context.invokeCallable(
                    arguments[0], new Object[] {accumulator, iterator.next()}));
        if (Reduced.isReduced(accumulator)) return Reduced.unreduced(accumulator);
      }
      return accumulator;
    } finally {
      hara.lang.base.Iter.close(iterator);
    }
  }

  @HaraProtocolExtension(protocol = IWatch.class, method = "watch-add", receiver = IWatch.class)
  static Object watchAdd(Object receiver, Object[] arguments) {
    IWatch watch = (IWatch) receiver;
    Object callback = arguments[1];
    watch.addWatch(
        arguments[0],
        entry ->
            invokeCallback(
                callback,
                new Object[] {
                  arguments[0],
                  receiver,
                  ((IWatch.WatchEntry) entry).oldVal(),
                  ((IWatch.WatchEntry) entry).newVal()
                }));
    return receiver;
  }

  @HaraProtocolExtension(
      protocol = IWatch.class, method = "watch-remove", receiver = IWatch.class)
  static Object watchRemove(Object receiver, Object[] arguments) {
    ((IWatch) receiver).removeWatch(arguments[0]);
    return receiver;
  }

  @HaraProtocolExtension(
      protocol = IWatch.class, method = "watch-list", receiver = IWatch.class)
  static Object watchList(Object receiver, Object[] arguments) {
    return ((IWatch) receiver).getWatches();
  }

  private static Map<String, Integer> navigationMethods() {
    Map<String, Integer> methods = new LinkedHashMap<>();
    methods.put("peek-first", 1);
    methods.put("peek-last", 1);
    methods.put("pop-first", 1);
    methods.put("pop-last", 1);
    methods.put("push-first", 2);
    methods.put("push-last", 2);
    return methods;
  }

  static Object lookupValue(ILookup<?, ?> lookup, Object[] arguments) {
    if (arguments.length < 1 || arguments.length > 2) {
      throw new HaraException("ILookup/lookup expects one or two arguments");
    }
    try {
      if (lookup instanceof ISequentialLookupType<?> sequential) {
        long index = sequentialLookupIndex(arguments[0]);
        if (index < 0 || index >= sequential.count()) {
          return arguments.length == 2 ? arguments[1] : null;
        }
        return sequential.nth(index);
      }
      return lookupValueUnchecked(lookup, arguments);
    } catch (IndexOutOfBoundsException error) {
      // `get` is safe associative lookup, including for sequential values.
      // Positional `nth` remains the operation that reports an invalid index.
      return arguments.length == 2 ? arguments[1] : null;
    }
  }

  private static Object lookupBytes(Object receiver, Object[] arguments) {
    if (arguments.length < 1 || arguments.length > 2 || !HaraNumericConversions.isNumeric(arguments[0])) {
      throw new HaraException("ILookup/lookup on bytes expects an index and optional default");
    }
    long index = HaraNumericConversions.toLong(arguments[0], "ILookup/lookup on bytes");
    byte[] bytes = (byte[]) receiver;
    if (index < 0 || index >= bytes.length) {
      return arguments.length == 2 ? arguments[1] : null;
    }
    return bytes[(int) index];
  }

  private static Object lookupTuple(Object receiver, Object[] arguments) {
    if (arguments.length < 1 || arguments.length > 2) {
      throw new HaraException("ILookup/lookup expects one or two arguments");
    }
    ILinearType<?> tuple = (ILinearType<?>) receiver;
    long index = sequentialLookupIndex(arguments[0]);
    if (index < 0 || index >= tuple.count()) {
      return arguments.length == 2 ? arguments[1] : null;
    }
    return tuple.nth(index);
  }

  private static Object lookupMapEntry(MapEntry<?, ?> entry, Object[] arguments) {
    if (arguments.length < 1 || arguments.length > 2) {
      throw new HaraException("ILookup/lookup expects one or two arguments");
    }
    long index = sequentialLookupIndex(arguments[0]);
    if (index < 0 || index >= 2) {
      return arguments.length == 2 ? arguments[1] : null;
    }
    return entry.nth(index);
  }

  @SuppressWarnings("unchecked")
  private static Object lookupValueUnchecked(ILookup<?, ?> lookup, Object[] arguments) {
    ILookup<Object, Object> typed = (ILookup<Object, Object>) lookup;
    return arguments.length == 1
        ? typed.lookup(arguments[0])
        : typed.lookup(arguments[0], arguments[1]);
  }

  @SuppressWarnings("unchecked")
  private static Object assocValue(IAssoc<?, ?> assoc, Object[] arguments) {
    Object key = arguments[0];
    if (assoc instanceof IVectorType && !(key instanceof Integer)) {
      key = assocIndex(key);
    }
    try {
      return ((IAssoc<Object, Object>) assoc).assoc(key, arguments[1]);
    } catch (IndexOutOfBoundsException error) {
      throw new HaraException("assoc index out of bounds: " + key);
    }
  }

  private static Object assocTuple(Object receiver, Object[] arguments) {
    ILinearType<?> tuple = (ILinearType<?>) receiver;
    int index = assocIndex(arguments[0]);
    int count = Math.toIntExact(tuple.count());
    if (index < 0 || index > count) {
      throw new HaraException("assoc index out of bounds: " + index);
    }
    Object[] values = new Object[count + (index == count ? 1 : 0)];
    for (int item = 0; item < count; item++) values[item] = tuple.nth(item);
    values[index] = arguments[1];
    Object result =
        values.length <= 8
            ? hara.kernel.builtin.BuiltinStruct.tuple(values)
            : hara.lang.data.Vector.Standard.from(null, values);
    if (receiver instanceof IObjType source && result instanceof IObjType target) {
      result = target.withMeta(source.meta());
    }
    return result;
  }

  private static Object findTuple(Object receiver, Object[] arguments) {
    Object key = arguments[0];
    long index = sequentialLookupIndex(key);
    ILinearType<?> tuple = (ILinearType<?>) receiver;
    if (index < 0 || index >= tuple.count()) return null;
    return new MapEntry<>(null, index, tuple.nth(index));
  }

  private static Object nthTuple(Object receiver, Object[] arguments) {
    long index = HaraNumericConversions.toLong(arguments[0], "INth/nth");
    try {
      return ((ILinearType<?>) receiver).nth(index);
    } catch (IndexOutOfBoundsException | java.util.NoSuchElementException error) {
      throw new HaraException("nth index out of bounds: " + index);
    }
  }

  private static Integer assocIndex(Object key) {
    if (!HaraNumericConversions.isNumeric(key)) {
      throw new HaraException("assoc index must be a number");
    }
    return HaraNumericConversions.toInt(key, "assoc index");
  }

  @SuppressWarnings("unchecked")
  private static Object conjValue(IConj<?> conj, Object value) {
    if (conj instanceof ISetType<?> && value == null) {
      value = HaraNull.SINGLETON;
    }
    if (conj instanceof IMapType<?, ?> && !(value instanceof MapEntry<?, ?>)) {
      if (value instanceof ILinearType<?> linear && linear.count() == 2) {
        value = new MapEntry<>(null, linear.nth(0), linear.nth(1));
      } else {
        throw new HaraException("IConj/conj map expects a two-element entry");
      }
    }
    return ((IConj<Object>) conj).conj(value);
  }

  @SuppressWarnings("unchecked")
  private static Object findValue(IFind<?, ?> find, Object key) {
    if (find instanceof ISequentialLookupType<?> sequential) {
      long index = sequentialLookupIndex(key);
      return index < 0 || index >= sequential.count()
          ? null
          : new MapEntry<>(null, index, sequential.nth(index));
    }
    return ((IFind<Object, Object>) find).find(key);
  }

  private static long sequentialLookupIndex(Object value) {
    if (!HaraNumericConversions.isNumeric(value)) {
      throw new HaraException(
          "sequential lookup expects a non-negative integer index, received "
              + hara.lang.base.G.display(value));
    }
    long index = HaraNumericConversions.toLong(value, "sequential lookup");
    if (index < 0) {
      throw new HaraException(
          "sequential lookup expects a non-negative integer index, received "
              + hara.lang.base.G.display(value));
    }
    return index;
  }

  static Object setValue(ISetType<?> set, Object[] arguments) {
    if (arguments.length < 1 || arguments.length > 2) {
      throw new HaraException("IFn set lookup expects one or two arguments");
    }
    Object found = findValue(set, arguments[0]);
    return found == null && arguments.length == 2 ? arguments[1] : found;
  }

  private static Object invokeCallback(Object callback, Object[] arguments) {
    if (callback instanceof HaraFunction) {
      HaraFunction function = (HaraFunction) callback;
      return function.callTarget().call(function.callArguments(arguments));
    }
    if (callback instanceof IFn) {
      return applyFunction((IFn<?, ?, ?>) callback, arguments);
    }
    if (callback instanceof Consumer<?>) {
      @SuppressWarnings("unchecked")
      Consumer<Object> consumer = (Consumer<Object>) callback;
      consumer.accept(arguments[0]);
      return null;
    }
    throw new HaraException("watch callback must be a Hara function or IFn");
  }

  @SuppressWarnings("unchecked")
  private static Object indexOfValue(IIndexed<?, ?> indexed, Object value) {
    return ((IIndexed<Object, Object>) indexed).indexOf(value);
  }

  @SuppressWarnings("unchecked")
  private static long indexOfKeyValue(IIndexedKV<?, ?> indexed, Object value) {
    return ((IIndexedKV<Object, Object>) indexed).indexOfKey(value);
  }

  @SuppressWarnings("unchecked")
  private static long indexOfValValue(IIndexedKV<?, ?> indexed, Object value) {
    return ((IIndexedKV<Object, Object>) indexed).indexOfVal(value);
  }

  @SuppressWarnings("unchecked")
  private static Object consValue(ICons<?> cons, Object value) {
    return ((ICons<Object>) cons).cons(value);
  }

  @SuppressWarnings("unchecked")
  private static Object consSequential(ISequential<?> sequential, Object value) {
    hara.lang.data.Seq<Object> tail =
        hara.lang.data.Seq.create(((ISequential<Object>) sequential).iterator());
    return new hara.lang.data.Cons<>(null, value, tail);
  }

  @SuppressWarnings("unchecked")
  private static Object dissocValue(IDissoc<?> dissoc, Object key) {
    return ((IDissoc<Object>) dissoc).dissoc(key);
  }

  @SuppressWarnings("unchecked")
  private static Object pushFirstValue(IPushFirst<?> pushFirst, Object value) {
    return ((IPushFirst<Object>) pushFirst).pushFirst(value);
  }

  @SuppressWarnings("unchecked")
  private static Object pushLastValue(IPushLast<?> pushLast, Object value) {
    return ((IPushLast<Object>) pushLast).pushLast(value);
  }

  @SuppressWarnings("unchecked")
  private static Object resetValue(IReset<?> reset, Object value) {
    return ((IReset<Object>) reset).reset(value);
  }

  @SuppressWarnings("unchecked")
  private static Object derefTimeoutValue(
      IDerefTimeout<?> deref, Object milliseconds, Object timeoutValue) {
    long timeout = HaraNumericConversions.toLong(milliseconds, "IDerefTimeout/deref-timeout");
    if (timeout < 0) {
      throw new HaraException("IDerefTimeout/deref-timeout expects a non-negative timeout");
    }
    return ((IDerefTimeout<Object>) deref).derefTimeout(timeout, timeoutValue);
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  static Object applyFunction(IFn<?, ?, ?> function, Object[] arguments) {
    return IFn.applyAsArray((IFn) function, arguments);
  }
}
