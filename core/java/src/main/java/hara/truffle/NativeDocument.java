package hara.truffle;

import hara.lang.data.Keyword;
import hara.lang.protocol.IMapType;
import hara.lang.protocol.ISequential;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Deque;
import java.util.List;

/** Renderer-neutral document constructors and the native plain-text layout engine. */
final class NativeDocument {
  private NativeDocument() {}

  private sealed interface Op
      permits Text, Pass, Escaped, Line, Break, Begin, End, Nest, Align, Outdent {}

  private record Text(String value, int width) implements Op {}
  private record Pass(String value) implements Op {}
  private record Escaped(String value) implements Op {}
  private record Line(String inline, String terminate) implements Op {}
  private record Break() implements Op {}
  private record Begin(int end) implements Op {}
  private record End() implements Op {}
  private record Nest(long offset) implements Op {}
  private record Align(long offset) implements Op {}
  private record Outdent() implements Op {}

  private sealed interface Task permits Visit, Emit {}
  private record Visit(Object value) implements Task {}
  private record Emit(Op operation) implements Task {}

  static Object operation(String operation, Object[] arguments) {
    String name = operation.replace("std.native.Document/", "");
    Object[] values = new Object[arguments.length];
    for (int index = 0; index < arguments.length; index++) {
      values[index] = HaraBox.unwrap(arguments[index]);
    }
    return switch (name) {
      case "element" -> element(values);
      case "text" -> text(values, "text");
      case "fragment", "group", "pass" -> tagged("document/" + name, values);
      case "annotate" -> {
        if (values.length == 0) {
          throw new HaraException("std.native.Document/annotate expects an annotation");
        }
        yield tagged("document/annotate", values);
      }
      case "escaped" -> {
        if (values.length != 1 || !(values[0] instanceof String)) {
          throw new HaraException("std.native.Document/escaped expects one string");
        }
        yield tagged("document/escaped", values);
      }
      case "line" -> {
        if (values.length > 2) {
          throw new HaraException(
              "std.native.Document/line expects optional inline and terminate strings");
        }
        for (Object value : values) {
          if (!(value instanceof String)) {
            throw new HaraException(
                "std.native.Document/line expects optional inline and terminate strings");
          }
        }
        yield tagged("document/line", values);
      }
      case "break" -> {
        if (values.length != 0) {
          throw new HaraException("std.native.Document/break expects no arguments");
        }
        yield tagged("document/break", values);
      }
      case "nest", "align" -> tagged("document/" + name, values);
      case "normalize" -> {
        requireArity(name, values, 1);
        serialize(values[0]);
        yield values[0];
      }
      case "valid?" -> {
        requireArity(name, values, 1);
        try {
          serialize(values[0]);
          yield true;
        } catch (RuntimeException invalid) {
          yield false;
        }
      }
      case "render" -> render(values);
      default -> throw new HaraException("unknown Document operation: " + name);
    };
  }

  private static Object element(Object[] values) {
    if (values.length == 0 || !(values[0] instanceof Keyword)) {
      throw new HaraException("std.native.Document/element expects a keyword tag");
    }
    return hara.lang.data.Vector.Standard.from(null, values);
  }

  private static Object tagged(String tag, Object[] children) {
    Object[] values = new Object[children.length + 1];
    values[0] = Keyword.create(tag);
    System.arraycopy(children, 0, values, 1, children.length);
    return hara.lang.data.Vector.Standard.from(null, values);
  }

  private static void requireArity(String operation, Object[] values, int arity) {
    if (values.length != arity) {
      throw new HaraException(
          "std.native.Document/" + operation + " expects " + arity + " argument");
    }
  }

  private static String text(Object[] values, String operation) {
    StringBuilder output = new StringBuilder();
    for (Object value : values) {
      if (value instanceof String string) output.append(string);
      else if (value instanceof Character character) output.append(character);
      else {
        throw new HaraException(
            "std.native.Document/" + operation + " expects text values");
      }
    }
    return output.toString();
  }

