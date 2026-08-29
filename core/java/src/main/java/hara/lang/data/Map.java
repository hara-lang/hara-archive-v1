package hara.lang.data;

import hara.lang.base.Eq;
import hara.lang.base.Ex;
import hara.lang.base.G;
import hara.lang.base.Iter;
import hara.lang.base.primitive.Array;
import hara.lang.base.primitive.Counter;
import hara.lang.protocol.IMapType;
import hara.lang.data.types.ObjMutable;
import hara.lang.data.types.ObjPersistent;
import hara.lang.protocol.*;

import java.util.Iterator;
import java.util.Map.Entry;
import java.util.concurrent.atomic.AtomicReference;

public interface Map<K, V> extends IMapType<K, V> {

  public interface Node<K, V> {
    Node<K, V> assoc(
        AtomicReference<Thread> edit, int shift, int hash, K key, V val, Counter added_leaf);

    Node<K, V> without(
        AtomicReference<Thread> edit, int shift, int hash, K key, Counter removed_leaf);

    IPair<K, V> find(int shift, int hash, K key);

    boolean isSingle();

    boolean hasData();

    boolean hasNodes();

    Node<K, V> getNode(int node_idx);

    Object[] getArray();

    int nodeArity();

    int dataArity();
  }

  public interface S {

    public static int mask(int hash, int shift) {
      return (hash >>> shift) & 0x01f;
    }

    public static int bitpos(int hash, int shift) {
      return 1 << mask(hash, shift);
    }

    public static boolean isAllowedToEdit(AtomicReference<Thread> x, AtomicReference<Thread> y) {
      return x != null && y != null && (x == y || x.get() == y.get());
    }

    public static Object[] removePair(Object[] array, int i) {
      Object[] newArray = new Object[array.length - 2];
      System.arraycopy(array, 0, newArray, 0, 2 * i);
      System.arraycopy(array, 2 * (i + 1), newArray, 2 * i, newArray.length - 2 * i);
      return newArray;
    }
  }

  public final class DataNode<K, V> implements Node<K, V> {

    @SuppressWarnings({"rawtypes", "unchecked"})
    public static final DataNode EMPTY = new DataNode(null, 0, 0, new Object[0]);

    int _datamap;
    int _nodemap;
    Object[] _array;
    final AtomicReference<Thread> _edit;

    int bitmapNodeIndex(final int bitmap, final int bitpos) {
      return Integer.bitCount(bitmap & (bitpos - 1));
    }

    int nodeAt(final int bit) {
      return _array.length - 1 - bitmapNodeIndex(_nodemap, bit);
    }

    public DataNode(AtomicReference<Thread> edit, int datamap, int nodemap, Object[] array) {
      _edit = edit;
      _datamap = datamap;
      _nodemap = nodemap;
      _array = array;
    }

    private Node<K, V> copyAndSet(AtomicReference<Thread> edit, int idx, Object val) {
      if (S.isAllowedToEdit(edit, _edit)) {
        _array[idx] = val;
        return this;
      } else {
        final Object[] new_array = _array.clone();
        new_array[idx] = val;
        return new DataNode<K, V>(edit, _datamap, _nodemap, new_array);
      }
    }

    private Node<K, V> mergeTwoKeyValuePairs(
        AtomicReference<Thread> edit,
        int shift,
        int current_hash,
        Object current_key,
        Object current_val,
        int hash,
        Object key,
        Object val) {
      if ((32 < shift) && (current_hash == hash)) {
        return new BranchNode<K, V>(
            edit, current_hash, 2, new Object[] {current_key, current_val, key, val});
      } else {
        final int current_mask = S.mask(current_hash, shift);
        final int mask = S.mask(hash, shift);

        if (current_mask == mask) {
          final Node<K, V> new_node =
              mergeTwoKeyValuePairs(
                  edit, (shift + 5), current_hash, current_key, current_val, hash, key, val);
          return new DataNode<K, V>(
              edit, 0, S.bitpos(current_hash, shift), new Object[] {new_node});
        } else {
          final int new_datamap = S.bitpos(current_hash, shift) | S.bitpos(hash, shift);

          if (current_mask < mask) {
            return new DataNode<K, V>(
                edit, new_datamap, 0, new Object[] {current_key, current_val, key, val});
          } else {
            return new DataNode<K, V>(
                edit, new_datamap, 0, new Object[] {key, val, current_key, current_val});
          }
        }
      }
    }

