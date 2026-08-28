package hara.kernel.base;

import hara.lang.base.Ex;
import hara.lang.base.G;
import hara.lang.base.primitive.Array;
import hara.lang.base.primitive.Num;
import hara.kernel.builtin.BuiltinStruct;
import hara.lang.data.*;
import hara.lang.protocol.ILinearType;
import hara.lang.protocol.IMapType;
import hara.lang.protocol.ISetType;
import hara.lang.protocol.Constant;
import hara.lang.protocol.IMetadata;
import hara.lang.protocol.IObjType;

import java.math.BigInteger;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.function.BiFunction;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

import static hara.kernel.builtin.BuiltinBasic.keyword;
import static hara.kernel.builtin.BuiltinCollection.merge;
import static hara.kernel.builtin.BuiltinStruct.*;

public interface Parser {

  @SuppressWarnings("rawtypes")
  public class LispReader {

    static final Keyword EOF = Keyword.create(null, "eof");

    static BiFunction[] macros = new BiFunction[256];
    static BiFunction[] dispatchMacros = new BiFunction[256];
    static Pattern symbolPat = Pattern.compile("[:]?([\\D&&[^/]].*/)?(/|[\\D&&[^/]][^/]*)");
    static Pattern intPat =
        Pattern.compile(
            "([-+]?)(?:(0)|([1-9][0-9]*)|0[xX]([0-9A-Fa-f]+)|0([0-7]+)|([1-9][0-9]?)[rR]([0-9A-Za-z]+)|0[0-9]+)");
    static Pattern floatPat =
        Pattern.compile("[-+]?[0-9]+(?:\\.[0-9]*)?(?:[eE][-+]?[0-9]+)?");

    static {
      macros['"'] = new StringReader();
      macros[';'] = new CommentReader();
      macros['^'] = new MetaReader();
      macros['('] = new ListReader();
      macros[')'] = new UnmatchedDelimiterReader();
      macros['['] = new VectorReader();
      macros[']'] = new UnmatchedDelimiterReader();
      macros['{'] = new MapReader();
      macros['}'] = new UnmatchedDelimiterReader();
      macros['\\'] = new CharacterReader();
      macros['#'] = new DispatchReader();
      macros['\''] = new QuoteReader(); // Added QuoteReader
      macros['@'] = new DerefReader();
      macros['`'] = new SyntaxQuoteReader();
      macros['~'] = new UnquoteReader();

      dispatchMacros['#'] = new SymbolicValueReader();
      dispatchMacros['^'] = new MetaReader();
      dispatchMacros['"'] = new RegexReader();
      dispatchMacros['{'] = new SetReader();
      dispatchMacros['('] = new AnonymousFunctionReader();
      dispatchMacros[39] = new VarReader();
      dispatchMacros['<'] = new UnreadableReader();
      dispatchMacros['_'] = new DiscardReader();
    }

    public interface S {

      static boolean nonConstituent(int ch) {
        return ch == '@' || ch == '`' || ch == '~';
      }

      static boolean isWhitespace(int ch) {
        return Character.isWhitespace(ch) || ch == ',';
      }
    }

    public static Object readString(String s, Map opts) {
      Reader r = new Reader(s);

      return read(r, (opts == null) ? hashMap(new Object[] {}) : opts);
    }

    public static void unread(Reader r, int ch) {
      if (ch != -1) r.unreadChar((char) ch);
    }

    @SuppressWarnings("serial")
    public static class ReaderException extends RuntimeException {
      final int line;
      final int column;

      public ReaderException(int line, int column, Throwable cause) {
        super(cause);
        this.line = line;
        this.column = column;
      }

      public int line() {
        return line;
      }

      public int column() {
        return column;
      }
    }

    public static int readSingle(Reader r) {
      Character c = r.readChar();
      return (c == null) ? -1 : c;
    }

    @SuppressWarnings("unchecked")
    public static Object read(Reader r, Map opts) {
      if (opts == null) opts = hashMap(new Object[] {});
      return read(r, !opts.has(EOF), opts.lookup(EOF), false, opts);
    }