  private static List<Object> sequential(Object value) {
    if (!(value instanceof ISequential<?> sequence)) return null;
    List<Object> values = new ArrayList<>();
    for (Object element : sequence) values.add(HaraBox.unwrap(element));
    return values;
  }

  private static void pushChildren(Deque<Task> stack, List<Object> values, int from) {
    for (int index = values.size() - 1; index >= from; index--) {
      stack.push(new Visit(values.get(index)));
    }
  }

  private static List<Op> serialize(Object document) {
    Deque<Task> stack = new ArrayDeque<>();
    List<Op> operations = new ArrayList<>();
    stack.push(new Visit(HaraBox.unwrap(document)));
    while (!stack.isEmpty()) {
      Task task = stack.pop();
      if (task instanceof Emit emit) {
        operations.add(emit.operation());
        continue;
      }
      Object value = HaraBox.unwrap(((Visit) task).value());
      if (value == null || value == HaraNull.SINGLETON) continue;
      if (value instanceof String string) {
        operations.add(new Text(string, string.codePointCount(0, string.length())));
        continue;
      }
      if (value instanceof Keyword keyword
          && (keyword.getName().equals("line"))
          && (keyword.getNamespace() == null || keyword.getNamespace().equals("document"))) {
        operations.add(new Line(" ", ""));
        continue;
      }
      List<Object> values = sequential(value);
      if (values == null) {
        throw new HaraException("Document expects strings or element vectors");
      }
      if (values.isEmpty()) continue;
      if (!(values.get(0) instanceof Keyword keyword)) {
        pushChildren(stack, values, 0);
        continue;
      }
      String tag =
          keyword.getNamespace() == null
              ? keyword.getName()
              : keyword.getNamespace() + "/" + keyword.getName();
      List<Object> body = values.subList(1, values.size());
      switch (tag) {
        case "text", "document/text" -> {
          String text = text(body.toArray(), "text");
          operations.add(new Text(text, text.codePointCount(0, text.length())));
        }
        case "pass", "document/pass" -> operations.add(new Pass(text(body.toArray(), "pass")));
        case "escaped", "document/escaped" -> {
          if (body.size() != 1) {
            throw new HaraException("std.native.Document/escaped expects one string");
          }
          operations.add(new Escaped(text(body.toArray(), "escaped")));
        }
        case "span", "document/span", "document/fragment" -> pushChildren(stack, body, 0);
        case "annotate", "document/annotate" -> {
          if (body.isEmpty()) {
            throw new HaraException("std.native.Document/annotate expects an annotation");
          }
          pushChildren(stack, body, 1);
        }
        case "line", "document/line" -> {
          if (body.size() > 2) {
            throw new HaraException(
                "std.native.Document/line expects optional inline and terminate text");
          }
          String inline = body.isEmpty() ? " " : text(new Object[] {body.get(0)}, "line");
          String terminate =
              body.size() < 2 ? "" : text(new Object[] {body.get(1)}, "line");
          operations.add(new Line(inline, terminate));
        }
        case "break", "document/break" -> {
          if (!body.isEmpty()) {
            throw new HaraException("std.native.Document/break expects no arguments");
          }
          operations.add(new Break());
        }
        case "group", "document/group" -> {
          stack.push(new Emit(new End()));
          pushChildren(stack, body, 0);
          stack.push(new Emit(new Begin(0)));
        }
        case "nest", "document/nest", "align", "document/align" -> {
          long fallback = tag.endsWith("nest") ? 2 : 0;
          long offset = fallback;
          int from = 0;
          if (!body.isEmpty() && body.get(0) instanceof Number number) {
            offset = number.longValue();
            from = 1;
          }
          stack.push(new Emit(new Outdent()));
          pushChildren(stack, body, from);
          stack.push(new Emit(tag.endsWith("nest") ? new Nest(offset) : new Align(offset)));
        }
        default ->
            throw new HaraException(
                "Document text renderer does not support element tag :" + tag);
      }
    }
    return operations;
  }