    private Node<K, V> copyAndMigrateToNode(
        AtomicReference<Thread> edit, int bit, Node<K, V> node) {
      final int idx_old = (2 * bitmapNodeIndex(_datamap, bit));
      final int idx_new = (_array.length - 2 - bitmapNodeIndex(_nodemap, bit));
      final Object[] dst = new Object[(_array.length - 1)];

      System.arraycopy(_array, 0, dst, 0, idx_old);
      System.arraycopy(_array, (2 + idx_old), dst, idx_old, (idx_new - idx_old));
      dst[idx_new] = node;
      System.arraycopy(_array, (2 + idx_new), dst, (idx_new + 1), (_array.length - 2 - idx_new));

      return new DataNode<K, V>(edit, (_datamap ^ bit), (_nodemap | bit), dst);
    }

    @Override
    @SuppressWarnings("unchecked")
    public Node<K, V> assoc(
        AtomicReference<Thread> edit, int shift, int hash, K key, V val, Counter added_leaf) {
      final int bit = S.bitpos(hash, shift);

      if ((_datamap & bit) != 0) {
        final int idx = bitmapNodeIndex(_datamap, bit);
        final Object current_key = _array[(2 * idx)];
        if (Eq.eq(key, current_key)) {
          return copyAndSet(edit, ((2 * idx) + 1), val);
        } else {
          final Object current_val = _array[((2 * idx) + 1)];
          final Node<K, V> new_node =
              mergeTwoKeyValuePairs(
                  edit,
                  (shift + 5),
                  (int) G.hashRapid(current_key),
                  current_key,
                  current_val,
                  hash,
                  key,
                  val);
          added_leaf.inc();

          return copyAndMigrateToNode(edit, bit, new_node);
        }
      } else if ((_nodemap & bit) != 0) {
        final int node_idx = nodeAt(bit);
        final Node<K, V> sub_node = (Node<K, V>) _array[node_idx];
        final Node<K, V> sub_node_new =
            sub_node.assoc(edit, (shift + 5), hash, key, val, added_leaf);

        if (sub_node == sub_node_new) {
          return this;
        } else {
          return copyAndSet(edit, node_idx, sub_node_new);
        }
      } else {
        final int n = _array.length;
        final int idx = (2 * bitmapNodeIndex(_datamap, bit));
        final Object[] new_array = new Object[(2 + n)];

        System.arraycopy(_array, 0, new_array, 0, idx);
        new_array[idx] = key;
        new_array[(idx + 1)] = val;
        System.arraycopy(_array, idx, new_array, (2 + idx), (n - idx));
        added_leaf.inc();

        return new DataNode<K, V>(edit, (_datamap | bit), _nodemap, new_array);
      }
    }

    @Override
    public boolean isSingle() {
      return ((0 == _nodemap) && (1 == Integer.bitCount(_datamap)));
    }

    private Node<K, V> copyAndRemoveValue(AtomicReference<Thread> edit, int bit) {
      final int idx = (2 * bitmapNodeIndex(_datamap, bit));
      final Object[] dst = new Object[(_array.length - 2)];
      System.arraycopy(_array, 0, dst, 0, idx);
      System.arraycopy(_array, (idx + 2), dst, idx, (_array.length - idx - 2));

      return new DataNode<K, V>(edit, (_datamap ^ bit), _nodemap, dst);
    }

