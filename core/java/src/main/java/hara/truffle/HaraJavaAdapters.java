package hara.truffle;

import hara.lang.base.Reduced;
import hara.lang.data.types.ISequentialLookupType;
import hara.lang.data.types.ISequentialType;
import hara.lang.data.types.ILinearType;
import hara.lang.data.types.ISetType;
import hara.lang.data.types.IMapType;
import hara.lang.data.types.IVectorType;
import hara.lang.data.Keyword;
import hara.lang.data.List;
import hara.lang.data.Symbol;
import hara.lang.data.TaggedLiteral;
import hara.lang.data.Tuple;
import hara.lang.protocol.*;
import java.util.Arrays;
import java.util.Collections;
import java.util.Iterator;
import java.util.LinkedHashMap;
import java.util.Map;
import java.nio.charset.StandardCharsets;
import java.util.function.Consumer;

/** Compatibility adapters from existing Java protocol interfaces to Hara protocol dispatch. */
public final class HaraJavaAdapters {
  private HaraJavaAdapters() {}

  private interface ProtocolInstaller {
    void register(HaraContext context, HaraProtocol protocol);
  }

  private static final Map<String, ProtocolInstaller> SEMANTIC_ADAPTERS = semanticAdapters();

  static void install(HaraContext context, HaraProtocolDeclarations.Registry registry) {
    HaraProtocolInterfaceAdapters.install(registry);
    for (String name : SEMANTIC_ADAPTERS.keySet()) {
      if (!registry.protocols().containsKey(name)) {
        throw new HaraException("Missing injected protocol for semantic adapter: " + name);
      }
    }
    for (Map.Entry<String, HaraProtocol> protocol : registry.protocols().entrySet()) {
      ProtocolInstaller installer = SEMANTIC_ADAPTERS.get(protocol.getKey());
      if (installer != null) installer.register(context, protocol.getValue());
    }
  }

  private static Map<String, ProtocolInstaller> semanticAdapters() {
    Map<String, ProtocolInstaller> adapters = new LinkedHashMap<>();
    adapters.put("IFn", HaraJavaAdapters::registerIFn);
    adapters.put("IStringLike", (context, protocol) -> registerStringLike(protocol));
    adapters.put("ILookup", (context, protocol) -> registerLookup(protocol));
    adapters.put("IAssoc", (context, protocol) -> registerAssoc(protocol));
    adapters.put("ICount", (context, protocol) -> registerCount(protocol));
    adapters.put("IConj", (context, protocol) -> registerConj(protocol));
    adapters.put("IFind", (context, protocol) -> registerFind(protocol));
    adapters.put("IEquality", (context, protocol) -> registerEquality(protocol));
    adapters.put("IHash", (context, protocol) -> registerHash(protocol));
    adapters.put("IDerefTimeout", (context, protocol) -> registerDerefTimeout(protocol));
    adapters.put("INth", (context, protocol) -> registerNth(protocol));
    adapters.put("IEmpty", (context, protocol) -> registerEmpty(protocol));
    adapters.put("IEncodable", HaraJavaAdapters::registerEncodable);
    adapters.put("ICons", (context, protocol) -> registerCons(protocol));
    adapters.put("IIter", (context, protocol) -> registerIter(protocol));
    adapters.put("IIterator", (context, protocol) -> registerIterator(protocol));
    adapters.put("IClose", (context, protocol) -> registerClose(protocol));
    adapters.put("ICas", (context, protocol) -> registerCas(protocol));
    adapters.put("IReduce", HaraJavaAdapters::registerReduce);
    adapters.put("IWatch", (context, protocol) -> registerWatch(protocol));
    return Collections.unmodifiableMap(adapters);
  }

  public static void registerIFn(HaraProtocol protocol) {
    protocol.extend(IFn.class, "invoke", HaraJavaAdapters::invokeFunction);
  }

  public static void registerIFn(HaraContext context, HaraProtocol protocol) {
    registerIFn(protocol);
    HaraProtocolInvoker callable =
        (receiver, arguments) ->
            context.invokeCallable(
                receiver,
                Arrays.stream(arguments)
                    .map(HaraJavaAdapters::unwrapArgument)
                    .toArray(Object[]::new));
    protocol.extend(HaraFunction.class, "invoke", callable);
    protocol.extend(HaraMultiFunction.class, "invoke", callable);
    protocol.extend(HaraType.class, "invoke", callable);
    protocol.extend(hara.lang.data.Pointer.class, "invoke", callable);
    protocol.extend(HbcMachine.HbcClosure.class, "invoke", callable);
    protocol.extend(HbcMachine.HbcMultiArity.class, "invoke", callable);
    protocol.extend(HbcMachine.HbcNativeCallable.class, "invoke", callable);
  }