    @SuppressWarnings("unchecked")
    public static Object read(
        Reader r, boolean eofIsError, Object eofValue, boolean isRecursive, Map opts) {
      while (true) {
        Character leading = r.readChar();
        if (leading == null) break;
        if (!S.isWhitespace(leading)) {
          r.unreadChar(leading);
          break;
        }
      }
      int startLine = r.getLineNumber();
      int startColumn = r.getColumnNumber();
      Object value = readForm(r, eofIsError, eofValue, isRecursive, opts);
      return attachSourceMetadata(
          value, startLine, startColumn, r.getLineNumber(), r.getColumnNumber());
    }

    @SuppressWarnings("unchecked")
    private static Object readForm(
        Reader r, boolean eofIsError, Object eofValue, boolean isRecursive, Map opts) {

      if (opts == null) opts = hashMap(new Object[] {});

      try {
        for (; ; ) {
          int ch = readSingle(r);

          while (S.isWhitespace(ch)) ch = readSingle(r);

          if (ch == -1) {
            if (eofIsError) throw new Ex.Runtime("EOF while reading");
            return eofValue;
          }

          if (Character.isDigit(ch)) {
            Object n = readNumber(r, (char) ch);
            return n;
          }

          BiFunction macroFn = getMacro(ch);
          if (macroFn != null) {
            Object ret = macroFn.apply(r, opts);
            if (ret == r) continue;
            return ret;
          }

          if (ch == '+' || ch == '-') {
            int ch2 = readSingle(r);
            if (Character.isDigit(ch2)) {
              unread(r, ch2);
              Object n = readNumber(r, (char) ch);
              return n;
            }
            unread(r, ch2);
          }

          String token = readToken(r, (char) ch, true);
          return interpretToken(token);
        }
      } catch (Exception e) {
        if (isRecursive) throw Ex.Sneaky(e);
        throw new ReaderException(r.getLineNumber(), r.getColumnNumber(), e);
      }
    }

    @SuppressWarnings({"rawtypes", "unchecked"})
    private static Object attachSourceMetadata(
        Object value, int startLine, int startColumn, int endLine, int endColumn) {
      if (!(value instanceof ILinearType)
          && !(value instanceof IMapType)
          && !(value instanceof hara.lang.protocol.ISetType)) return value;
      IObjType object = (IObjType) value;
      IMapType span =
          hashMap(
              Array.objects(
                  keyword("line"),
                  startLine,
                  keyword("column"),
                  startColumn,
                  keyword("end-line"),
                  endLine,
                  keyword("end-column"),
                  endColumn));
      IMetadata existing = object.meta();
      if (existing instanceof IMapType) {
        span = (IMapType) merge((IMapType) existing, span);
      }
      return object.withMeta(span);
    }

    @SuppressWarnings("unchecked")
    public static ArrayList readDelimitedList(char delim, Reader r, boolean isRecursive, Map opts) {
      final int firstline = r.getLineNumber();

      ArrayList list = new ArrayList();

      for (; ; ) {
        int ch = readSingle(r);

        while (S.isWhitespace(ch)) ch = readSingle(r);

        int formLine = r.getLineNumber();
        int formColumn = Math.max(1, r.getColumnNumber() - 1);

        if (ch == -1) {
          if (firstline < 0) throw new Ex.Runtime("EOF while reading");
          else throw new Ex.Runtime("EOF while reading, starting at line " + firstline);
        }

        if (ch == delim) break;

        BiFunction macroFn = getMacro(ch);
        if (macroFn != null) {
          Object mret = macroFn.apply(r, opts);
          if (mret != r) {
            list.add(
                attachSourceMetadata(
                    mret, formLine, formColumn, r.getLineNumber(), r.getColumnNumber()));
          }
        } else {
          unread(r, ch);
          Object o = read(r, true, null, isRecursive, opts);
          if (o != r) list.add(o);
        }
      }
      return list;
    }

    private static String readToken(Reader r, char initch, boolean leadConstituent) {
      StringBuilder sb = new StringBuilder();
      if (leadConstituent && S.nonConstituent(initch))
        throw new Ex.Runtime("Invalid leading character: " + initch);

      sb.append(initch);

      for (; ; ) {
        int ch = readSingle(r);

        if (ch == -1 || S.isWhitespace(ch) || isTerminatingMacro(ch)) {
          unread(r, ch);
          return sb.toString();
        } else if (S.nonConstituent(ch))
          throw new Ex.Runtime("Invalid constituent character: " + (char) ch);
        sb.append((char) ch);
      }
    }