    private Node<K, V> copyAndMigrateToInline(
        AtomicReference<Thread> edit, int bit, Node<K, V> node) {
      final int idx_old = (_array.length - 1 - bitmapNodeIndex(_nodemap, bit));
      final int idx_new = (2 * bitmapNodeIndex(_datamap, bit));
      final Object[] dst = new Object[(_array.length + 1)];

      System.arraycopy(_array, 0, dst, 0, idx_new);
      dst[idx_new] = node.getArray()[0];
      dst[(idx_new + 1)] = node.getArray()[1];
      System.arraycopy(_array, idx_new, dst, (idx_new + 2), (idx_old - idx_new));
      System.arraycopy(_array, (idx_old + 1), dst, (idx_old + 2), (_array.length - 1 - idx_old));

      return new DataNode<K, V>(edit, (_datamap | bit), (_nodemap ^ bit), dst);
    }

    @Override
    @SuppressWarnings("unchecked")
    public Node<K, V> without(
        AtomicReference<Thread> edit, int shift, int hash, K key, Counter removed_leaf) {
      final int bit = S.bitpos(hash, shift);

      if ((_datamap & bit) != 0) {
        final int idx = bitmapNodeIndex(_datamap, bit);
        ;
        if (Eq.eq(key, _array[(2 * idx)])) {
          removed_leaf.inc();
          if ((Integer.bitCount(_datamap) == 2) && (_nodemap == 0)) {
            final int new_datamap = (shift == 0) ? (_datamap ^ bit) : S.bitpos(hash, 0);
            if (idx == 0) {
              return new DataNode<K, V>(edit, new_datamap, 0, new Object[] {_array[2], _array[3]});
            } else {
              return new DataNode<K, V>(edit, new_datamap, 0, new Object[] {_array[0], _array[1]});
            }
          } else {
            return copyAndRemoveValue(edit, bit);
          }
        } else {
          return this;
        }
      }

      if ((_nodemap & bit) != 0) {
        final int node_idx = nodeAt(bit);
        final Node<K, V> sub_node = (Node<K, V>) _array[node_idx];
        final Node<K, V> sub_node_new =
            sub_node.without(edit, (shift + 5), hash, key, removed_leaf);

        if (sub_node != sub_node_new) {
          if (sub_node_new.isSingle()) {
            if ((_datamap == 0) && (Integer.bitCount(_nodemap) == 1)) {
              return sub_node_new;
            } else {
              return copyAndMigrateToInline(edit, bit, sub_node_new);
            }
          } else {
            return copyAndSet(edit, node_idx, sub_node_new);
          }
        }
      }

      return this;
    }

    @Override
    @SuppressWarnings({"unchecked", "rawtypes"})
    public IPair<K, V> find(int shift, int hash, K key) {
      final int bit = S.bitpos(hash, shift);

      if ((_datamap & bit) != 0) {
        final int idx = bitmapNodeIndex(_datamap, bit);
        final K current_key = (K) _array[2 * idx];
        if (Eq.eq(current_key, key)) {
          return new MapEntry<K, V>(null, current_key, (V) _array[2 * idx + 1]);
        } else {
          return null;
        }
      } else if ((_nodemap & bit) != 0) {
        return ((Node<K, V>) _array[nodeAt(bit)]).find((shift + 5), hash, key);
      } else {
        return null;
      }
    }

    @Override
    public boolean hasNodes() {
      return _nodemap != 0;
    }

    @Override
    public boolean hasData() {
      return _datamap != 0;
    }

    @Override
    public int nodeArity() {
      return Integer.bitCount(_nodemap);
    }

    @Override
    public int dataArity() {
      return Integer.bitCount(_datamap);
    }

    @Override
    @SuppressWarnings("unchecked")
    public Node<K, V> getNode(int node_idx) {
      return (Node<K, V>) _array[(_array.length - node_idx)];
    }

    @Override
    public Object[] getArray() {
      return _array;
    }
  }

  static final class BranchNode<K, V> implements Node<K, V> {

    final int _hash;
    int _size;
    Object[] _array;
    final AtomicReference<Thread> _edit;

    BranchNode(AtomicReference<Thread> edit, int hash, int count, Object... array) {
      _edit = edit;
      _hash = hash;
      _size = count;
      _array = array;
    }

    public int findIndex(Object key) {
      for (int i = 0; i < 2 * _size; i += 2) {
        if (Eq.eq(key, _array[i])) {
          return i;
        }
      }
      return -1;
    }