  public static void registerStringLike(HaraProtocol protocol) {
    protocol.extend(String.class, "to-string", (receiver, arguments) -> receiver);
    protocol.extend(
        String.class, "from-string", (receiver, arguments) -> String.valueOf(arguments[0]));
    protocol.extend(
        Keyword.class,
        "to-string",
        (receiver, arguments) -> {
          Keyword keyword = (Keyword) receiver;
          return keyword.getNamespace() == null
              ? keyword.getName()
              : keyword.getNamespace() + "/" + keyword.getName();
        });
    protocol.extend(
        Keyword.class,
        "from-string",
        (receiver, arguments) -> Keyword.create(String.valueOf(arguments[0])));
    protocol.extend(
        Symbol.class, "to-string", (receiver, arguments) -> ((Symbol) receiver).pathString());
    protocol.extend(
        Symbol.class,
        "from-string",
        (receiver, arguments) -> Symbol.create(String.valueOf(arguments[0])));
    protocol.extend(
        byte[].class,
        "to-string",
        (receiver, arguments) -> new String((byte[]) receiver, StandardCharsets.UTF_8));
    protocol.extend(
        byte[].class,
        "from-string",
        (receiver, arguments) -> String.valueOf(arguments[0]).getBytes(StandardCharsets.UTF_8));
  }

  public static void installApplicable(HaraProtocol protocol) {
    protocol.extend(
        IApplicable.class,
        "apply-in",
        (receiver, arguments) ->
            ((IApplicable) receiver).applyIn(arguments[0], (Object[]) arguments[1]));
    protocol.extend(
        IApplicable.class,
        "apply-default",
        (receiver, arguments) -> ((IApplicable) receiver).applyDefault());
    protocol.extend(
        IApplicable.class,
        "transform-in",
        (receiver, arguments) ->
            ((IApplicable) receiver).transformIn(arguments[0], (Object[]) arguments[1]));
    protocol.extend(
        IApplicable.class,
        "transform-out",
        (receiver, arguments) ->
            ((IApplicable) receiver)
                .transformOut(arguments[0], (Object[]) arguments[1], arguments[2]));
  }

  public static void installPointer(HaraProtocol protocol) {
    protocol.extend(
        IPointer.class, "ptr-context", (receiver, arguments) -> ((IPointer) receiver).ptrContext());
  }

  public static void installSpace(HaraProtocol protocol) {
    protocol.extend(
        ISpace.class,
        "context-set",
        (receiver, arguments) -> {
          ((ISpace) receiver).contextSet(arguments[0], arguments[1], arguments[2]);
          return receiver;
        });
    protocol.extend(
        ISpace.class,
        "context-unset",
        (receiver, arguments) -> {
          ((ISpace) receiver).contextUnset(arguments[0]);
          return receiver;
        });
    protocol.extend(
        ISpace.class, "context-list", (receiver, arguments) -> ((ISpace) receiver).contextList());
    protocol.extend(
        ISpace.class,
        "context-get",
        (receiver, arguments) -> ((ISpace) receiver).contextGet(arguments[0]));
    protocol.extend(
        ISpace.class, "rt-active", (receiver, arguments) -> ((ISpace) receiver).activeRuntimes());
    protocol.extend(
        ISpace.class,
        "rt-get",
        (receiver, arguments) -> ((ISpace) receiver).runtimeGet(arguments[0]));
    protocol.extend(
        ISpace.class,
        "rt-start",
        (receiver, arguments) -> ((ISpace) receiver).runtimeStart(arguments[0]));
    protocol.extend(
        ISpace.class,
        "rt-started?",
        (receiver, arguments) -> ((ISpace) receiver).runtimeStarted(arguments[0]));
    protocol.extend(
        ISpace.class,
        "rt-stopped?",
        (receiver, arguments) -> ((ISpace) receiver).runtimeStopped(arguments[0]));
    protocol.extend(
        ISpace.class,
        "rt-stop",
        (receiver, arguments) -> {
          ((ISpace) receiver).runtimeStop(arguments[0]);
          return receiver;
        });
  }

  /** Invokes an existing Java IFn using the same collection lookup semantics as protocol calls. */
  public static Object invokeFunction(Object receiver, Object[] arguments) {
    IFn<?, ?, ?> function = (IFn<?, ?, ?>) receiver;
    Object[] values = Arrays.stream(arguments).map(HaraJavaAdapters::unwrapArgument).toArray(Object[]::new);
    if (function instanceof ILookup) {
      return lookupValue((ILookup<?, ?>) function, values);
    }
    if (function instanceof ISequentialLookupType && values.length == 1) {
      return ((ISequentialLookupType<?>) function)
          .nth(HaraNumericConversions.toLong(values[0], "IFn sequential lookup"));
    }
    if (function instanceof ISetType) {
      return setValue((ISetType<?>) function, values);
    }
    return applyFunction(function, values);
  }

