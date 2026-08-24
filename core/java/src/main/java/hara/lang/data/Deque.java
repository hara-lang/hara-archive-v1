package hara.lang.data;

import hara.lang.base.Ex;
import hara.lang.protocol.ILinearType;
import hara.lang.data.types.ISequentialLookupType;
import hara.lang.data.types.ObjPersistent;
import hara.lang.protocol.IAssoc;
import hara.lang.protocol.IMetadata;
import hara.lang.protocol.IPopFirst;
import hara.lang.protocol.IPopLast;
import hara.lang.protocol.IPushFirst;
import hara.lang.protocol.IPushLast;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Iterator;

/** Persistent deque backed by a count-measured finger tree. */
public interface Deque<E>
    extends ILinearType<E>,
        ISequentialLookupType<E>,
        IAssoc<Long, E>,
        IPushFirst<E>,
        IPushLast<E>,
        IPopFirst,
        IPopLast {
  abstract class Item<E> {
    abstract long measure();
    abstract E get(long index);
    abstract Item<E> replace(long index, E value);
    abstract void collect(ArrayList<E> output);

    java.util.List<Item<E>> children() {
      throw new IllegalStateException("leaf has no children");
    }
  }

  final class Leaf<E> extends Item<E> {
    final E value;
    Leaf(E value) { this.value = value; }
    long measure() { return 1; }
    E get(long index) { return index == 0 ? value : null; }
    Item<E> replace(long index, E next) { return index == 0 ? new Leaf<>(next) : null; }
    void collect(ArrayList<E> output) { output.add(value); }
  }

  final class Branch<E> extends Item<E> {
    final long measure;
    final java.util.List<Item<E>> children;
    Branch(java.util.List<Item<E>> children) {
      if (children.size() < 2 || children.size() > 3) throw new IllegalArgumentException();
      this.children = immutable(children);
      long total = 0;
      for (Item<E> child : children) total += child.measure();
      this.measure = total;
    }
    long measure() { return measure; }
    E get(long index) {
      for (Item<E> child : children) {
        if (index < child.measure()) return child.get(index);
        index -= child.measure();
      }
      return null;
    }
    Item<E> replace(long index, E value) {
      for (int position = 0; position < children.size(); position++) {
        Item<E> child = children.get(position);
        if (index < child.measure()) {
          Item<E> replacement = child.replace(index, value);
          if (replacement == null) return null;
          ArrayList<Item<E>> next = new ArrayList<>(children);
          next.set(position, replacement);
          return new Branch<>(next);
        }
        index -= child.measure();
      }
      return null;
    }
    void collect(ArrayList<E> output) { for (Item<E> child : children) child.collect(output); }
    java.util.List<Item<E>> children() { return children; }
  }

  abstract class FingerTree<E> {
    abstract long measure();
    abstract FingerTree<E> pushFirst(Item<E> item);
    abstract FingerTree<E> pushLast(Item<E> item);
    abstract View<E> popFirst();
    abstract View<E> popLast();
    abstract E get(long index);
    abstract FingerTree<E> replace(long index, E value);
    abstract void collect(ArrayList<E> output);

    static <E> FingerTree<E> empty() { return new Empty<>(); }
    static <E> FingerTree<E> fromItems(java.util.List<Item<E>> items) {
      FingerTree<E> tree = empty();
      for (Item<E> item : items) tree = tree.pushLast(item);
      return tree;
    }
  }

  final class Empty<E> extends FingerTree<E> {
    long measure() { return 0; }
    FingerTree<E> pushFirst(Item<E> item) { return new Single<>(item); }
    FingerTree<E> pushLast(Item<E> item) { return new Single<>(item); }
    View<E> popFirst() { return null; }
    View<E> popLast() { return null; }
    E get(long index) { return null; }
    FingerTree<E> replace(long index, E value) { return null; }
    void collect(ArrayList<E> output) {}
  }

  final class Single<E> extends FingerTree<E> {
    final Item<E> item;
    Single(Item<E> item) { this.item = item; }
    long measure() { return item.measure(); }
    FingerTree<E> pushFirst(Item<E> next) {
      return new Deep<>(items(next), FingerTree.empty(), items(item));
    }
    FingerTree<E> pushLast(Item<E> next) {
      return new Deep<>(items(item), FingerTree.empty(), items(next));
    }
    View<E> popFirst() { return new View<>(item, FingerTree.empty()); }
    View<E> popLast() { return new View<>(item, FingerTree.empty()); }
    E get(long index) { return item.get(index); }
    FingerTree<E> replace(long index, E value) {
      Item<E> replacement = item.replace(index, value);
      return replacement == null ? null : new Single<>(replacement);
    }
    void collect(ArrayList<E> output) { item.collect(output); }
  }

  final class Deep<E> extends FingerTree<E> {
    final long measure;
    final java.util.List<Item<E>> prefix;
    final FingerTree<E> middle;
    final java.util.List<Item<E>> suffix;
    Deep(java.util.List<Item<E>> prefix, FingerTree<E> middle, java.util.List<Item<E>> suffix) {
      if (prefix.isEmpty() || prefix.size() > 4 || suffix.isEmpty() || suffix.size() > 4)
        throw new IllegalArgumentException();
      this.prefix = immutable(prefix);
      this.middle = middle;
      this.suffix = immutable(suffix);
      long total = middle.measure();
      for (Item<E> item : prefix) total += item.measure();
      for (Item<E> item : suffix) total += item.measure();
      this.measure = total;
    }
    long measure() { return measure; }
    FingerTree<E> pushFirst(Item<E> item) {
      if (prefix.size() < 4) {
        ArrayList<Item<E>> next = new ArrayList<>(); next.add(item); next.addAll(prefix);
        return new Deep<>(next, middle, suffix);
      }
      return new Deep<>(items(item, prefix.get(0)),
          middle.pushFirst(new Branch<>(prefix.subList(1, 4))), suffix);
    }
    FingerTree<E> pushLast(Item<E> item) {
      if (suffix.size() < 4) {
        ArrayList<Item<E>> next = new ArrayList<>(suffix); next.add(item);
        return new Deep<>(prefix, middle, next);
      }
      return new Deep<>(prefix, middle.pushLast(new Branch<>(suffix.subList(0, 3))),
          items(suffix.get(3), item));
    }
    View<E> popFirst() {
      Item<E> first = prefix.get(0);
      if (prefix.size() > 1) return new View<>(first, new Deep<>(prefix.subList(1, prefix.size()), middle, suffix));
      View<E> view = middle.popFirst();
      return view == null
          ? new View<>(first, FingerTree.fromItems(suffix))
          : new View<>(first, new Deep<>(view.item.children(), view.tree, suffix));
    }
    View<E> popLast() {
      Item<E> last = suffix.get(suffix.size() - 1);
      if (suffix.size() > 1) return new View<>(last, new Deep<>(prefix, middle, suffix.subList(0, suffix.size() - 1)));
      View<E> view = middle.popLast();
      return view == null
          ? new View<>(last, FingerTree.fromItems(prefix))
          : new View<>(last, new Deep<>(prefix, view.tree, view.item.children()));
    }
    E get(long index) {
      for (Item<E> item : prefix) { if (index < item.measure()) return item.get(index); index -= item.measure(); }
      if (index < middle.measure()) return middle.get(index);
      index -= middle.measure();
      for (Item<E> item : suffix) { if (index < item.measure()) return item.get(index); index -= item.measure(); }
      return null;
    }
    FingerTree<E> replace(long index, E value) {
      ArrayList<Item<E>> next;
      for (int position = 0; position < prefix.size(); position++) {
        Item<E> item = prefix.get(position);
        if (index < item.measure()) { next = new ArrayList<>(prefix); next.set(position, item.replace(index, value)); return new Deep<>(next, middle, suffix); }
        index -= item.measure();
      }
      if (index < middle.measure()) return new Deep<>(prefix, middle.replace(index, value), suffix);
      index -= middle.measure();
      for (int position = 0; position < suffix.size(); position++) {
        Item<E> item = suffix.get(position);
        if (index < item.measure()) { next = new ArrayList<>(suffix); next.set(position, item.replace(index, value)); return new Deep<>(prefix, middle, next); }
        index -= item.measure();
      }
      return null;
    }
    void collect(ArrayList<E> output) {
      for (Item<E> item : prefix) item.collect(output);
      middle.collect(output);
      for (Item<E> item : suffix) item.collect(output);
    }
  }

  final class View<E> {
    final Item<E> item;
    final FingerTree<E> tree;
    View(Item<E> item, FingerTree<E> tree) { this.item = item; this.tree = tree; }
  }

  @SafeVarargs
  static <E> java.util.List<Item<E>> items(Item<E>... values) {
    ArrayList<Item<E>> output = new ArrayList<>();
    Collections.addAll(output, values);
    return output;
  }
  static <E> java.util.List<Item<E>> immutable(java.util.List<Item<E>> values) {
    return Collections.unmodifiableList(new ArrayList<>(values));
  }

  final class Standard<E> extends ObjPersistent implements Deque<E> {
    private final FingerTree<E> tree;
    private static final Standard<Object> EMPTY = new Standard<>(null, FingerTree.empty());
    private Standard(IMetadata metadata, FingerTree<E> tree) { super(metadata); this.tree = tree; }
    @SuppressWarnings("unchecked") public static <E> Standard<E> empty(IMetadata metadata) {
      Standard<E> empty = (Standard<E>) EMPTY;
      return metadata == null ? empty : empty.withMeta(metadata);
    }
    @SafeVarargs public static <E> Standard<E> from(IMetadata metadata, E... values) {
      Standard<E> output = empty(metadata);
      for (E value : values) output = output.pushLast(value);
      return output;
    }
    public static <E> Standard<E> into(Iterator<E> values) {
      Standard<E> output = empty(null);
      while (values.hasNext()) output = output.pushLast(values.next());
      return output;
    }
    public long count() { return tree.measure(); }
    public E nth(long index) {
      if (index < 0 || index >= count()) throw new Ex.OutOfBounds();
      return tree.get(index);
    }
    public Iterator<E> iterator() { ArrayList<E> values = new ArrayList<>(); tree.collect(values); return values.iterator(); }
    public Standard<E> pushFirst(E value) { return new Standard<>(_meta, tree.pushFirst(new Leaf<>(value))); }
    public Standard<E> pushLast(E value) { return new Standard<>(_meta, tree.pushLast(new Leaf<>(value))); }
    public Standard<E> popFirst() { View<E> view = tree.popFirst(); return view == null ? this : new Standard<>(_meta, view.tree); }
    public Standard<E> popLast() { View<E> view = tree.popLast(); return view == null ? this : new Standard<>(_meta, view.tree); }
    public Standard<E> assoc(Long index, E value) {
      FingerTree<E> next = index == null ? null : tree.replace(index, value);
      if (next == null) throw new Ex.OutOfBounds();
      return new Standard<>(_meta, next);
    }
    public Standard<E> withMeta(IMetadata metadata) { return metadata == _meta ? this : new Standard<>(metadata, tree); }
    public Standard<E> empty() { return empty(_meta); }
  }
}
