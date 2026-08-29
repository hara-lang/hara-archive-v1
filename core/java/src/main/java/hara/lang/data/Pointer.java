package hara.lang.data;

import hara.lang.protocol.Constant;
import hara.lang.protocol.IApplicable;
import hara.lang.protocol.ICount;
import hara.lang.protocol.IContext;
import hara.lang.protocol.IDeref;
import hara.lang.protocol.IIter;
import hara.lang.protocol.ILookup;
import hara.lang.protocol.IMetadata;
import hara.lang.protocol.IObjType;
import hara.lang.protocol.IPointer;
import hara.lang.protocol.IMapType;
import java.util.Collections;
import java.util.Iterator;
import java.util.LinkedHashMap;
import java.util.Objects;

/** An immutable context-qualified reference descriptor. */
public final class Pointer
    implements IPointer,
        IApplicable,
        IDeref<Object>,
        ILookup<Object, Object>,
        ICount,
        IIter<java.util.Map.Entry<Object, Object>>,
        hara.lang.protocol.IInvokeIn,
        IObjType {
  private static final Keyword CONTEXT_KEY = Keyword.create("context");

  private final Keyword context;
  private final java.util.Map<Object, Object> values;
  private final IMetadata metadata;

  public Pointer(Object context, java.util.Map<?, ?> values) {
    this(context, values, null);
  }

  private Pointer(Object context, java.util.Map<?, ?> values, IMetadata metadata) {
    if (!(context instanceof Keyword keyword)) {
      throw new IllegalArgumentException("pointer :context must be a keyword");
    }
    this.context = keyword;
    java.util.Map<Object, Object> copied = new LinkedHashMap<>();
    if (values != null) {
      values.forEach(
          (key, value) -> {
            if (!(key instanceof Keyword)) {
              throw new IllegalArgumentException("pointer descriptor fields must use keyword keys");
            }
            if (CONTEXT_KEY.equals(key)) {
              throw new IllegalArgumentException("pointer fields cannot contain :context");
            }
            copied.put(key, value);
          });
    }
    this.values = Collections.unmodifiableMap(copied);
    this.metadata = metadata;
  }

  /** Builds a pointer from its canonical {:context keyword, ...fields} descriptor. */
  public static Pointer fromDescriptor(Object descriptor) {
    java.util.Map<Object, Object> entries = new LinkedHashMap<>();
    if (descriptor instanceof java.util.Map<?, ?> map) {
      map.forEach(entries::put);
    } else if (descriptor instanceof IMapType<?, ?> map) {
      for (Object item : map) {
        if (!(item instanceof java.util.Map.Entry<?, ?> entry)) {
          throw new IllegalArgumentException("pointer expects one descriptor map");
        }
        entries.put(entry.getKey(), entry.getValue());
      }
    } else {
      throw new IllegalArgumentException("pointer expects one descriptor map");
    }
    if (!entries.containsKey(CONTEXT_KEY)) {
      throw new IllegalArgumentException("pointer descriptor requires :context");
    }
    Object context = entries.remove(CONTEXT_KEY);
    return new Pointer(context, entries);
  }

  public Keyword context() {
    return context;
  }

  public java.util.Map<Object, Object> values() {
    return values;
  }

  @Override
  public Object ptrContext() {
    return context;
  }

  @Override
  public Object lookup(Object key) {
    Object value = values.get(key);
    if (value == null && key instanceof Keyword) {
      value = values.get(((Keyword) key).getName());
    }
    return value;
  }

  @Override
  public Object lookup(Object key, Object notFound) {
    Object value = lookup(key);
    return value == null && !containsKey(key) ? notFound : value;
  }

  @Override
  public java.util.Map.Entry<Object, Object> find(Object key) {
    return containsKey(key) ? new MapEntry<>(null, key, lookup(key)) : null;
  }

  @Override
  @SuppressWarnings("unchecked")
  public Iterator<Object> keys() {
    return (Iterator<Object>) (Iterator<?>) values.keySet().iterator();
  }

  @Override
  @SuppressWarnings("unchecked")
  public Iterator<Object> vals() {
    return (Iterator<Object>) (Iterator<?>) values.values().iterator();
  }

  @Override
  public long count() {
    return values.size();
  }

  @Override
  public Iterator<java.util.Map.Entry<Object, Object>> iter() {
    return hara.lang.base.Iter.map(
        values.entrySet().iterator(),
        entry -> new MapEntry<>(null, entry.getKey(), entry.getValue()));
  }

  private boolean containsKey(Object key) {
    return values.containsKey(key)
        || (key instanceof Keyword && values.containsKey(((Keyword) key).getName()));
  }

  @Override
  public Object deref() {
    throw new IllegalStateException("pointer/runtime-unavailable: deref requires an evaluator");
  }

  @Override
  public Object applyDefault() {
    throw new IllegalStateException("pointer/runtime-unavailable: resolution requires an evaluator");
  }

  @Override
  public Object applyIn(Object runtime, Object[] args) {
    Object[] call = new Object[(args == null ? 0 : args.length) + 2];
    call[0] = Keyword.create("pointer", "invoke");
    call[1] = this;
    if (args != null) System.arraycopy(args, 0, call, 2, args.length);
    return requireRuntime(runtime).call(call);
  }

  @Override
  public Object invokeIn(IContext runtime, Object... args) {
    return applyIn(runtime, args);
  }

  @Override
  public Object transformIn(Object runtime, Object[] args) {
    return requireRuntime(runtime).transformInPtr(this, args);
  }

  @Override
  public Object transformOut(Object runtime, Object[] args, Object value) {
    return requireRuntime(runtime).transformOutPtr(this, value);
  }

  @Override
  public IMetadata meta() {
    return metadata;
  }

  @Override
  public String display() {
    Object[] descriptor = new Object[(values.size() + 1) * 2];
    descriptor[0] = CONTEXT_KEY;
    descriptor[1] = context;
    int index = 2;
    for (java.util.Map.Entry<Object, Object> entry : values.entrySet()) {
      descriptor[index++] = entry.getKey();
      descriptor[index++] = entry.getValue();
    }
    return "#ptr " + hara.lang.data.OrderedMap.Standard.from(null, descriptor).display();
  }

  @Override
  public Pointer withMeta(IMetadata meta) {
    return metadata == meta ? this : new Pointer(context, values, meta);
  }

  @Override
  public long hashCalc(Constant.HashType type) {
    return Objects.hash(context, values);
  }

  private IContext requireRuntime(Object runtime) {
    if (runtime instanceof IContext) return (IContext) runtime;
    throw new IllegalArgumentException("Pointer application requires an IContext runtime");
  }

  @Override
  public boolean equals(Object other) {
    return other instanceof Pointer pointer
        && Objects.equals(context, pointer.context)
        && Objects.equals(values, pointer.values);
  }

  @Override
  public int hashCode() {
    return Objects.hash(context, values);
  }

  @Override
  public Constant.ObjType getObjType() {
    return Constant.ObjType.POINTER;
  }

  @Override
  public String getObjName() {
    return "POINTER";
  }
}