    private static Object readNumber(Reader r, char initch) {
      StringBuilder sb = new StringBuilder();
      sb.append(initch);

      for (; ; ) {
        int ch = readSingle(r);
        if (ch == -1 || S.isWhitespace(ch) || isMacro(ch)) {
          unread(r, ch);
          break;
        }
        sb.append((char) ch);
      }

      String s = sb.toString();
      if (s.matches("[-+]?\\d+/\\d+")) {
        throw new Ex.Unsupported("Ratios are not supported");
      }
      if (s.matches("[-+]?[0-9].*[NM]")) {
        throw new Ex.Unsupported(
            "Invalid number: legacy numeric suffixes N and M are not supported: " + s);
      }
      Object n = matchNumber(s);
      if (n == null) throw new NumberFormatException("Invalid number: " + s);
      return n;
    }

    private static int readUnicodeChar(String token, int offset, int length, int base) {
      if (token.length() != offset + length)
        throw new IllegalArgumentException("Invalid unicode character: \\" + token);
      int uc = 0;
      for (int i = offset; i < offset + length; ++i) {
        int d = Character.digit(token.charAt(i), base);
        if (d == -1) throw new IllegalArgumentException("Invalid digit: " + token.charAt(i));
        uc = uc * base + d;
      }
      return uc;
    }

    private static int readUnicodeChar(Reader r, int initch, int base, int length, boolean exact) {
      int uc = Character.digit(initch, base);
      if (uc == -1) throw new IllegalArgumentException("Invalid digit: " + (char) initch);
      int i = 1;
      for (; i < length; ++i) {
        int ch = readSingle(r);
        if (ch == -1 || S.isWhitespace(ch) || isMacro(ch)) {
          unread(r, ch);
          break;
        }
        int d = Character.digit(ch, base);
        if (d == -1) throw new IllegalArgumentException("Invalid digit: " + (char) ch);
        uc = uc * base + d;
      }
      if (i != length && exact)
        throw new IllegalArgumentException(
            "Invalid character length: " + i + ", should be: " + length);
      return uc;
    }

    private static Object interpretToken(String s) {
      if (s.equals("nil")) {
        return null;
      } else if (s.equals("true")) {
        return Constant.T;
      } else if (s.equals("false")) {
        return Constant.F;
      }

      Object ret = matchSymbol(s);
      if (ret != null) return ret;

      throw new Ex.Runtime("Invalid token: " + s);
    }

    private static Object matchSymbol(String s) {
      boolean isKeyword = s.charAt(0) == ':';
      if (s.equals(":/")) {
        throw new Ex.Unsupported("Keyword not allowed: \":/\"");
      }

      return isKeyword ? Keyword.create(s.substring(1)) : Symbol.create(s);
    }

    private static Number matchNumber(String s) {
      Matcher m = intPat.matcher(s);
      if (m.matches()) {
        if (m.group(2) != null) return Num.num(0);
        boolean negate = m.group(1).equals("-");
        String n;
        int radix;
        if ((n = m.group(3)) != null) radix = 10;
        else if ((n = m.group(4)) != null) radix = 16;
        else if ((n = m.group(5)) != null) radix = 8;
        else if ((n = m.group(7)) != null) radix = Integer.parseInt(m.group(6));
        else return null;
        BigInteger value = new BigInteger(n, radix);
        if (negate) value = value.negate();
        if (value.compareTo(BigInteger.valueOf(Long.MIN_VALUE)) >= 0
            && value.compareTo(BigInteger.valueOf(Long.MAX_VALUE)) <= 0) {
          return Num.num(value.longValue());
        }
        return null;
      }
      m = floatPat.matcher(s);
      if (m.matches() && (s.indexOf('.') >= 0 || s.indexOf('e') >= 0 || s.indexOf('E') >= 0)) {
        double value = Double.valueOf(s);
        if (!Double.isFinite(value)) throw new Ex.Runtime("non-finite number");
        return value;
      }
      return null;
    }

    private static BiFunction getMacro(int ch) {
      if (ch < macros.length) return macros[ch];
      return null;
    }