  private static Object unwrapArgument(Object value) {
    Object unwrapped = HaraBox.unwrap(value);
    return unwrapped == HaraNull.SINGLETON ? null : unwrapped;
  }

  public static void registerLookup(HaraProtocol protocol) {
    protocol.extendIntrinsic(
        ILookup.class,
        "lookup",
        (receiver, arguments) -> {
          if (arguments.length < 1 || arguments.length > 2) {
            throw new HaraException("ILookup/lookup expects one or two arguments");
          }
          return lookupValue((ILookup<?, ?>) receiver, arguments);
        });
    protocol.extendIntrinsic(Tuple.Tup0.class, "lookup", HaraJavaAdapters::lookupTuple);
    protocol.extendIntrinsic(Tuple.Tup1.class, "lookup", HaraJavaAdapters::lookupTuple);
    protocol.extendIntrinsic(byte[].class, "lookup", HaraJavaAdapters::lookupBytes);
    protocol.extendIntrinsic(
        ISetType.class,
        "lookup",
        (receiver, arguments) -> {
          if (arguments.length < 1 || arguments.length > 2) {
            throw new HaraException("ILookup/lookup expects one or two arguments");
          }
          return setValue((ISetType<?>) receiver, arguments);
        });
    protocol.extendNilIntrinsic(
        "lookup",
        (receiver, arguments) -> {
          if (arguments.length < 1 || arguments.length > 2) {
            throw new HaraException("ILookup/lookup expects one or two arguments");
          }
          return arguments.length == 2 ? arguments[1] : null;
        });
  }

  public static void registerAssoc(HaraProtocol protocol) {
    protocol.extendIntrinsic(
        IAssoc.class,
        "assoc",
        (receiver, arguments) -> {
          return assocValue((IAssoc<?, ?>) receiver, arguments);
        });
    protocol.extendIntrinsic(Tuple.Tup0.class, "assoc", HaraJavaAdapters::assocTuple);
    protocol.extendIntrinsic(Tuple.Tup1.class, "assoc", HaraJavaAdapters::assocTuple);
  }

  public static void registerCount(HaraProtocol protocol) {
    protocol.extend(ICount.class, "count", (receiver, arguments) -> ((ICount) receiver).count());
    protocol.extend(
        String.class,
        "count",
        (receiver, arguments) -> {
          String value = (String) receiver;
          return (long) value.codePointCount(0, value.length());
        });
    protocol.extend(byte[].class, "count", (receiver, arguments) -> ((byte[]) receiver).length);
    protocol.extendNil("count", (receiver, arguments) -> 0L);
  }

  public static void registerConj(HaraProtocol protocol) {
    protocol.extend(
        IConj.class, "conj", (receiver, arguments) -> conjValue((IConj<?>) receiver, arguments[0]));
    protocol.extendNil("conj", (receiver, arguments) -> List.Standard.from(null, arguments[0]));
  }

  public static void registerFind(HaraProtocol protocol) {
    protocol.extend(
        IFind.class,
        "find",
        (receiver, arguments) -> findValue((IFind<?, ?>) receiver, arguments[0]));
    protocol.extendIntrinsic(Tuple.Tup0.class, "find", HaraJavaAdapters::findTuple);
    protocol.extendIntrinsic(Tuple.Tup1.class, "find", HaraJavaAdapters::findTuple);
  }

  public static void registerEquality(HaraProtocol protocol) {
    protocol.extend(
        IEquality.class,
        "equality",
        (receiver, arguments) -> ((IEquality) receiver).equality(arguments[0]));
    protocol.extend(
        byte[].class,
        "equality",
        (receiver, arguments) ->
            arguments.length == 1
                && arguments[0] instanceof byte[]
                && Arrays.equals((byte[]) receiver, (byte[]) arguments[0]));
  }

  public static void registerHash(HaraProtocol protocol) {
    protocol.extend(IHash.class, "hash", (receiver, arguments) -> ((IHash) receiver).hashGet());
    protocol.extend(
        byte[].class, "hash", (receiver, arguments) -> (long) Arrays.hashCode((byte[]) receiver));
  }

  public static void installMetadata(HaraProtocol protocol) {
    protocol.extend(IObjType.class, "meta", (receiver, arguments) -> ((IObjType) receiver).meta());
    protocol.extend(
        IObjType.class,
        "with-meta",
        (receiver, arguments) ->
            ((IObjType) receiver).withMeta((hara.lang.protocol.IMetadata) arguments[0]));
  }