    private Node<K, V> mutableAssoc(int idx, K key, V val, Counter addedLeaf) {
      if (idx == -1) {
        Object[] new_array = new Object[(_array.length + 2)];
        System.arraycopy(_array, 0, new_array, 0, _array.length);
        new_array[_array.length] = key;
        new_array[(_array.length + 1)] = val;
        addedLeaf.inc();

        _array = new_array;
        _size = _size + 1;
      } else {
        if (_array[(idx + 1)] != val) {
          _array[(idx + 1)] = val;
        }
      }

      return this;
    }

    private Node<K, V> persistentAssoc(int idx, K key, V val, Counter addedLeaf) {
      if (idx == -1) {
        Object[] new_array = new Object[(_array.length + 2)];
        System.arraycopy(_array, 0, new_array, 0, _array.length);
        new_array[_array.length] = key;
        new_array[(_array.length + 1)] = val;
        addedLeaf.inc();

        return new BranchNode<K, V>(_edit, _hash, (_size + 1), new_array);
      } else {
        if (_array[(idx + 1)] == val) {
          return this;
        } else {
          Object[] new_array = _array.clone();
          new_array[(idx + 1)] = val;

          return new BranchNode<K, V>(_edit, _hash, (_size + 1), new_array);
        }
      }
    }

    @Override
    public Node<K, V> assoc(
        AtomicReference<Thread> edit, int shift, int hash, K key, V val, Counter addedLeaf) {
      int idx = findIndex(key);
      if (S.isAllowedToEdit(edit, _edit)) {
        BranchNode<K, V> new_node =
            (edit == _edit) ? this : new BranchNode<K, V>(edit, _hash, _size, _array.clone());
        return new_node.mutableAssoc(idx, key, val, addedLeaf);
      } else {
        return persistentAssoc(idx, key, val, addedLeaf);
      }
    }

    @Override
    public boolean isSingle() {
      return (1 == _size);
    }

    @Override
    @SuppressWarnings("unchecked")
    public Node<K, V> without(
        AtomicReference<Thread> edit, int shift, int hash, K key, Counter removed_leaf) {
      final int idx = findIndex(key);
      if (idx != -1) {
        removed_leaf.inc();
        switch (_size) {
          case 1:
            return DataNode.EMPTY;
          case 2:
            final int hash_idx = (Eq.eq(key, _array[0])) ? 2 : 0;
            return DataNode.EMPTY.assoc(
                edit, 0, hash, _array[hash_idx], _array[(hash_idx + 1)], removed_leaf);
          default:
            return new BranchNode<K, V>(edit, hash, (_size - 1), S.removePair(_array, (idx / 2)));
        }
      }
      return this;
    }

    @Override
    @SuppressWarnings({"unchecked", "rawtypes"})
    public IPair<K, V> find(int shift, int hash, Object key) {
      int idx = findIndex(key);
      if (idx < 0) {
        return null;
      }
      if (Eq.eq(key, _array[idx])) {
        return new MapEntry<K, V>(null, (K) _array[idx], (V) _array[idx + 1]);
      }
      return null;
    }

    @Override
    public boolean hasNodes() {
      return false;
    }

    @Override
    public boolean hasData() {
      return true;
    }

    @Override
    public int nodeArity() {
      return 0;
    }

    @Override
    public int dataArity() {
      return _size;
    }

    @Override
    public Node<K, V> getNode(int node_idx) {
      return null;
    }

    @Override
    public Object[] getArray() {
      return _array;
    }
  }

  @SuppressWarnings("unchecked")
  static final class NodeIter<K, V> implements Iterator<Entry<K, V>> {
    private static final Object NULL = new Object();
    private Object _next = NULL;

    final Node<K, V>[] _nodes = new Node[7];
    final int[] cursor_lengths = new int[7];

    Object[] _array;
    int _lvl = 0;
    int _data_idx;
    int _data_len;