    private static boolean isMacro(int ch) {
      return (ch < macros.length && macros[ch] != null);
    }

    private static boolean isTerminatingMacro(int ch) {
      return (ch != '#' && ch != '\'' && isMacro(ch));
    }

    public static class UnreadableReader implements BiFunction<Reader, Map, Void> {
      @Override
      public Void apply(Reader reader, Map opts) {
        throw new Ex.Runtime("Unreadable form");
      }
    }

    public static class UnmatchedDelimiterReader implements BiFunction<Reader, Map, Void> {

      @SuppressWarnings("unchecked")
      @Override
      public Void apply(Reader reader, Map opts) {
        throw new Ex.Runtime("Unmatched delimiter: " + opts.lookup(Keyword.create("delimiter")));
      }
    }

    public static class QuoteReader implements BiFunction<Reader, Map, Object> {
      @Override
      public Object apply(Reader r, Map opts) {
        Object o = read(r, true, null, true, opts);
        return List.Standard.from(null, Symbol.create("quote"), o);
      }
    }

    public static class DerefReader implements BiFunction<Reader, Map, Object> {
      @Override
      public Object apply(Reader r, Map opts) {
        Object o = read(r, true, null, true, opts);
        return List.Standard.from(null, Symbol.create("deref"), o);
      }
    }

    /** Reader shorthand for a Var reference: #'foo -> (var foo). */
    public static class VarReader implements BiFunction<Reader, Map, Object> {
      @Override
      public Object apply(Reader r, Map opts) {
        Object o = read(r, true, null, true, opts);
        if (!(o instanceof Symbol)) {
          throw new Ex.Runtime("Var quote expects a symbol");
        }
        return List.Standard.from(null, Symbol.create("var"), o);
      }
    }

    public static class SyntaxQuoteReader implements BiFunction<Reader, Map, Object> {
      @Override
      public Object apply(Reader r, Map opts) {
        Object o = read(r, true, null, true, opts);
        return List.Standard.from(null, Symbol.create("syntax-quote"), o);
      }
    }

    public static class UnquoteReader implements BiFunction<Reader, Map, Object> {
      @Override
      public Object apply(Reader r, Map opts) {
        int ch = readSingle(r);
        if (ch == -1) throw new Ex.Runtime("EOF while reading character");
        if (ch == '@') {
          Object o = read(r, true, null, true, opts);
          return List.Standard.from(null, Symbol.create("unquote-splicing"), o);
        } else {
          unread(r, ch);
          Object o = read(r, true, null, true, opts);
          return List.Standard.from(null, Symbol.create("unquote"), o);
        }
      }
    }

    public static class CharacterReader implements BiFunction<Reader, Map, HaraCharacter> {

      @Override
      public HaraCharacter apply(Reader reader, Map opts) {
        Reader r = reader;
        int ch = readSingle(r);
        if (ch == -1) throw new Ex.Runtime("EOF while reading character");
        String token = readToken(r, (char) ch, false);
        if (token.codePointCount(0, token.length()) == 1)
          return HaraCharacter.of(token.codePointAt(0));
        else if (token.equals("newline")) return HaraCharacter.of('\n');
        else if (token.equals("space")) return HaraCharacter.of(' ');
        else if (token.equals("tab")) return HaraCharacter.of('\t');
        else if (token.equals("backspace")) return HaraCharacter.of('\b');
        else if (token.equals("formfeed")) return HaraCharacter.of('\f');
        else if (token.equals("return")) return HaraCharacter.of('\r');
        else if (token.startsWith("u")) {
          int length = token.length() - 1;
          if (length != 4 && length != 6)
            throw new Ex.Runtime("Invalid unicode character: \\" + token);
          return HaraCharacter.of(readUnicodeChar(token, 1, length, 16));
        } else if (token.startsWith("o")) {
          int len = token.length() - 1;
          if (len > 3) throw new Ex.Runtime("Invalid octal escape sequence length: " + len);
          int uc = readUnicodeChar(token, 1, len, 8);
          if (uc > 0377) throw new Ex.Runtime("Octal escape sequence must be in range [0, 377].");
          return HaraCharacter.of(uc);
        }
        throw new Ex.Runtime("Unsupported character: \\" + token);
      }
    }