  public static void installDeref(HaraProtocol protocol) {
    protocol.extend(IDeref.class, "deref", (receiver, arguments) -> ((IDeref<?>) receiver).deref());
  }

  public static void registerDerefTimeout(HaraProtocol protocol) {
    protocol.extend(
        IDerefTimeout.class,
        "deref-timeout",
        (receiver, arguments) ->
            derefTimeoutValue((IDerefTimeout<?>) receiver, arguments[0], arguments[1]));
  }

  public static void registerNth(HaraProtocol protocol) {
    protocol.extendIntrinsic(
        INth.class,
        "nth",
        (receiver, arguments) -> {
          long index = HaraNumericConversions.toLong(arguments[0], "INth/nth");
          try {
            return ((INth<?>) receiver).nth(index);
          } catch (IndexOutOfBoundsException | java.util.NoSuchElementException error) {
            throw new HaraException("nth index out of bounds: " + index);
          }
        });
    protocol.extendIntrinsic(
        byte[].class,
        "nth",
        (receiver, arguments) -> {
          long index = HaraNumericConversions.toLong(arguments[0], "INth/nth");
          byte[] bytes = (byte[]) receiver;
          if (index < 0 || index >= bytes.length) {
            throw new HaraException("byte index out of bounds: " + index);
          }
          return bytes[(int) index];
        });
    protocol.extend(
        Iterable.class,
        "nth",
        (receiver, arguments) -> {
          long index = HaraNumericConversions.toLong(arguments[0], "INth/nth");
          try {
            return hara.lang.base.Iter.nth(((Iterable<?>) receiver).iterator(), index);
          } catch (IndexOutOfBoundsException | java.util.NoSuchElementException error) {
            throw new HaraException("nth index out of bounds: " + index);
          }
        });
    // Compact vectors implement INth directly, but use a more specific non-intrinsic adapter so
    // the specialized collection node preserves the shared bounds diagnostic.
    protocol.extend(Tuple.Tup0.class, "nth", HaraJavaAdapters::nthTuple);
    protocol.extend(Tuple.Tup1.class, "nth", HaraJavaAdapters::nthTuple);
  }

  public static void registerEmpty(HaraProtocol protocol) {
    protocol.extend(IEmpty.class, "empty", (receiver, arguments) -> ((IEmpty) receiver).empty());
    protocol.extendNil("empty", (receiver, arguments) -> null);
  }

  public static void installDisplay(HaraProtocol protocol) {
    protocol.extend(
        IDisplay.class, "display", (receiver, arguments) -> ((IDisplay) receiver).display());
  }

  public static void registerEncodable(HaraContext context, HaraProtocol protocol) {
    protocol.extendNil(
        "encode-with",
        (receiver, arguments) ->
            context.invokeProtocol("IEncodeVisitor", "visit-nil", arguments[0]));
    protocol.extendDefault(
        "encode-with",
        (receiver, arguments) -> {
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
                      : receiver instanceof Character
                          ? "visit-character"
                          : receiver instanceof String
                              ? "visit-string"
                              : receiver instanceof Keyword
                                  ? "visit-keyword"
                                  : receiver instanceof Symbol
                                      ? "visit-symbol"
                                      : receiver instanceof IVectorType<?>
                                              || receiver instanceof Tuple.Tup0
                                              || receiver instanceof Tuple.Tup1<?>
                                          ? "visit-vector"
                                          : receiver instanceof IMapType<?, ?>
                                              ? "visit-map"
                                              : receiver instanceof ISetType<?>
                                                  ? "visit-set"
                                                  : receiver instanceof ISequentialType<?>
                                                      ? "visit-seq"
                                                      : "visit-unknown";
          return context.invokeProtocol("IEncodeVisitor", method, visitor, receiver);
        });
  }

  public static void installCollection(HaraProtocol protocol) {
    protocol.extend(
        IColl.class, "start-string", (receiver, arguments) -> ((IColl<?>) receiver).startString());
    protocol.extend(
        IColl.class, "end-string", (receiver, arguments) -> ((IColl<?>) receiver).endString());
    protocol.extend(
        IColl.class, "sep-string", (receiver, arguments) -> ((IColl<?>) receiver).sepString());
    protocol.extend(
        IColl.class, "iterator", (receiver, arguments) -> ((IColl<?>) receiver).iterator());
  }