    @SuppressWarnings("rawtypes")
    NodeIter(Node<K, V> node) {

      _array = node.getArray();
      _lvl = 0;
      _data_idx = 0;

      _data_len = node.dataArity();
      _nodes[0] = node;
      cursor_lengths[0] = node.nodeArity();

      if (_data_len == 0) {
        advance();
      } else {
        _data_len = _data_len - 1;
      }

      _next = new MapEntry<>(null, _array[(_data_idx * 2)], _array[((_data_idx * 2) + 1)]);
    }

    @SuppressWarnings("rawtypes")
    private boolean advance() {
      if (_data_idx < _data_len) {
        _data_idx = _data_idx + 1;
        _next = new MapEntry<>(null, _array[(_data_idx * 2)], _array[((_data_idx * 2) + 1)]);

        return true;
      } else {
        while (_lvl >= 0) {
          int node_idx = cursor_lengths[_lvl];
          if (node_idx == 0) {
            _lvl = _lvl - 1;
          } else {
            cursor_lengths[_lvl] = (node_idx - 1);

            Node<K, V> node = _nodes[_lvl].getNode(node_idx);
            boolean has_nodes = node.hasNodes();
            int new_lvl = has_nodes ? (_lvl + 1) : _lvl;

            if (has_nodes) {
              _nodes[new_lvl] = node;
              cursor_lengths[new_lvl] = node.nodeArity();
            }

            if (node.hasData()) {
              _array = node.getArray();
              _lvl = new_lvl;
              _data_idx = 0;
              _data_len = (node.dataArity() - 1);
              _next =
                  new MapEntry<>(null, _array[(_data_idx * 2)], _array[((_data_idx * 2) + 1)]);

              return true;
            }

            _lvl = _lvl + 1;
          }
        }

        return false;
      }
    }

    @Override
    public boolean hasNext() {
      if (_next != NULL) {
        return true;
      }

      return advance();
    }

    @Override
    public IPair<K, V> next() {
      Object ret = _next;
      if (ret != NULL) {
        _next = NULL;
        return (IPair<K, V>) ret;
      } else if (advance()) {
        return next();
      }
      throw new Ex.NoSuchElement();
    }

    @Override
    public void remove() {
      throw new Ex.Unsupported();
    }
  }

  public interface Base<K, V> extends Map<K, V>, IAssoc<K, V>, IObjType {
    abstract Node<K, V> _root();

    abstract int _size();

    @Override
    default IPair<K, V> find(K key) {
      return _root().find(0, (int) G.hashRapid(key), key);
    }

    @SuppressWarnings("unchecked")
    @Override
    default Iterator<Entry<K, V>> iterator() {
      return ((_root() == null) || (_size() == 0))
          ? (Iterator<Entry<K, V>>) Iter.emptyIterator()
          : new NodeIter<K, V>(_root());
    }

    @Override
    default long count() {
      return _size();
    }
  }

  @SuppressWarnings("unchecked")
  public final class Mutable<K, V> extends ObjMutable implements Base<K, V>, IToPersistent {

    private volatile Node<K, V> _root;
    private volatile int _size;

    private final AtomicReference<Thread> _edit;
    private final Counter _leafFlag = new Counter(0);

    public Mutable(IMetadata meta, AtomicReference<Thread> edit, Node<K, V> root, int size) {
      super(meta);
      _edit = edit;
      _root = root;
      _size = size;
    }

    public static <K, V> Mutable<K, V> empty(IMetadata meta) {
      return new Mutable<K, V>(
          meta, new AtomicReference<Thread>(Thread.currentThread()), DataNode.EMPTY, 0);
    }

    @SuppressWarnings("rawtypes")
    public static <K, V> Mutable<K, V> from(IMetadata meta, Object... elements) {
      return into(empty(meta), (Iterator) Iter.partitionPair(Array.toIter(elements)));
    }

    public static <K, V> Mutable<K, V> into(Iterator<Entry<K, V>> it) {
      return into(empty(null), it);
    }

    public static <K, V> Mutable<K, V> into(Mutable<K, V> map, Iterator<Entry<K, V>> it) {
      return Iter.reduce(it, map, (m, e) -> m.assoc(e.getKey(), e.getValue()));
    }