    public static class StringReader implements BiFunction<Reader, Map, String> {
      @Override
      public String apply(Reader r, Map opts) {
        StringBuilder sb = new StringBuilder();

        for (int ch = readSingle(r); ch != '"'; ch = readSingle(r)) {
          if (ch == -1) throw new Ex.Runtime("EOF while reading string");
          if (ch == '\\') // escape
          {
            ch = readSingle(r);
            if (ch == -1) throw new Ex.Runtime("EOF while reading string");
            switch (ch) {
              case 't':
                ch = '\t';
                break;
              case 'r':
                ch = '\r';
                break;
              case 'n':
                ch = '\n';
                break;
              case '\\':
                break;
              case '"':
                break;
              case 'b':
                ch = '\b';
                break;
              case 'f':
                ch = '\f';
                break;
              case 'u':
                {
                  ch = readSingle(r);
                  if (Character.digit(ch, 16) == -1)
                    throw new Ex.Runtime("Invalid unicode escape: \\u" + (char) ch);
                  ch = readUnicodeChar(r, ch, 16, 4, true);
                  break;
                }
              default:
                {
                  if (Character.isDigit(ch)) {
                    ch = readUnicodeChar(r, ch, 8, 3, false);
                    if (ch > 0377)
                      throw new Ex.Runtime("Octal escape sequence must be in range [0, 377].");
                  } else throw new Ex.Runtime("Unsupported escape character: \\" + (char) ch);
                }
            }
          }
          sb.append((char) ch);
        }
        return sb.toString();
      }
    }

    public static class RegexReader implements BiFunction<Reader, Map, Pattern> {
      static java.io.StringReader stringrdr = new java.io.StringReader(""); // Unused?

      @Override
      public Pattern apply(Reader r, Map opts) {
        StringBuilder sb = new StringBuilder();
        for (int ch = readSingle(r); ch != '"'; ch = readSingle(r)) {
          if (ch == -1) throw new Ex.Runtime("EOF while reading regex");
          sb.append((char) ch);
          if (ch == '\\') // escape
          {
            ch = readSingle(r);
            if (ch == -1) throw new Ex.Runtime("EOF while reading regex");
            sb.append((char) ch);
          }
        }
        return Pattern.compile(sb.toString());
      }
    }

    public static class CommentReader implements BiFunction<Reader, Map, Reader> {
      @Override
      public Reader apply(Reader r, Map opts) {
        int ch;
        do {
          ch = readSingle(r);
        } while (ch != -1 && ch != '\n' && ch != '\r');
        return r;
      }
    }

    public static class DiscardReader implements BiFunction<Reader, Map, Reader> {
      @Override
      public Reader apply(Reader r, Map opts) {
        read(r, true, null, true, opts);
        return r;
      }
    }

    public static class DispatchReader implements BiFunction<Reader, Map, Object> {

      @SuppressWarnings("unchecked")
      @Override
      public Object apply(Reader r, Map opts) {
        int ch = readSingle(r);
        if (ch == -1) throw new Ex.Runtime("EOF while reading hash dispatch");
        BiFunction fn = dispatchMacros[ch];

        if (fn != null) return fn.apply(r, opts);
        if (!Character.isLetter(ch))
          throw new Ex.Runtime(String.format("No dispatch macro for: %c", (char) ch));

        String token = readToken(r, (char) ch, true);
        Object tag = matchSymbol(token);
        if (!(tag instanceof Symbol)) throw new Ex.Runtime("Invalid reader tag: " + token);
        Object form = read(r, true, null, true, opts);
        if ("ptr".equals(((Symbol) tag).display())) {
          try {
            return Pointer.fromDescriptor(form);
          } catch (IllegalArgumentException error) {
            throw new Ex.Runtime(error.getMessage());
          }
        }
        return new TaggedLiteral((Symbol) tag, form);
      }
    }