  public static void registerCons(HaraProtocol protocol) {
    protocol.extend(
        hara.lang.data.Seq.class,
        "cons",
        (receiver, arguments) -> consValue((ICons<?>) receiver, arguments[0]));
    protocol.extend(
        hara.lang.data.Cons.class,
        "cons",
        (receiver, arguments) -> consValue((ICons<?>) receiver, arguments[0]));
    protocol.extend(
        hara.lang.data.Deque.class,
        "cons",
        (receiver, arguments) -> consValue((ICons<?>) receiver, arguments[0]));
    protocol.extend(
        hara.lang.data.Queue.class,
        "cons",
        (receiver, arguments) -> consSequential((ISequentialType<?>) receiver, arguments[0]));
    protocol.extend(
        List.class,
        "cons",
        (receiver, arguments) -> consSequential((ISequentialType<?>) receiver, arguments[0]));
    protocol.extend(
        hara.lang.data.Vector.class,
        "cons",
        (receiver, arguments) -> consSequential((ISequentialType<?>) receiver, arguments[0]));
    protocol.extend(
        Tuple.Tup0.class,
        "cons",
        (receiver, arguments) -> consSequential((ISequentialType<?>) receiver, arguments[0]));
    protocol.extend(
        Tuple.Tup1.class,
        "cons",
        (receiver, arguments) -> consSequential((ISequentialType<?>) receiver, arguments[0]));
    protocol.extend(
        ISequentialType.class,
        "cons",
        (receiver, arguments) -> consSequential((ISequentialType<?>) receiver, arguments[0]));
    protocol.extend(
        ICons.class, "cons", (receiver, arguments) -> consValue((ICons<?>) receiver, arguments[0]));
    protocol.extendNil(
        "cons", (receiver, arguments) -> new hara.lang.data.Cons<>(null, arguments[0], null));
  }

  public static void installDissoc(HaraProtocol protocol) {
    protocol.extend(
        IDissoc.class,
        "dissoc",
        (receiver, arguments) -> dissocValue((IDissoc<?>) receiver, arguments[0]));
  }

  public static void installIndexed(HaraProtocol protocol) {
    protocol.extend(
        IIndexed.class,
        "index-of",
        (receiver, arguments) -> indexOfValue((IIndexed<?, ?>) receiver, arguments[0]));
  }

  public static void installIndexedKV(HaraProtocol protocol) {
    protocol.extend(
        IIndexedKV.class,
        "index-of-key",
        (receiver, arguments) -> indexOfKeyValue((IIndexedKV<?, ?>) receiver, arguments[0]));
    protocol.extend(
        IIndexedKV.class,
        "index-of-val",
        (receiver, arguments) -> indexOfValValue((IIndexedKV<?, ?>) receiver, arguments[0]));
  }

  public static void installPeekFirst(HaraProtocol protocol) {
    protocol.extend(
        IPeekFirst.class,
        "peek-first",
        (receiver, arguments) -> ((IPeekFirst<?>) receiver).peekFirst());
  }

  public static void installPeekLast(HaraProtocol protocol) {
    protocol.extend(
        IPeekLast.class,
        "peek-last",
        (receiver, arguments) -> ((IPeekLast<?>) receiver).peekLast());
  }

  public static void installPopFirst(HaraProtocol protocol) {
    protocol.extend(
        IPopFirst.class, "pop-first", (receiver, arguments) -> ((IPopFirst) receiver).popFirst());
  }

  public static void installPopLast(HaraProtocol protocol) {
    protocol.extend(
        IPopLast.class, "pop-last", (receiver, arguments) -> ((IPopLast) receiver).popLast());
  }

  public static void installPushFirst(HaraProtocol protocol) {
    protocol.extend(
        IPushFirst.class,
        "push-first",
        (receiver, arguments) -> pushFirstValue((IPushFirst<?>) receiver, arguments[0]));
  }

  public static void installPushLast(HaraProtocol protocol) {
    protocol.extend(
        IPushLast.class,
        "push-last",
        (receiver, arguments) -> pushLastValue((IPushLast<?>) receiver, arguments[0]));
  }

  public static void installContextLifeCycle(HaraProtocol protocol) {
    protocol.extend(
        IContextLifeCycle.class,
        "has-module?",
        (receiver, arguments) -> ((IContextLifeCycle) receiver).hasModule(arguments[0]));
    protocol.extend(
        IContextLifeCycle.class,
        "setup-module",
        (receiver, arguments) -> {
          ((IContextLifeCycle) receiver).setupModule(arguments[0]);
          return receiver;
        });
    protocol.extend(
        IContextLifeCycle.class,
        "teardown-module",
        (receiver, arguments) -> {
          ((IContextLifeCycle) receiver).teardownModule(arguments[0]);
          return receiver;
        });
    protocol.extend(
        IContextLifeCycle.class,
        "has-pointer?",
        (receiver, arguments) -> ((IContextLifeCycle) receiver).hasPointer((IPointer) arguments[0]));
    protocol.extend(
        IContextLifeCycle.class,
        "setup-pointer",
        (receiver, arguments) -> {
          ((IContextLifeCycle) receiver).setupPointer((IPointer) arguments[0]);
          return receiver;
        });
    protocol.extend(
        IContextLifeCycle.class,
        "teardown-pointer",
        (receiver, arguments) -> {
          ((IContextLifeCycle) receiver).teardownPointer((IPointer) arguments[0]);
          return receiver;
        });
  }

