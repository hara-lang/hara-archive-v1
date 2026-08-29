package hara.lang.data;

import hara.lang.protocol.IMapType;
import hara.lang.data.types.ObjPersistent;
import hara.lang.protocol.IMetadata;
import hara.lang.protocol.IPeekFirst;
import hara.lang.protocol.IPeekLast;
import hara.lang.protocol.IPopFirst;
import hara.lang.protocol.IPopLast;
import java.util.ArrayList;
import java.util.Iterator;
import java.util.Map.Entry;

/** Persistent map iterated by ascending priority with stable insertion-order ties. */
public interface PriorityMap<K, V>
    extends IMapType<K, V>, IPeekFirst<Entry<K, V>>, IPeekLast<Entry<K, V>>, IPopFirst, IPopLast {

  final class Standard<K, V extends Comparable<? super V>> extends ObjPersistent
      implements PriorityMap<K, V> {
    private final Map.Standard<K, V> priorities;
    private final SortedMap.Standard<V, OrderedMap.Standard<K, Boolean>> buckets;

    private Standard(
        IMetadata metadata,
        Map.Standard<K, V> priorities,
        SortedMap.Standard<V, OrderedMap.Standard<K, Boolean>> buckets) {
      super(metadata);
      this.priorities = priorities;
      this.buckets = buckets;
    }

    @SuppressWarnings("unchecked")
    public static <K, V extends Comparable<? super V>> Standard<K, V> empty(IMetadata metadata) {
      return new Standard<>(metadata, (Map.Standard<K, V>) Map.Standard.EMPTY, SortedMap.Standard.empty(null));
    }

    @SuppressWarnings({"rawtypes", "unchecked"})
    public static Standard from(IMetadata metadata, Object... values) {
      if ((values.length & 1) != 0) throw new IllegalArgumentException("priority-map expects key/value pairs");
      Standard output = empty(metadata);
      for (int index = 0; index < values.length; index += 2)
        output = output.assoc(values[index], (Comparable) values[index + 1]);
      return output;
    }

    public long count() { return priorities.count(); }
    public Entry<K, V> find(K key) { return priorities.find(key); }

    public Iterator<Entry<K, V>> iterator() {
      ArrayList<Entry<K, V>> output = new ArrayList<>();
      for (Entry<V, OrderedMap.Standard<K, Boolean>> priority : buckets) {
        for (Entry<K, Boolean> key : priority.getValue())
          output.add(new MapEntry<>(null, key.getKey(), priority.getKey()));
      }
      return output.iterator();
    }

    public Standard<K, V> assoc(K key, V priority) {
      Entry<K, V> current = priorities.find(key);
      if (current != null && java.util.Objects.equals(current.getValue(), priority)) return this;
      SortedMap.Standard<V, OrderedMap.Standard<K, Boolean>> nextBuckets = buckets;
      if (current != null) {
        V previous = current.getValue();
        OrderedMap.Standard<K, Boolean> bucket = nextBuckets.lookup(previous);
        OrderedMap.Standard<K, Boolean> next = bucket.dissoc(key);
        nextBuckets = next.count() == 0 ? nextBuckets.dissoc(previous) : nextBuckets.assoc(previous, next);
      }
      OrderedMap.Standard<K, Boolean> bucket = nextBuckets.lookup(priority);
      if (bucket == null) bucket = new OrderedMap.Standard<>(null);
      bucket = bucket.assoc(key, Boolean.TRUE);
      return new Standard<>(_meta, priorities.assoc(key, priority), nextBuckets.assoc(priority, bucket));
    }

    public Standard<K, V> dissoc(K key) {
      Entry<K, V> current = priorities.find(key);
      if (current == null) return this;
      V priority = current.getValue();
      OrderedMap.Standard<K, Boolean> bucket = buckets.lookup(priority);
      OrderedMap.Standard<K, Boolean> next = bucket.dissoc(key);
      SortedMap.Standard<V, OrderedMap.Standard<K, Boolean>> nextBuckets =
          next.count() == 0 ? buckets.dissoc(priority) : buckets.assoc(priority, next);
      return new Standard<>(_meta, priorities.dissoc(key), nextBuckets);
    }

    public Entry<K, V> peekFirst() { Iterator<Entry<K, V>> values = iterator(); return values.hasNext() ? values.next() : null; }
    public Entry<K, V> peekLast() { Entry<K, V> last = null; for (Entry<K, V> value : this) last = value; return last; }
    public Standard<K, V> popFirst() { Entry<K, V> value = peekFirst(); return value == null ? this : dissoc(value.getKey()); }
    public Standard<K, V> popLast() { Entry<K, V> value = peekLast(); return value == null ? this : dissoc(value.getKey()); }
    public Standard<K, V> withMeta(IMetadata metadata) { return metadata == _meta ? this : new Standard<>(metadata, priorities, buckets); }
    public Standard<K, V> empty() { return empty(_meta); }
  }
}