    public static class AnonymousFunctionReader implements BiFunction<Reader, Map, Object> {
      @Override
      public Object apply(Reader r, Map opts) {
        Object body = List.Standard.from(null, readDelimitedList(')', r, true, opts).toArray());
        AnonymousArguments arguments = new AnonymousArguments();
        collectAnonymousArguments(body, arguments);
        long id = r.nextAnonymousFunctionId();
        ArrayList<Object> parameters = new ArrayList<>();
        for (int index = 1; index <= arguments.maximum; index++) {
          parameters.add(anonymousArgument(id, index));
        }
        if (arguments.variadic) {
          parameters.add(Symbol.create("&"));
          parameters.add(anonymousRestArgument(id));
        }
        return List.Standard.from(
            null,
            Symbol.create("fn"),
            Vector.Standard.from(null, parameters.toArray()),
            rewriteAnonymousArguments(body, id));
      }
    }

    private static final class AnonymousArguments {
      int maximum;
      boolean variadic;
    }

    private static Symbol anonymousArgument(long id, int index) {
      return Symbol.create("__reader_fn_" + id + "_" + index);
    }

    private static Symbol anonymousRestArgument(long id) {
      return Symbol.create("__reader_fn_" + id + "_rest");
    }

    private static void collectAnonymousArguments(Object form, AnonymousArguments arguments) {
      if (form instanceof Symbol symbol) {
        String name = symbol.getName();
        if (symbol.getNamespace() != null || !name.startsWith("%")) return;
        if (name.equals("%") || name.equals("%1")) {
          arguments.maximum = Math.max(arguments.maximum, 1);
        } else if (name.equals("%&")) {
          arguments.variadic = true;
        } else {
          try {
            int index = Integer.parseInt(name.substring(1));
            if (index == 0) {
              throw new Ex.Runtime("Anonymous function arguments begin at %1");
            }
            arguments.maximum = Math.max(arguments.maximum, index);
          } catch (NumberFormatException error) {
            throw new Ex.Runtime("Invalid anonymous function argument: " + name);
          }
        }
      } else if (form instanceof IMapType<?, ?> map) {
        for (Object rawEntry : map) {
          java.util.Map.Entry<?, ?> entry = (java.util.Map.Entry<?, ?>) rawEntry;
          collectAnonymousArguments(entry.getKey(), arguments);
          collectAnonymousArguments(entry.getValue(), arguments);
        }
      } else if (form instanceof Iterable<?> values) {
        for (Object value : values) collectAnonymousArguments(value, arguments);
      } else if (form instanceof TaggedLiteral tagged) {
        collectAnonymousArguments(tagged.form(), arguments);
      }
    }

    private static Object rewriteAnonymousArguments(Object form, long id) {
      if (form instanceof Symbol symbol) {
        if (symbol.getNamespace() != null) return form;
        String name = symbol.getName();
        if (name.equals("%") || name.equals("%1")) return anonymousArgument(id, 1);
        if (name.equals("%&")) return anonymousRestArgument(id);
        if (name.startsWith("%")) {
          return anonymousArgument(id, Integer.parseInt(name.substring(1)));
        }
        return form;
      }
      if (form instanceof List<?> values) {
        ArrayList<Object> rewritten = rewriteAnonymousValues(values, id);
        return List.Standard.from(formMeta(form), rewritten.toArray());
      }
      if (form instanceof ILinearType<?> values && "[".equals(values.startString())) {
        ArrayList<Object> rewritten = rewriteAnonymousValues(values, id);
        return rewritten.size() <= 8
            ? ((IObjType) tuple(rewritten.toArray())).withMeta(formMeta(form))
            : Vector.Standard.from(formMeta(form), rewritten.toArray());
      }
      if (form instanceof IMapType<?, ?> map) {
        ArrayList<Object> entries = new ArrayList<>();
        for (Object rawEntry : map) {
          java.util.Map.Entry<?, ?> entry = (java.util.Map.Entry<?, ?>) rawEntry;
          entries.add(rewriteAnonymousArguments(entry.getKey(), id));
          entries.add(rewriteAnonymousArguments(entry.getValue(), id));
        }
        return form instanceof OrderedMap
            ? BuiltinStruct.orderedMap(entries)
            : BuiltinStruct.hashMap(entries);
      }
      if (form instanceof ISetType<?> values) {
        return BuiltinStruct.orderedSet(rewriteAnonymousValues(values, id));
      }
      if (form instanceof TaggedLiteral tagged) {
        return new TaggedLiteral(tagged.tag(), rewriteAnonymousArguments(tagged.form(), id));
      }
      return form;
    }