  public static void installHashCached(HaraProtocol protocol) {
    protocol.extend(
        IHashCached.class,
        "hash-current",
        (receiver, arguments) -> ((IHashCached) receiver).hashCurrent());
    protocol.extend(
        IHashCached.class,
        "hash-put",
        (receiver, arguments) -> {
          ((IHashCached) receiver)
              .hashPut(HaraNumericConversions.toLong(arguments[0], "IHashCached/hash-put"));
          return receiver;
        });
  }

  public static void registerIter(HaraProtocol protocol) {
    protocol.extend(
        IIter.class, "iter", (receiver, arguments) -> ((IIter<?>) receiver).iter());
    protocol.extend(
        Iterable.class, "iter", (receiver, arguments) -> ((Iterable<?>) receiver).iterator());
    protocol.extend(
        Iterator.class, "iter", (receiver, arguments) -> receiver);
    protocol.extend(String.class, "iter", (receiver, arguments) ->
        hara.lang.base.Iter.chars(((String) receiver).toCharArray()));
    protocol.extend(java.util.Map.class, "iter", (receiver, arguments) ->
        ((java.util.Map<?, ?>) receiver).entrySet().iterator());
    protocol.extend(java.util.Map.Entry.class, "iter", (receiver, arguments) -> {
      java.util.Map.Entry<?, ?> entry = (java.util.Map.Entry<?, ?>) receiver;
      return hara.lang.base.Iter.objects(entry.getKey(), entry.getValue());
    });
    protocol.extendNil("iter", (receiver, arguments) -> hara.lang.base.Iter.emptyIterator());
    installArrayIter(protocol, Object[].class);
    installArrayIter(protocol, boolean[].class);
    installArrayIter(protocol, byte[].class);
    installArrayIter(protocol, char[].class);
    installArrayIter(protocol, short[].class);
    installArrayIter(protocol, int[].class);
    installArrayIter(protocol, long[].class);
    installArrayIter(protocol, float[].class);
    installArrayIter(protocol, double[].class);
  }

  private static void installArrayIter(HaraProtocol protocol, Class<?> type) {
    protocol.extend(type, "iter", (receiver, arguments) -> hara.lang.base.Iter.iter(receiver));
  }

  public static void registerIterator(HaraProtocol protocol) {
    protocol.extend(
        Iterator.class,
        "iter-next?",
        (receiver, arguments) -> ((Iterator<?>) receiver).hasNext());
    protocol.extend(
        Iterator.class,
        "iter-next",
        (receiver, arguments) -> {
          Iterator<?> iterator = (Iterator<?>) receiver;
          if (!iterator.hasNext()) {
            throw new HaraException("iter-next reached the end of the iterator");
          }
          return iterator.next();
        });
  }

  public static void registerClose(HaraProtocol protocol) {
    protocol.extend(
        Iterator.class,
        "close",
        (receiver, arguments) -> {
          hara.lang.base.Iter.close((Iterator<?>) receiver);
          return null;
        });
    protocol.extend(
        AutoCloseable.class,
        "close",
        (receiver, arguments) -> {
          try {
            ((AutoCloseable) receiver).close();
            return receiver;
          } catch (Exception error) {
            throw new HaraException("close failed: " + error.getMessage());
          }
        });
  }

  public static void registerCas(HaraProtocol protocol) {
    protocol.extend(
        ICas.class,
        "cas",
        (receiver, arguments) -> {
          Object oldValue = arguments[0];
          Object newValue = arguments[1];
          if (receiver instanceof hara.lang.data.Atom.Swap swap) {
            swap.validate(newValue);
            boolean changed = swap.cas(oldValue, newValue);
            if (changed) swap.notifyWatches(oldValue, newValue);
            return changed;
          }
          return ((ICas<Object>) receiver).cas(oldValue, newValue);
        });
  }

