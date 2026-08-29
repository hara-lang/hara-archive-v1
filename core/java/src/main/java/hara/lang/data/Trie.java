package hara.lang.data;

import hara.lang.base.G;
import hara.lang.base.Iter;
import hara.lang.data.types.ObjMutable;
import hara.lang.data.types.ObjPersistent;
import hara.lang.protocol.*;

import java.util.*;
import java.util.List;
import java.util.Map;
import java.util.Map.Entry;
import java.util.function.Function;

public interface Trie<V>
    extends IColl<String>, ICount, IObjType, IAssoc<String, V>, IDissoc<String>, ILookup<String, V> {

  @Override
  Trie<V> assoc(String key, V val);

  @Override
  Trie<V> dissoc(String key);

  @Override
  default IPair<String, V> find(String key) {
    Node<V> current = rootNode();
    for (char ch : key.toCharArray()) {
      Node<V> node = current.getChildren().get(ch);
      if (node == null) {
        return null;
      }
      current = node;
    }
    return current.isEndOfWord()
        ? new MapEntry<>(null, key, current.getValue())
        : null;
  }

  @Override
  default Iterator<String> keys() {
    return new TrieIterator<>(rootNode());
  }

  @Override
  default Iterator<V> vals() {
    return Iter.map(new TrieEntryIterator<>(rootNode()), Entry::getValue);
  }

  Node<V> rootNode();

  @Override
  default Constant.ObjType getObjType() {
    return Constant.ObjType.MAP;
  }

  @Override
  default String startString() {
    return "#{";
  }

  @Override
  default String endString() {
    return "}";
  }

  @Override
  default long hashCalc(Constant.HashType t) {
    Function<Object, Long> f = G.hashFn(t);
    long acc = Long.valueOf(hashSeed().hashCode());
    Iterator<Entry<String, V>> it = new TrieEntryIterator<>(rootNode());
    while (it.hasNext()) {
      Entry<String, V> entry = it.next();
      acc += f.apply(entry.getKey()) + f.apply(entry.getValue());
    }
    return acc;
  }

  @Override
  default boolean equality(Object other) {
    if (other instanceof Trie) {
      Trie<?> otherTrie = (Trie<?>) other;
      if (count() != otherTrie.count()) {
        return false;
      }
      Iterator<String> it = iterator();
      while (it.hasNext()) {
        String key = it.next();
        if (!otherTrie.has(key)
            || !Objects.equals(find(key).getValue(), otherTrie.find(key).getValue())) {
          return false;
        }
      }
      return true;
    }
    return false;
  }

  public static class Node<V> {
    private final Map<Character, Node<V>> children;
    private V value;
    private boolean isEndOfWord;

    public Node() {
      this.children = new HashMap<>();
      this.isEndOfWord = false;
      this.value = null;
    }

    public Node(Node<V> other) {
      this.children = new HashMap<>(other.children);
      this.isEndOfWord = other.isEndOfWord;
      this.value = other.value;
    }

    public Map<Character, Node<V>> getChildren() {
      return children;
    }

    public V getValue() {
      return value;
    }

    public void setValue(V value) {
      this.value = value;
    }

    public boolean isEndOfWord() {
      return isEndOfWord;
    }

    public void setEndOfWord(boolean endOfWord) {
      isEndOfWord = endOfWord;
    }
  }

  public static class TrieEntryIterator<V> implements Iterator<Entry<String, V>> {
    private final Stack<AbstractMap.SimpleEntry<Node<V>, String>> stack = new Stack<>();
    private Entry<String, V> nextValue;

    public TrieEntryIterator(Node<V> root) {
      if (root != null) {
        stack.push(new AbstractMap.SimpleEntry<>(root, ""));
      }
      findNext();
    }

    @Override
    public boolean hasNext() {
      return nextValue != null;
    }

    @Override
    public Entry<String, V> next() {
      if (!hasNext()) {
        throw new NoSuchElementException();
      }
      Entry<String, V> result = nextValue;
      findNext();
      return result;
    }

    private void findNext() {
      nextValue = null;
      while (!stack.isEmpty()) {
        AbstractMap.SimpleEntry<Node<V>, String> entry = stack.pop();
        Node<V> node = entry.getKey();
        String prefix = entry.getValue();

        List<Character> sortedKeys = new ArrayList<>(node.getChildren().keySet());
        Collections.sort(sortedKeys, Collections.reverseOrder());

        for (char ch : sortedKeys) {
          stack.push(new AbstractMap.SimpleEntry<>(node.getChildren().get(ch), prefix + ch));
        }

        if (node.isEndOfWord()) {
          nextValue = new MapEntry<>(null, prefix, node.getValue());
          return;
        }
      }
    }
  }

  public static class TrieIterator<V> implements Iterator<String> {
    private final TrieEntryIterator<V> entryIterator;

    public TrieIterator(Node<V> root) {
      this.entryIterator = new TrieEntryIterator<>(root);
    }

    @Override
    public boolean hasNext() {
      return entryIterator.hasNext();
    }

    @Override
    public String next() {
      return entryIterator.next().getKey();
    }
  }

  public class Mutable<V> extends ObjMutable implements Trie<V> {
    private Node<V> root;
    private int _size;

    public Mutable() {
      super(null);
      this.root = new Node<>();
      this._size = 0;
    }

    @Override
    public Node<V> rootNode() {
      return root;
    }

    public Mutable(IMetadata meta) {
      super(meta);
      this.root = new Node<>();
      this._size = 0;
    }

    @Override
    public Trie<V> assoc(String key, V val) {
      Node<V> current = root;
      for (char ch : key.toCharArray()) {
        current = current.getChildren().computeIfAbsent(ch, c -> new Node<>());
      }
      if (!current.isEndOfWord()) {
        _size++;
      }
      current.setEndOfWord(true);
      current.setValue(val);
      return this;
    }

    @Override
    public Trie<V> dissoc(String key) {
      if (has(key)) {
        dissocHelper(root, key, 0);
        _size--;
      }
      return this;
    }

    private boolean dissocHelper(Node<V> current, String key, int index) {
      if (index == key.length()) {
        if (!current.isEndOfWord()) {
          return false;
        }
        current.setEndOfWord(false);
        current.setValue(null);
        return current.getChildren().isEmpty();
      }
      char ch = key.charAt(index);
      Node<V> node = current.getChildren().get(ch);
      if (node == null) {
        return false;
      }
      boolean shouldDeleteCurrentNode = dissocHelper(node, key, index + 1);
      if (shouldDeleteCurrentNode) {
        current.getChildren().remove(ch);
        return current.getChildren().isEmpty() && !current.isEndOfWord();
      }
      return false;
    }

    @Override
    public long count() {
      return _size;
    }

    @Override
    public IEmpty empty() {
      return new Mutable<V>();
    }

    @Override
    public Iterator<String> iterator() {
      return new TrieIterator<>(root);
    }

    @Override
    public Trie<V> conj(String s) {
      return assoc(s, null);
    }
  }

  public class Standard<V> extends ObjPersistent implements Trie<V> {
    private final Node<V> root;
    private final int _size;

    public Standard() {
      super(null);
      this.root = new Node<>();
      this._size = 0;
    }

    @Override
    public Node<V> rootNode() {
      return root;
    }

    public Standard(IMetadata meta, Node<V> root, int size) {
      super(meta);
      this.root = root;
      this._size = size;
    }

    @Override
    public Trie<V> assoc(String key, V val) {
      Node<V> newRoot = new Node<>(root);
      Node<V> current = newRoot;
      boolean isNewWord = !has(key);

      for (char ch : key.toCharArray()) {
        Node<V> child = current.getChildren().get(ch);
        Node<V> newNode = (child == null) ? new Node<>() : new Node<>(child);
        current.getChildren().put(ch, newNode);
        current = newNode;
      }
      current.setEndOfWord(true);
      current.setValue(val);
      return new Standard<>(_meta, newRoot, isNewWord ? _size + 1 : _size);
    }

    @Override
    public Trie<V> dissoc(String key) {
      int newSize = has(key) ? _size - 1 : _size;
      Node<V> newRoot = new Node<>(root);
      dissocHelper(newRoot, key, 0);
      return new Standard<>(_meta, newRoot, newSize);
    }

    private boolean dissocHelper(Node<V> current, String key, int index) {
      if (index == key.length()) {
        if (!current.isEndOfWord()) {
          return false;
        }
        current.setEndOfWord(false);
        current.setValue(null);
        return current.getChildren().isEmpty();
      }
      char ch = key.charAt(index);
      Node<V> node = current.getChildren().get(ch);
      if (node == null) {
        return false;
      }
      Node<V> newNode = new Node<>(node);
      current.getChildren().put(ch, newNode);
      boolean shouldDeleteCurrentNode = dissocHelper(newNode, key, index + 1);
      if (shouldDeleteCurrentNode) {
        current.getChildren().remove(ch);
        return current.getChildren().isEmpty() && !current.isEndOfWord();
      }
      return false;
    }

    @Override
    public long count() {
      return _size;
    }

    @Override
    public IEmpty empty() {
      return new Standard<V>();
    }

    @Override
    public Iterator<String> iterator() {
      return new TrieIterator<>(root);
    }

    @Override
    public IObjType withMeta(IMetadata meta) {
      return new Standard<>(meta, root, _size);
    }

    @Override
    public Trie<V> conj(String s) {
      return assoc(s, null);
    }
  }
}