    private static ArrayList<Object> rewriteAnonymousValues(Iterable<?> values, long id) {
      ArrayList<Object> rewritten = new ArrayList<>();
      for (Object value : values) rewritten.add(rewriteAnonymousArguments(value, id));
      return rewritten;
    }

    private static IMetadata formMeta(Object form) {
      return form instanceof IObjType object ? object.meta() : null;
    }

    public static class SymbolicValueReader implements BiFunction<Reader, Map, Object> {
      @SuppressWarnings("unchecked")
      @Override
      public Object apply(Reader r, Map opts) {
        Object o = read(r, true, null, true, opts);

        if (!(o instanceof Symbol)) throw new Ex.Runtime("Invalid token: ##" + o);
        if (o.equals(Symbol.create(null, "Inf"))
            || o.equals(Symbol.create(null, "-Inf"))
            || o.equals(Symbol.create(null, "NaN"))) {
          throw new Ex.Runtime("non-finite number");
        }
        throw new Ex.Runtime("Unknown symbolic value: ##" + o);
      }
    }

    public static IMetadata getMeta(Reader r) {
      int line = r.getLineNumber();
      int column = r.getColumnNumber();
      return hashMap(Array.objects(keyword("line"), line, keyword("column"), column));
    }

    public static class ListReader implements BiFunction<Reader, Map, List> {
      @Override
      public List apply(Reader r, Map opts) {
        ArrayList list = readDelimitedList(')', r, true, opts);
        return (list.isEmpty()) ? List.Standard.EMPTY : list(list);
      }
    }

    public static class MetaReader implements BiFunction<Reader, Map, IObjType> {

      @Override
      public IObjType apply(Reader r, Map opts) {

        Object meta = read(r, true, null, true, opts);

        if (meta instanceof Symbol || meta instanceof String)
          meta = hashMap(Array.objects(keyword("tag"), meta));
        else if (meta instanceof Keyword) meta = hashMap(Array.objects(meta, Constant.T));
        else if (!(meta instanceof IMapType))
          throw new IllegalArgumentException("Metadata must be Symbol,Keyword,String or map");

        Object o = read(r, true, null, true, opts);
        if (o instanceof IObjType) {
          IMapType ometa = (IMapType) ((IObjType) o).meta();

          if (ometa == null) {
            ometa = (IMapType) meta;
          } else {
            ometa = (IMapType) merge(ometa, meta);
          }
          return ((IObjType) o).withMeta(ometa);
        } else throw new IllegalArgumentException("Metadata can only be applied to I.ObjTypes");
      }
    }

    public static class VectorReader implements BiFunction<Reader, Map, ILinearType> {
      @Override
      public ILinearType apply(Reader r, Map opts) {
        ArrayList list = readDelimitedList(']', r, true, opts);
        return list.size() <= 8 ? tuple(list.toArray()) : vector(list);
      }
    }

    public static class MapReader implements BiFunction<Reader, Map, hara.lang.data.Map> {
      @Override
      public hara.lang.data.Map apply(Reader r, Map opts) {
        ArrayList list = readDelimitedList('}', r, true, opts);

        if ((list.size() & 1) == 1) {
          throw new Ex.Runtime(" literal must contain an even number of forms");
        }

        HashSet<Object> keys = new HashSet<>();
        for (int i = 0; i < list.size(); i += 2) {
          Object key = list.get(i);
          if (keys.contains(key)) {
            throw new Ex.Runtime("Duplicate key: " + key);
          }
          keys.add(key);
        }

        return BuiltinStruct.hashMap(list);
      }
    }

    public static class SetReader implements BiFunction<Reader, Map, OrderedSet> {
      @Override
      public OrderedSet apply(Reader r, Map opts) {
        ArrayList list = readDelimitedList('}', r, true, opts);

        HashSet<Object> items = new HashSet<>();
        for (Object item : list) {
          if (items.contains(item)) {
            throw new Ex.Runtime("Duplicate item: " + item);
          }
          items.add(item);
        }

        return BuiltinStruct.orderedSet(list);
      }
    }
  }

  public static void main(String[] args) {
    G.prn(LispReader.readString("a/b", null));
  }
}