  public static void registerReduce(HaraContext context, HaraProtocol protocol) {
    protocol.extend(
        IReduce.class,
        "reduce",
        (receiver, arguments) -> {
          Object result;
          if (arguments.length == 1) {
            result = ((IReduce) receiver).reduce(arguments[0]);
          } else if (arguments.length == 2) {
            result = ((IReduce) receiver).reduce(arguments[0], arguments[1]);
          } else {
            throw new HaraException(
                "IReduce/reduce expects a function and optional initial value");
          }
          result = HaraBox.unwrap(result);
          return Reduced.isReduced(result) ? Reduced.unreduced(result) : result;
        });
    HaraProtocolInvoker fallback =
        (receiver, arguments) -> {
          if (arguments.length < 1 || arguments.length > 2) {
            throw new HaraException("IReduce/reduce expects a function and optional initial value");
          }
          Iterator<?> iterator = hara.lang.base.Iter.iter(receiver);
          try {
            Object accumulator;
            if (arguments.length == 2) {
              accumulator = arguments[1];
            } else {
              if (!iterator.hasNext()) {
                throw new HaraException(
                    "IReduce/reduce cannot reduce an empty value without init");
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
        };
    protocol.extend(Iterable.class, "reduce", fallback);
    protocol.extend(Iterator.class, "reduce", fallback);
    protocol.extend(String.class, "reduce", (receiver, arguments) ->
        fallback.invoke(hara.lang.base.Iter.chars(((String) receiver).toCharArray()), arguments));
    protocol.extend(java.util.Map.class, "reduce", fallback);
    protocol.extend(java.util.Map.Entry.class, "reduce", fallback);
    protocol.extendNil("reduce", fallback);
    installArrayReduce(protocol, fallback, Object[].class);
    installArrayReduce(protocol, fallback, boolean[].class);
    installArrayReduce(protocol, fallback, byte[].class);
    installArrayReduce(protocol, fallback, char[].class);
    installArrayReduce(protocol, fallback, short[].class);
    installArrayReduce(protocol, fallback, int[].class);
    installArrayReduce(protocol, fallback, long[].class);
    installArrayReduce(protocol, fallback, float[].class);
    installArrayReduce(protocol, fallback, double[].class);
  }

  private static void installArrayReduce(
      HaraProtocol protocol, HaraProtocolInvoker fallback, Class<?> type) {
    protocol.extend(type, "reduce", fallback);
  }

  public static void installPromise(HaraProtocol protocol) {
    protocol.extend(IPromise.class, "state", (receiver, arguments) -> ((IPromise) receiver).state());
    protocol.extend(IPromise.class, "value", (receiver, arguments) -> ((IPromise) receiver).value());
    protocol.extend(
        IPromise.class, "then", (receiver, arguments) -> ((IPromise) receiver).then(arguments[0]));
    protocol.extend(
        IPromise.class,
        "catch",
        (receiver, arguments) -> ((IPromise) receiver).catchError(arguments[0]));
    protocol.extend(
        IPromise.class,
        "finally",
        (receiver, arguments) -> ((IPromise) receiver).finallyDo(arguments[0]));
    protocol.extend(
        IPromise.class, "cancel", (receiver, arguments) -> ((IPromise) receiver).cancel());
  }

  public static void installCoroutine(HaraProtocol protocol) {
    protocol.extend(
        ICoroutine.class, "status", (receiver, arguments) -> ((ICoroutine) receiver).status());
    protocol.extend(
        ICoroutine.class,
        "resume",
        (receiver, arguments) -> ((ICoroutine) receiver).resume(arguments));
  }

  public static void installStream(HaraProtocol protocol) {
    protocol.extend(IStream.class, "next", (receiver, arguments) -> ((IStream) receiver).next());
  }

  public static void installRealize(HaraProtocol protocol) {
    protocol.extend(
        IRealize.class,
        "realized?",
        (receiver, arguments) -> ((IRealize<?>) receiver).isRealized());
    protocol.extend(
        IRealize.class, "realize", (receiver, arguments) -> ((IRealize<?>) receiver).realize());
  }

  public static void installReset(HaraProtocol protocol) {
    protocol.extend(
        IReset.class,
        "reset",
        (receiver, arguments) -> resetValue((IReset<?>) receiver, arguments[0]));
  }

  public static void installConversion(HaraProtocol mutable, HaraProtocol persistent) {
    mutable.extend(
        IToMutable.class,
        "to-mutable",
        (receiver, arguments) -> ((IToMutable) receiver).toMutable());
    persistent.extend(
        IToPersistent.class,
        "to-persistent",
        (receiver, arguments) -> ((IToPersistent) receiver).toPersistent());
  }

  public static void registerWatch(HaraProtocol protocol) {
    protocol.extend(
        IWatch.class,
        "watch-add",
        (receiver, arguments) -> {
          IWatch watch = (IWatch) receiver;
          Object callback = arguments[1];
          watch.addWatch(
              arguments[0],
              entry ->
                  invokeCallback(
                      callback,
                      new Object[] {arguments[0], receiver, ((IWatch.WatchEntry) entry).oldVal(),
                          ((IWatch.WatchEntry) entry).newVal()}));
          return receiver;
        });
    protocol.extend(
        IWatch.class,
        "watch-remove",
        (receiver, arguments) -> {
          ((IWatch) receiver).removeWatch(arguments[0]);
          return receiver;
        });
    protocol.extend(
        IWatch.class, "watch-list", (receiver, arguments) -> ((IWatch) receiver).getWatches());
  }

  public static void installNamespaced(HaraProtocol protocol) {
    protocol.extend(
        INamespaced.class, "name", (receiver, arguments) -> ((INamespaced) receiver).getName());
    protocol.extend(
        INamespaced.class,
        "namespace",
        (receiver, arguments) -> ((INamespaced) receiver).getNamespace());
  }

  public static void installContext(HaraProtocol protocol) {
    protocol.extend(
        IContext.class, "call", (receiver, arguments) -> ((IContext) receiver).call(arguments));
  }

  public static void installInvokeIn(HaraProtocol protocol) {
    protocol.extend(
        IInvokeIn.class,
        "invoke-in",
        (receiver, arguments) -> {
          if (arguments.length < 1 || !(arguments[0] instanceof IContext)) {
            throw new HaraException("IInvokeIn/invoke-in expects a context");
          }
          return ((IInvokeIn) receiver)
              .invokeIn(
                  (IContext) arguments[0], Arrays.copyOfRange(arguments, 1, arguments.length));
        });
  }

  public static void installExceptionInfo(HaraProtocol protocol) {
    protocol.extend(IExInfo.class, "data", (receiver, arguments) -> ((IExInfo) receiver).getData());
  }

  public static void installPair(HaraProtocol protocol) {
    protocol.extend(
        IPair.class, "key", (receiver, arguments) -> ((Map.Entry<?, ?>) receiver).getKey());
    protocol.extend(
        IPair.class, "value", (receiver, arguments) -> ((Map.Entry<?, ?>) receiver).getValue());
  }

  public static void installComponent(HaraProtocol protocol) {
    protocol.extend(
        IComponent.class, "props", (receiver, arguments) -> ((IComponent) receiver).getProps());
    protocol.extend(
        IComponent.class, "status", (receiver, arguments) -> ((IComponent) receiver).getStatus());
    protocol.extend(
        IComponent.class, "started?", (receiver, arguments) -> ((IComponent) receiver).isStarted());
    protocol.extend(
        IComponent.class, "stopped?", (receiver, arguments) -> ((IComponent) receiver).isStopped());
    protocol.extend(
        IComponent.class, "start", (receiver, arguments) -> ((IComponent) receiver).start());
    protocol.extend(
        IComponent.class, "stop", (receiver, arguments) -> ((IComponent) receiver).stop());
    protocol.extend(
        IComponent.class, "kill", (receiver, arguments) -> ((IComponent) receiver).kill());
    protocol.extend(
        IComponent.class, "remote?", (receiver, arguments) -> ((IComponent) receiver).isRemote());
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

  private static Object lookupValue(ILookup<?, ?> lookup, Object[] arguments) {
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
    return new Tuple.Tup2.L<>(null, index, tuple.nth(index));
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
    if (conj instanceof hara.lang.data.Seq<?>) {
      throw new HaraException("protocol/unsupported-receiver: IConj/conj does not support Seq");
    }
    if (conj instanceof ISetType<?> && value == null) {
      value = HaraNull.SINGLETON;
    }
    if (conj instanceof IMapType<?, ?> && value instanceof ILinearType<?> pair && pair.count() == 2) {
      value = new java.util.AbstractMap.SimpleImmutableEntry<>(pair.nth(0), pair.nth(1));
    }
    return ((IConj<Object>) conj).conj(value);
  }

  @SuppressWarnings("unchecked")
  private static Object findValue(IFind<?, ?> find, Object key) {
    if (find instanceof ISequentialLookupType<?> sequential) {
      long index = sequentialLookupIndex(key);
      return index < 0 || index >= sequential.count()
          ? null
          : new hara.lang.data.Tuple.Tup2.L<>(null, index, sequential.nth(index));
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

  private static Object setValue(ISetType<?> set, Object[] arguments) {
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
  private static Object consSequential(ISequentialType<?> sequential, Object value) {
    hara.lang.data.Seq<Object> tail =
        hara.lang.data.Seq.create(((ISequentialType<Object>) sequential).iterator());
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
  private static Object applyFunction(IFn<?, ?, ?> function, Object[] arguments) {
    return IFn.applyAsArray((IFn) function, arguments);
  }
}