  private static int[] annotateGroups(List<Op> operations) {
    int right = 0;
    int[] rights = new int[operations.size()];
    Deque<Integer> groups = new ArrayDeque<>();
    for (int index = 0; index < operations.size(); index++) {
      Op operation = operations.get(index);
      if (operation instanceof Text text) right += text.width();
      else if (operation instanceof Escaped) right += 1;
      else if (operation instanceof Line line) {
        right += line.inline().codePointCount(0, line.inline().length());
      } else if (operation instanceof Begin) groups.push(index);
      else if (operation instanceof End) {
        if (groups.isEmpty()) throw new HaraException("Document contains an unmatched group end");
        operations.set(groups.pop(), new Begin(right));
      }
      rights[index] = right;
    }
    if (!groups.isEmpty()) throw new HaraException("Document contains an unmatched group begin");
    return rights;
  }

  private static Object render(Object[] values) {
    if (values.length < 1 || values.length > 2) {
      throw new HaraException(
          "std.native.Document/render expects a document and optional options map");
    }
    IMapType<?, ?> options;
    if (values.length == 1) options = hara.lang.data.Map.Standard.EMPTY;
    else if (values[1] instanceof IMapType<?, ?> map) options = map;
    else throw new HaraException("std.native.Document/render expects an options map");
    Object format = lookup(options, Keyword.create("format"));
    if (format != null
        && (!(format instanceof Keyword keyword)
            || !keyword.getName().equals("text")
            || keyword.getNamespace() != null)) {
      throw new HaraException("std.native.Document/render currently supports only :text");
    }
    Object widthValue = lookup(options, Keyword.create("width"));
    long width = widthValue == null ? 80 : widthValue instanceof Number n ? n.longValue() : -1;
    if (width < 0 || width > Integer.MAX_VALUE) {
      throw new HaraException(
          "std.native.Document/render width must be a non-negative integer");
    }
    List<Op> operations = serialize(values[0]);
    int[] rights = annotateGroups(operations);
    StringBuilder output = new StringBuilder();
    Deque<Long> tabs = new ArrayDeque<>();
    tabs.push(0L);
    int fits = 0;
    long length = width;
    long column = 0;
    for (int index = 0; index < operations.size(); index++) {
      Op operation = operations.get(index);
      long indent = tabs.peek();
      if (operation instanceof Text text) {
        if (column == 0 && indent > 0) {
          output.append(" ".repeat((int) indent));
          column += indent;
        }
        output.append(text.value());
        column += text.width();
      } else if (operation instanceof Escaped escaped) {
        if (column == 0 && indent > 0) {
          output.append(" ".repeat((int) indent));
          column += indent;
        }
        output.append(escaped.value());
        column += 1;
      } else if (operation instanceof Pass pass) output.append(pass.value());
      else if (operation instanceof Line line) {
        if (fits == 0) {
          output.append(line.terminate()).append('\n');
          column = 0;
          length = Math.max(0, (long) rights[index] + width - Math.max(0, indent));
        } else {
          output.append(line.inline());
          column += line.inline().codePointCount(0, line.inline().length());
        }
      } else if (operation instanceof Break) {
        output.append('\n');
        column = 0;
        length = Math.max(0, (long) rights[index] + width - Math.max(0, indent));
      } else if (operation instanceof Nest nest) tabs.push(indent + nest.offset());
      else if (operation instanceof Align align) tabs.push(column + align.offset());
      else if (operation instanceof Outdent) {
        if (tabs.size() == 1) throw new HaraException("Document contains an unmatched outdent");
        tabs.pop();
      } else if (operation instanceof Begin begin) {
        fits = fits > 0 ? fits + 1 : begin.end() <= length ? 1 : 0;
      } else if (operation instanceof End) fits = Math.max(0, fits - 1);
    }
    if (tabs.size() != 1) {
      throw new HaraException("Document contains an unmatched indentation scope");
    }
    return output.toString();
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  private static Object lookup(IMapType<?, ?> map, Object key) {
    return ((IMapType) map).lookup(key);
  }
}