    private void ensureEditable() {
      if (_edit.get() == null) {
        throw new IllegalAccessError("Transient used after persistent! call");
      }
    }

    public static <K, V> Mutable<K, V> create(IMetadata meta, Base<K, V> other) {
      var ret = empty(meta);
      for (Object o : other) {
        var e = (Entry<K, V>) o;
        ret = ret.assoc(e.getKey(), e.getValue());
      }
      return (Mutable<K, V>) ret;
    }

    @Override
    public Mutable<K, V> assoc(K key, V val) {
      _leafFlag.reset(0);

      var n0 = ((_root == null) ? DataNode.EMPTY : _root);
      var n1 = n0.assoc(_edit, 0, (int) G.hashRapid(key), key, val, _leafFlag);
      if (n1 != _root) {
        _root = n1;
      }
      _size += _leafFlag.deref();
      return this;
    }

    @Override
    public Mutable<K, V> dissoc(K key) {
      if (_root == null) return this;
      _leafFlag.reset(0);
      var n = _root.without(_edit, 0, (int) G.hashRapid(key), key, _leafFlag);
      if (n != _root) {
        _root = n;
      }
      _size -= _leafFlag.deref();
      return this;
    }

    @Override
    public Node<K, V> _root() {
      ensureEditable();
      return _root;
    }

    @Override
    public int _size() {
      return _size;
    }

    @Override
    public Standard<K, V> toPersistent() {
      ensureEditable();
      _edit.set(null);
      return new Standard<K, V>(_meta, _size, _root);
    }

    @Override
    public IEmpty empty() {
      _size = 0;
      _root = DataNode.EMPTY;
      return this;
    }
  }

  @SuppressWarnings("unchecked")
  public class Standard<K, V> extends ObjPersistent implements Base<K, V>, IToMutable {

    private final int _size;
    private final Node<K, V> _root;

    @SuppressWarnings("rawtypes")
    public static final Standard EMPTY = new Standard(null, 0, DataNode.EMPTY);

    Standard(IMetadata meta, int size, Node<K, V> root) {
      super(meta);
      _size = size;
      _root = root;
    }

    public static <K, V> Standard<K, V> from(IMetadata meta, Object... elements) {
      return (Standard<K, V>) Mutable.from(meta, elements).toPersistent();
    }

    public static <K, V> Standard<K, V> into(Iterator<Entry<K, V>> it) {
      return Mutable.into(it).toPersistent();
    }

    public static <K, V> Standard<K, V> into(Standard<K, V> map, Iterator<Entry<K, V>> it) {
      return Mutable.into(map.toMutable(), it).toPersistent();
    }

    @Override
    public Standard<K, V> assoc(K key, V val) {
      var added_leaf = new Counter(0);
      var n0 = _root == null ? DataNode.EMPTY : _root;
      var n1 = n0.assoc(null, 0, (int) G.hashRapid(key), key, val, added_leaf);

      return (n1 == _root) ? this : new Standard<K, V>(meta(), _size + added_leaf.deref(), n1);
    }

    @Override
    public Standard<K, V> dissoc(K key) {
      if (_root == null) {
        return this;
      }
      Node<K, V> new_root = _root.without(null, 0, (int) G.hashRapid(key), key, new Counter(0));
      if (new_root == _root) {
        return this;
      }
      return new Standard<K, V>(meta(), (_size - 1), new_root);
    }

    @Override
    public Node<K, V> _root() {
      return _root;
    }

    @Override
    public int _size() {
      return _size;
    }

    @Override
    public Standard<K, V> withMeta(IMetadata meta) {
      return (meta() == meta) ? this : new Standard<K, V>(meta, _size, _root);
    }

    @Override
    public Standard<K, V> empty() {
      return (_meta == null) ? EMPTY : EMPTY.withMeta(_meta);
    }

    @Override
    public Mutable<K, V> toMutable() {
      return new Mutable<K, V>(
          _meta, new AtomicReference<Thread>(Thread.currentThread()), _root, _size);
    }
  }
}
