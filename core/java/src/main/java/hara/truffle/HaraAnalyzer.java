package hara.truffle;

import com.oracle.truffle.api.frame.FrameDescriptor;
import com.oracle.truffle.api.frame.FrameSlotKind;
import com.oracle.truffle.api.source.SourceSection;
import hara.lang.data.Keyword;
import hara.lang.data.List;
import hara.lang.data.Symbol;
import hara.lang.data.TaggedLiteral;
import hara.lang.protocol.IMapType;
import hara.lang.protocol.ILinearType;
import hara.lang.protocol.ISetType;
import hara.lang.protocol.IObjType;
import hara.kernel.builtin.BuiltinStruct;
import hara.truffle.node.HaraExpressionNode;
import hara.truffle.node.HaraNodes;
import hara.truffle.node.HaraRootNode;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.Iterator;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.Map;
import java.util.Set;

final class HaraAnalyzer {
  private final HaraLanguage language;
  private final HaraContext context;
  private final SourceSection sourceSection;
  private final FrameDescriptor.Builder frames;
  private final Map<Symbol, Integer> locals;
  private final HaraAnalyzer parent;
  private final Map<Symbol, Integer> captureSlots;
  private final Map<Symbol, Integer> captureSources;
  private final HaraNodes.RecurTarget recurTarget;

  private HaraAnalyzer(
      HaraLanguage language,
      SourceSection sourceSection,
      HaraContext context,
      FrameDescriptor.Builder frames,
      Map<Symbol, Integer> locals,
      HaraAnalyzer parent,
      Map<Symbol, Integer> captureSlots,
      Map<Symbol, Integer> captureSources,
      HaraNodes.RecurTarget recurTarget) {
    this.language = language;
    this.context = context;
    this.sourceSection = sourceSection;
    this.frames = frames;
    this.locals = locals;
    this.parent = parent;
    this.captureSlots = captureSlots;
    this.captureSources = captureSources;
    this.recurTarget = recurTarget;
  }

  static com.oracle.truffle.api.RootCallTarget compile(
      HaraLanguage language, Object[] forms, SourceSection sourceSection, HaraContext context) {
    FrameDescriptor.Builder frames = FrameDescriptor.newBuilder();
    HaraAnalyzer analyzer =
        new HaraAnalyzer(
            language, sourceSection, context, frames, Map.of(), null, null, null, null);
    HaraExpressionNode[] expressions = new HaraExpressionNode[forms.length];
    int index = 0;
    while (index < forms.length) {
      if (analyzer.topLevelForm(forms[index], "ns")) {
        expressions[index] = analyzer.analyze(forms[index]);
        index++;
      }
      int end = index;
      while (end < forms.length && !analyzer.topLevelForm(forms[end], "ns")) end++;
      analyzer.predeclareTopLevel(forms, index, end);
      while (index < end) {
        expressions[index] = analyzer.analyze(forms[index]);
        index++;
      }
    }
    HaraExpressionNode body = new HaraNodes.Do(expressions);
    return new HaraRootNode(
            language,
            frames.build(),
            body,
            new int[0],
            new int[0],
            new int[0],
            sourceSection,
            true,
            false)
        .getCallTarget();
  }

  private boolean topLevelForm(Object form, String name) {
    return form instanceof List<?> list
        && list.count() > 0
        && list.nth(0) instanceof Symbol operator
        && operator.getNamespace() == null
        && name.equals(operator.getName());
  }

  private void predeclareTopLevel(Object[] forms, int start, int end) {
    Set<String> definitionForms = Set.of("def", "defn", "defn-", "defmacro");
    for (int index = start; index < end; index++) {
      if (!(forms[index] instanceof List<?> list)
          || list.count() < 2
          || !(list.nth(0) instanceof Symbol operator)
          || operator.getNamespace() != null
          || !definitionForms.contains(operator.getName())
          || !(list.nth(1) instanceof Symbol name)
          || name.getNamespace() != null) {
        continue;
      }
      context.declareCurrent(name);
    }
  }

  /** Creates a lexical sub-scope sharing the current function frame. */
  private HaraAnalyzer subScope(
      Map<Symbol, Integer> subLocals, HaraNodes.RecurTarget subRecurTarget) {
    return new HaraAnalyzer(
        language, sourceSection, context, frames, subLocals, this, null, null, subRecurTarget);
  }

  /** Creates the analyzer for a nested function with its own frame. */
  private HaraAnalyzer functionScope(
      FrameDescriptor.Builder functionFrames,
      Map<Symbol, Integer> functionLocals,
      Map<Symbol, Integer> functionCaptureSlots,
      Map<Symbol, Integer> functionCaptureSources) {
    return new HaraAnalyzer(
        language,
        sourceSection,
        context,
        functionFrames,
        functionLocals,
        this,
        functionCaptureSlots,
        functionCaptureSources,
        null);
  }

  /**
   * Resolves a symbol to a frame slot visible from this scope, allocating capture slots on
   * demand in every function frame crossed on the way out. Capture allocation is therefore
   * transitive: a nested function that references a grandparent binding forces its immediate
   * parent to capture that binding as well. Returns null when the symbol is not lexically
   * bound in any enclosing scope.
   */
  private Integer lookupLexical(Symbol symbol) {
    Integer slot = locals.get(symbol);
    if (slot != null) {
      return slot;
    }
    if (parent == null) {
      return null;
    }
    Integer source = parent.lookupLexical(symbol);
    if (source == null) {
      return null;
    }
    if (frames != parent.frames) {
      slot = frames.addSlot(FrameSlotKind.Object, symbol, null);
      captureSlots.put(symbol, slot);
      captureSources.put(symbol, source);
      locals.put(symbol, slot);
      return slot;
    }
    return source;
  }

  /** Whether the symbol is bound in any visible lexical scope, without allocating captures. */
  private boolean isLexicallyBound(Symbol symbol) {
    for (HaraAnalyzer scope = this; scope != null; scope = scope.parent) {
      if (scope.locals.containsKey(symbol)) {
        return true;
      }
    }
    return false;
  }

  private HaraExpressionNode analyze(Object form) {
    try {
      HaraExpressionNode node = analyzeForm(form);
      attachSourceSection(node, form);
      return node;
    } catch (HaraException error) {
      if (error.haraLocation() != null || !(form instanceof IObjType)) {
        throw error;
      }
      HaraExpressionNode location = new HaraNodes.Literal(null);
      attachSourceSection(location, form);
      throw new HaraException(error.getMessage(), location);
    }
  }

  private HaraExpressionNode analyzeForm(Object form) {
    Object expanded = expandMacro(form);
    if (expanded != form) {
      return analyze(expanded);
    }
    if (form instanceof Symbol) {
      Symbol symbol = (Symbol) form;
      Integer slot = lookupLexical(symbol);
      if (slot == null) {
        return new HaraNodes.ReadGlobal(context.canonicalSymbol(symbol), symbol);
      }
      return new HaraNodes.ReadLocal(slot);
    }
    if (!(form instanceof List<?>)) {
      HaraExpressionNode collection = analyzeCollectionLiteral(form);
      if (collection != null) {
        return collection;
      }
      return new HaraNodes.Literal(form);
    }
    List<?> list = (List<?>) form;
    if (list.count() == 0) {
      return new HaraNodes.Literal(list);
    }

    Object operator = list.nth(0);
    if (operator instanceof Symbol) {
      Symbol symbolOperator = (Symbol) operator;
      if (symbolOperator.getNamespace() != null) {
        return analyzeInvocation(list);
      }
      String name = symbolOperator.getName();
      switch (name) {
        case "quote":
          return analyzeQuote(list);
        case "syntax-quote":
          return analyzeSyntaxQuote(list);
        case "do":
          return analyzeDo(list, 1);
        case "comment":
          return new HaraNodes.Literal(null);
        case "if":
          return analyzeIf(list);
        case "when":
          return analyzeWhen(list, false);
        case "when-not":
          return analyzeWhen(list, true);
        case "cond":
          return analyzeCond(list);
        case "and":
          return analyzeAnd(list);
        case "or":
          return analyzeOr(list);
        case "let":
          return analyzeLet(list);
        case "letfn":
          return analyzeLetFn(list);
        case "binding":
          return analyzeBinding(list);
        case "loop":
          return analyzeLoop(list);
        case "recur":
          return analyzeRecur(list);
        case "throw":
          return analyzeThrow(list);
        case "try":
          return analyzeTry(list);
        case "fn":
          return analyzeFunction(list);
        case "defn":
          return analyzeDefn(list);
        case "defn-":
          return analyzeDefn(list, true);
        case "declare":
          return analyzeDeclare(list);
        case "defmulti":
          return analyzeDefMulti(list);
        case "defmethod":
          return analyzeDefMethod(list);
        case "def":
          return analyzeDef(list);
        case "var":
          return analyzeVar(list);
        case "set!":
          return analyzeSetVar(list);
        case "defstruct":
          return analyzeNamedDefinition(list, false);
        case "defmutable":
          return analyzeNamedDefinition(list, true);
        case "defprotocol":
          return analyzeDefProtocol(list);
        case "extend-type":
          return analyzeExtendType(list);
        case "defmacro":
          return analyzeDefMacro(list);
        case "new":
          return analyzeNativeNew(list);
        case "ns":
          return analyzeNamespace(list);
        case "ns+":
          return analyzeAnonymousNamespace(list);
        case "require":
          return analyzeRequire(list);
        case "alias":
          return analyzeAlias(list);
        case "field":
          return analyzeField(list);
        case ".":
          return analyzeMarkerCall(list);
        default:
          return analyzeInvocation(list);
      }
    }
    return analyzeInvocation(list);
  }

  @SuppressWarnings("rawtypes")
  private void attachSourceSection(HaraExpressionNode node, Object form) {
    if (sourceSection == null) return;
    node.setHaraSourceSection(sourceSection);
    if (!(form instanceof IObjType)) return;
    Object metadata = ((IObjType) form).meta();
    if (!(metadata instanceof IMapType)) return;
    IMapType span = (IMapType) metadata;
    Object lineValue = span.lookup(Keyword.create("line"));
    Object columnValue = span.lookup(Keyword.create("column"));
    Object endLineValue = span.lookup(Keyword.create("end-line"));
    Object endColumnValue = span.lookup(Keyword.create("end-column"));
    if (!(lineValue instanceof Number)
        || !(columnValue instanceof Number)
        || !(endLineValue instanceof Number)
        || !(endColumnValue instanceof Number)) return;
    try {
      int line = ((Number) lineValue).intValue();
      int column = ((Number) columnValue).intValue();
      int endLine = ((Number) endLineValue).intValue();
      int endColumn = ((Number) endColumnValue).intValue();
      int start = sourceSection.getSource().getLineStartOffset(line) + column - 1;
      int end = sourceSection.getSource().getLineStartOffset(endLine) + endColumn - 1;
      node.setHaraSourceSection(
          sourceSection.getSource().createSection(start, Math.max(1, end - start)));
    } catch (RuntimeException ignored) {
      // Keep the whole-source section as the safe fallback for malformed metadata.
    }
  }

  private HaraExpressionNode analyzeCollectionLiteral(Object form) {
    if (form instanceof IMapType<?, ?>) {
      IMapType<?, ?> map = (IMapType<?, ?>) form;
      ArrayList<HaraExpressionNode> elements = new ArrayList<>();
      Iterator<?> iterator = map.iterator();
      while (iterator.hasNext()) {
        Object entry = iterator.next();
        if (!(entry instanceof java.util.Map.Entry<?, ?>)) {
          throw error("map literal contains a non-entry value");
        }
        java.util.Map.Entry<?, ?> pair = (java.util.Map.Entry<?, ?>) entry;
        elements.add(analyze(pair.getKey()));
        elements.add(analyze(pair.getValue()));
      }
      HaraNodes.CollectionLiteral.Kind kind = HaraNodes.CollectionLiteral.Kind.MAP;
      if (form instanceof hara.lang.data.OrderedMap) {
        kind = HaraNodes.CollectionLiteral.Kind.ORDERED_MAP;
      } else if (form instanceof hara.lang.data.SortedMap) {
        kind = HaraNodes.CollectionLiteral.Kind.SORTED_MAP;
      }
      return new HaraNodes.CollectionLiteral(kind, elements.toArray(new HaraExpressionNode[0]));
    }
    if (form instanceof ISetType<?>) {
      ISetType<?> set = (ISetType<?>) form;
      ArrayList<HaraExpressionNode> elements = new ArrayList<>();
      Iterator<?> iterator = set.iterator();
      while (iterator.hasNext()) {
        elements.add(analyze(iterator.next()));
      }
      HaraNodes.CollectionLiteral.Kind kind = HaraNodes.CollectionLiteral.Kind.SET;
      if (form instanceof hara.lang.data.OrderedSet) {
        kind = HaraNodes.CollectionLiteral.Kind.ORDERED_SET;
      } else if (form instanceof hara.lang.data.SortedSet) {
        kind = HaraNodes.CollectionLiteral.Kind.SORTED_SET;
      }
      return new HaraNodes.CollectionLiteral(kind, elements.toArray(new HaraExpressionNode[0]));
    }
    if (form instanceof ILinearType<?>) {
      ILinearType<?> linear = (ILinearType<?>) form;
      HaraNodes.CollectionLiteral.Kind kind =
          form instanceof hara.lang.data.Tuple.Tup0
                  || form instanceof hara.lang.data.Tuple.Tup1<?>
              ? HaraNodes.CollectionLiteral.Kind.TUPLE
              : form instanceof hara.lang.data.Queue
                  ? HaraNodes.CollectionLiteral.Kind.QUEUE
                  : HaraNodes.CollectionLiteral.Kind.VECTOR;
      HaraExpressionNode[] elements = new HaraExpressionNode[(int) linear.count()];
      for (int i = 0; i < elements.length; i++) {
        elements[i] = analyze(linear.nth(i));
      }
      return new HaraNodes.CollectionLiteral(kind, elements);
    }
    return null;
  }

  private HaraExpressionNode analyzeQuote(List<?> form) {
    requireCount(form, 2, "quote");
    return new HaraNodes.Literal(form.nth(1));
  }

  private HaraExpressionNode analyzeSyntaxQuote(List<?> form) {
    requireCount(form, 2, "syntax-quote");
    ArrayList<HaraExpressionNode> unquotes = new ArrayList<>();
    Object template =
        syntaxQuoteTemplate(form.nth(1), unquotes, new LinkedHashMap<>());
    if (template instanceof HaraNodes.SyntaxQuote.Unquote) {
      throw error("unquote-splicing is only valid inside a collection");
    }
    return new HaraNodes.SyntaxQuote(
        template, unquotes.toArray(new HaraExpressionNode[0]));
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  private Object syntaxQuoteTemplate(
      Object value,
      ArrayList<HaraExpressionNode> unquotes,
      Map<Symbol, Integer> autoGensyms) {
    if (value instanceof Symbol symbol) {
      if (symbol.getNamespace() == null && symbol.getName().endsWith("#")) {
        int index =
            autoGensyms.computeIfAbsent(symbol, ignored -> autoGensyms.size());
        String prefix = symbol.getName().substring(0, symbol.getName().length() - 1);
        return new HaraNodes.SyntaxQuote.AutoGensym(index, prefix);
      }
      return value;
    }
    if (value instanceof List<?> list) {
      if (syntaxForm(list, "unquote") || syntaxForm(list, "unquote-splicing")) {
        requireCount(list, 2, ((Symbol) list.nth(0)).getName());
        int index = unquotes.size();
        unquotes.add(analyze(list.nth(1)));
        return new HaraNodes.SyntaxQuote.Unquote(
            index, syntaxForm(list, "unquote-splicing"));
      }
      ArrayList<Object> output = new ArrayList<>();
      for (Object item : list) {
        output.add(syntaxQuoteTemplate(item, unquotes, autoGensyms));
      }
      return hara.lang.data.List.Standard.from(list.meta(), output.toArray());
    }
    if (value instanceof ILinearType<?> vector
        && !(value instanceof hara.lang.data.List)
        && "[".equals(vector.startString())) {
      ArrayList<Object> output = new ArrayList<>();
      for (Object item : vector) {
        output.add(syntaxQuoteTemplate(item, unquotes, autoGensyms));
      }
      Object sequence =
          output.size() <= 8
              ? hara.kernel.builtin.BuiltinStruct.tuple(output.toArray())
              : hara.lang.data.Vector.Standard.from(null, output.toArray());
      return ((hara.lang.protocol.IObjType) sequence)
          .withMeta(((hara.lang.protocol.IObjType) vector).meta());
    }
    if (value instanceof ISetType<?> set) {
      ArrayList<Object> output = new ArrayList<>();
      for (Object item : set) {
        output.add(syntaxQuoteTemplate(item, unquotes, autoGensyms));
      }
      return hara.lang.data.Set.Standard.from(set.meta(), output.toArray());
    }
    if (value instanceof IMapType<?, ?> map) {
      ArrayList<Object> output = new ArrayList<>();
      for (Object entryValue : map) {
        java.util.Map.Entry<?, ?> entry = (java.util.Map.Entry<?, ?>) entryValue;
        output.add(syntaxQuoteTemplate(entry.getKey(), unquotes, autoGensyms));
        output.add(syntaxQuoteTemplate(entry.getValue(), unquotes, autoGensyms));
      }
      if (value instanceof hara.lang.data.OrderedMap) {
        return hara.lang.data.OrderedMap.Standard.from(map.meta(), output.toArray());
      }
      return hara.lang.data.Map.Standard.from(map.meta(), output.toArray());
    }
    return value;
  }

  private boolean syntaxForm(List<?> form, String name) {
    return form.count() > 0
        && form.nth(0) instanceof Symbol symbol
        && symbol.getNamespace() == null
        && name.equals(symbol.getName());
  }

  private HaraExpressionNode analyzeDo(List<?> form, int start) {
    Object[] forms = new Object[(int) form.count() - start];
    for (int i = start; i < form.count(); i++) {
      forms[i - start] = form.nth(i);
    }
    return analyzeForms(forms);
  }

  private HaraExpressionNode analyzeForms(Object[] forms) {
    HaraExpressionNode[] expressions = new HaraExpressionNode[forms.length];
    for (int i = 0; i < forms.length; i++) {
      expressions[i] = analyze(forms[i]);
    }
    return new HaraNodes.Do(expressions);
  }

  private HaraExpressionNode analyzeIf(List<?> form) {
    if (form.count() != 3 && form.count() != 4) {
      throw error("if expects two or three arguments");
    }
    HaraExpressionNode alternative =
        form.count() == 4 ? analyze(form.nth(3)) : new HaraNodes.Literal(null);
    return new HaraNodes.If(analyze(form.nth(1)), analyze(form.nth(2)), alternative);
  }

  private HaraExpressionNode analyzeWhen(List<?> form, boolean negate) {
    if (form.count() < 2) throw error((negate ? "when-not" : "when") + " expects a condition");
    Object[] body = new Object[(int) form.count() - 2];
    for (int i = 2; i < form.count(); i++) body[i - 2] = form.nth(i);
    HaraExpressionNode consequent =
        body.length == 0 ? new HaraNodes.Literal(null) : analyzeForms(body);
    HaraExpressionNode condition = analyze(form.nth(1));
    if (negate) return new HaraNodes.If(condition, new HaraNodes.Literal(null), consequent);
    return new HaraNodes.If(condition, consequent, new HaraNodes.Literal(null));
  }

  private HaraExpressionNode analyzeCond(List<?> form) {
    if (form.count() == 1) return new HaraNodes.Literal(null);
    if (form.count() % 2 == 0) throw error("cond expects test/expression pairs");
    HaraExpressionNode result = new HaraNodes.Literal(null);
    for (int i = (int) form.count() - 2; i >= 1; i -= 2) {
      Object test = form.nth(i);
      HaraExpressionNode condition = analyze(test);
      HaraExpressionNode consequent = analyze(form.nth(i + 1));
      result = new HaraNodes.If(condition, consequent, result);
    }
    return result;
  }

  private HaraExpressionNode analyzeAnd(List<?> form) {
    if (form.count() == 1) return new HaraNodes.Literal(true);
    HaraExpressionNode[] expressions = new HaraExpressionNode[(int) form.count() - 1];
    for (int i = 1; i < form.count(); i++) expressions[i - 1] = analyze(form.nth(i));
    return new HaraNodes.ShortCircuit(true, expressions);
  }

  private HaraExpressionNode analyzeOr(List<?> form) {
    if (form.count() == 1) return new HaraNodes.Literal(null);
    HaraExpressionNode[] expressions = new HaraExpressionNode[(int) form.count() - 1];
    for (int i = 1; i < form.count(); i++) expressions[i - 1] = analyze(form.nth(i));
    return new HaraNodes.ShortCircuit(false, expressions);
  }

  private HaraExpressionNode analyzeLet(List<?> form) {
    if (form.count() < 3 || !isBindingVector(form.nth(1))) {
      throw error("let expects a binding vector and a body");
    }
    ILinearType<?> bindings = (ILinearType<?>) form.nth(1);
    if (bindings.count() % 2 != 0) {
      throw error("let expects an even number of binding forms");
    }

    int bindingCount = (int) bindings.count() / 2;
    int[] rawSlots = new int[bindingCount];
    HaraExpressionNode[] initializers = new HaraExpressionNode[bindingCount];
    ArrayList<int[]> patternSlots = new ArrayList<>();
    ArrayList<HaraExpressionNode[]> patternInitializers = new ArrayList<>();
    Map<Symbol, Integer> bodyLocals = new HashMap<>(locals);
    for (int i = 0; i < bindingCount; i++) {
      Object pattern = bindings.nth(i * 2L);
      HaraAnalyzer initializerAnalyzer = subScope(bodyLocals, recurTarget);
      initializers[i] = initializerAnalyzer.analyze(bindings.nth(i * 2L + 1));
      rawSlots[i] =
          frames.addSlot(FrameSlotKind.Object, Symbol.create(null, "__hara_let_" + i), null);
      ArrayList<Integer> slots = new ArrayList<>();
      ArrayList<HaraExpressionNode> values = new ArrayList<>();
      Map<Symbol, Integer> introducedLocals = new HashMap<>();
      addPatternBindings(
          pattern, new HaraNodes.ReadLocal(rawSlots[i]), frames, introducedLocals, slots, values);
      bodyLocals.putAll(introducedLocals);
      patternSlots.add(slots.stream().mapToInt(Integer::intValue).toArray());
      patternInitializers.add(values.toArray(new HaraExpressionNode[0]));
    }

    HaraAnalyzer bodyAnalyzer = subScope(bodyLocals, recurTarget);
    HaraExpressionNode body = bodyAnalyzer.analyzeDo(form, 2);
    for (int i = bindingCount - 1; i >= 0; i--) {
      body = new HaraNodes.Let(patternSlots.get(i), patternInitializers.get(i), body);
      body =
          new HaraNodes.Let(
              new int[] {rawSlots[i]}, new HaraExpressionNode[] {initializers[i]}, body);
    }
    return body;
  }

  private HaraExpressionNode analyzeLetFn(List<?> form) {
    if (form.count() < 3 || !isBindingVector(form.nth(1))) {
      throw error("letfn expects a function binding vector and a body");
    }
    ILinearType<?> bindings = (ILinearType<?>) form.nth(1);
    int functionCount = (int) bindings.count();
    int[] slots = new int[functionCount];
    Map<Symbol, Integer> functionLocals = new HashMap<>(locals);
    ArrayList<Object[]> definitions = new ArrayList<>();
    for (int i = 0; i < functionCount; i++) {
      Object definition = bindings.nth(i);
      if (!(definition instanceof List<?>) || ((List<?>) definition).count() < 3) {
        throw error("letfn definitions must be (name [arguments] body...)");
      }
      List<?> definitionList = (List<?>) definition;
      Object name = definitionList.nth(0);
      if (!(name instanceof Symbol) || ((Symbol) name).getNamespace() != null) {
        throw error("letfn names must be unqualified symbols");
      }
      Symbol symbol = (Symbol) name;
      if (functionLocals.containsKey(symbol)) {
        throw error("Duplicate letfn name: " + symbol.getName());
      }
      slots[i] = frames.addSlot(FrameSlotKind.Object, symbol, null);
      functionLocals.put(symbol, slots[i]);
      Object[] body = new Object[(int) definitionList.count() - 2];
      for (int j = 2; j < definitionList.count(); j++) body[j - 2] = definitionList.nth(j);
      definitions.add(new Object[] {definitionList.nth(1), body});
    }

    HaraAnalyzer letFnAnalyzer = subScope(functionLocals, recurTarget);
    HaraExpressionNode[] functions = new HaraExpressionNode[functionCount];
    for (int i = 0; i < functionCount; i++) {
      Object[] definition = definitions.get(i);
      if (!isBindingVector(definition[0])) {
        throw error("letfn parameters must be a binding vector");
      }
      functions[i] =
          letFnAnalyzer.analyzeFunction((ILinearType<?>) definition[0], (Object[]) definition[1]);
    }
    HaraExpressionNode body = letFnAnalyzer.analyzeDo(form, 2);
    return new HaraNodes.LetFn(slots, functions, body);
  }

  private HaraExpressionNode analyzeBinding(List<?> form) {
    if (form.count() < 3 || !isBindingVector(form.nth(1))) {
      throw error("binding expects a binding vector and a body");
    }
    ILinearType<?> bindings = (ILinearType<?>) form.nth(1);
    if (bindings.count() % 2 != 0) {
      throw error("binding expects an even number of binding forms");
    }
    int count = (int) bindings.count() / 2;
    Symbol[] symbols = new Symbol[count];
    HaraExpressionNode[] initializers = new HaraExpressionNode[count];
    for (int i = 0; i < count; i++) {
      Object name = bindings.nth(i * 2L);
      if (!(name instanceof Symbol)) {
        throw error("binding names must be symbols");
      }
      // Dynamic bindings execute after analysis and may run while a caller's
      // namespace is current. Capture the defining Var now so unqualified
      // symbols and namespace aliases do not resolve against that caller.
      symbols[i] = context.canonicalSymbol((Symbol) name);
      initializers[i] = analyze(bindings.nth(i * 2L + 1));
    }
    return new HaraNodes.Binding(symbols, initializers, analyzeDo(form, 2));
  }

  private HaraExpressionNode analyzeFunction(List<?> form) {
    if (form.count() >= 2 && isBindingVector(form.nth(1))) {
      if (form.count() < 3) {
        throw error("fn expects a parameter vector and a body");
      }
      Object[] bodyForms = new Object[(int) form.count() - 2];
      for (int i = 2; i < form.count(); i++) {
        bodyForms[i - 2] = form.nth(i);
      }
      return analyzeFunction((ILinearType<?>) form.nth(1), bodyForms);
    }
    if (form.count() < 2) {
      throw error("fn expects a parameter vector and a body");
    }
    HaraExpressionNode[] alternatives = new HaraExpressionNode[(int) form.count() - 1];
    for (int i = 1; i < form.count(); i++) {
      Object clause = form.nth(i);
      if (!(clause instanceof List<?>) || ((List<?>) clause).count() < 2) {
        throw error("multi-arity fn clauses must contain a parameter vector and body");
      }
      List<?> clauseList = (List<?>) clause;
      if (!isBindingVector(clauseList.nth(0))) {
        throw error("multi-arity fn clause expects a parameter vector");
      }
      Object[] bodyForms = new Object[(int) clauseList.count() - 1];
      for (int j = 1; j < clauseList.count(); j++) {
        bodyForms[j - 1] = clauseList.nth(j);
      }
      alternatives[i - 1] = analyzeFunction((ILinearType<?>) clauseList.nth(0), bodyForms);
    }
    return new HaraNodes.MultiFunction(alternatives);
  }

  private HaraExpressionNode analyzeFunction(ILinearType<?> parameters, Object[] bodyForms) {
    FrameDescriptor.Builder functionFrames = FrameDescriptor.newBuilder();
    Map<Symbol, Integer> functionLocals = new HashMap<>();
    int restIndex = -1;
    for (int i = 0; i < parameters.count(); i++) {
      Object parameter = parameters.nth(i);
      if (parameter instanceof Symbol && "&".equals(((Symbol) parameter).getName())) {
        if (restIndex >= 0 || i + 2 != parameters.count()) {
          throw error("& must appear once before the final variadic binding");
        }
        restIndex = i;
      }
    }
    boolean variadic = restIndex >= 0;
    int fixedArity = variadic ? restIndex : (int) parameters.count();
    int[] parameterSlots = new int[fixedArity + (variadic ? 1 : 0)];
    ArrayList<Integer> destructureSlots = new ArrayList<>();
    ArrayList<HaraExpressionNode> destructureInitializers = new ArrayList<>();
    for (int i = 0; i < fixedArity; i++) {
      Object parameter = parameters.nth(i);
      Symbol rawSymbol =
          parameter instanceof Symbol ? (Symbol) parameter : Symbol.create(null, "__hara_arg_" + i);
      parameterSlots[i] = functionFrames.addSlot(FrameSlotKind.Object, rawSymbol, null);
      functionLocals.put(rawSymbol, parameterSlots[i]);
      if (!(parameter instanceof Symbol)) {
        addPatternBindings(
            parameter,
            new HaraNodes.ReadLocal(parameterSlots[i]),
            functionFrames,
            functionLocals,
            destructureSlots,
            destructureInitializers);
      }
    }
    if (variadic) {
      Object restParameter = parameters.nth(restIndex + 1);
      Symbol rawSymbol =
          restParameter instanceof Symbol
              ? (Symbol) restParameter
              : Symbol.create(null, "__hara_rest");
      parameterSlots[fixedArity] = functionFrames.addSlot(FrameSlotKind.Object, rawSymbol, null);
      functionLocals.put(rawSymbol, parameterSlots[fixedArity]);
      if (!(restParameter instanceof Symbol)) {
        addPatternBindings(
            restParameter,
            new HaraNodes.ReadLocal(parameterSlots[fixedArity]),
            functionFrames,
            functionLocals,
            destructureSlots,
            destructureInitializers);
      }
    }

    Map<Symbol, Integer> captureSlots = new LinkedHashMap<>();
    Map<Symbol, Integer> captureSources = new LinkedHashMap<>();
    HaraAnalyzer functionAnalyzer =
        functionScope(functionFrames, functionLocals, captureSlots, captureSources);
    HaraExpressionNode body = functionAnalyzer.analyzeForms(bodyForms);
    if (!destructureSlots.isEmpty()) {
      body =
          new HaraNodes.Let(
              destructureSlots.stream().mapToInt(Integer::intValue).toArray(),
              destructureInitializers.toArray(new HaraExpressionNode[0]),
              body);
    }
    int[] capturedSlots = captureSlots.values().stream().mapToInt(Integer::intValue).toArray();
    int[] capturedSources = captureSources.values().stream().mapToInt(Integer::intValue).toArray();
    HaraRootNode root =
        new HaraRootNode(
            language,
            functionFrames.build(),
            body,
            parameterSlots,
            capturedSlots,
            capturedSources,
            sourceSection,
            false,
            variadic);
    return new HaraNodes.FunctionLiteral(
        root.getCallTarget(), fixedArity, variadic, capturedSlots.length != 0);
  }

  private HaraExpressionNode analyzeDefn(List<?> form) {
    return analyzeDefn(form, false);
  }

  private HaraExpressionNode analyzeDefn(List<?> form, boolean privateDefinition) {
    if (form.count() < 4) {
      throw error("defn expects a name, parameter vector, and body");
    }

    Object name = form.nth(1);
    if (!(name instanceof Symbol)) {
      throw error("defn name must be a symbol");
    }
    Symbol symbol = definitionSymbol((Symbol) name, form);
    if (symbol.getNamespace() != null) {
      throw error("defn name must not be qualified");
    }
    context.declareCurrent(symbol);

    int parametersIndex = 2;
    String docstring = null;
    if (form.nth(parametersIndex) instanceof String) {
      docstring = (String) form.nth(parametersIndex);
      parametersIndex++;
    }
    IMapType<?, ?> attributes = null;
    if (parametersIndex < form.count() && form.nth(parametersIndex) instanceof IMapType<?, ?>) {
      attributes = (IMapType<?, ?>) form.nth(parametersIndex);
      parametersIndex++;
    }
    if (parametersIndex >= form.count()) {
      throw error("defn expects a body");
    }

    Object parameterForm = form.nth(parametersIndex);
    if (!isBindingVector(parameterForm)
        && (!(parameterForm instanceof List<?>) || ((List<?>) parameterForm).count() < 2)) {
      throw error("defn expects a parameter vector or arity clauses");
    }
    if (isBindingVector(parameterForm) && parametersIndex + 1 >= form.count()) {
      throw error("defn expects a body");
    }

    Object[] bodyForms = new Object[(int) form.count() - parametersIndex - 1];
    for (int i = parametersIndex + 1; i < form.count(); i++) {
      bodyForms[i - parametersIndex - 1] = form.nth(i);
    }
    HaraExpressionNode function;
    if (isBindingVector(parameterForm)) {
      function = analyzeFunction((ILinearType<?>) parameterForm, bodyForms);
    } else {
      List<?> clauses = form;
      HaraExpressionNode[] alternatives =
          new HaraExpressionNode[(int) form.count() - parametersIndex];
      for (int i = parametersIndex; i < form.count(); i++) {
        Object clause = form.nth(i);
        if (!(clause instanceof List<?>) || ((List<?>) clause).count() < 2) {
          throw error("defn arity clauses must contain a parameter vector and body");
        }
        List<?> clauseList = (List<?>) clause;
        if (!isBindingVector(clauseList.nth(0))) {
          throw error("defn arity clause expects a parameter vector");
        }
        Object[] clauseBody = new Object[(int) clauseList.count() - 1];
        for (int j = 1; j < clauseList.count(); j++) {
          clauseBody[j - 1] = clauseList.nth(j);
        }
        alternatives[i - parametersIndex] =
            analyzeFunction((ILinearType<?>) clauseList.nth(0), clauseBody);
      }
      function = new HaraNodes.MultiFunction(alternatives);
    }
    Object[] signatures;
    if (isBindingVector(parameterForm)) {
      signatures = new Object[] {parameterForm};
    } else {
      signatures = new Object[(int) form.count() - parametersIndex];
      for (int i = parametersIndex; i < form.count(); i++) {
        signatures[i - parametersIndex] = ((List<?>) form.nth(i)).nth(0);
      }
    }
    IMapType<Object, Object> definitionMetadata;
    if (symbol.meta() instanceof IMapType<?, ?>) {
      definitionMetadata = (IMapType<Object, Object>) symbol.meta();
    } else {
      definitionMetadata = hara.lang.data.Map.Standard.EMPTY;
    }
    if (attributes != null) {
      for (Object entryObject : attributes) {
        java.util.Map.Entry<?, ?> entry = (java.util.Map.Entry<?, ?>) entryObject;
        definitionMetadata =
            (IMapType<Object, Object>) definitionMetadata.assoc(entry.getKey(), entry.getValue());
      }
    }
    if (docstring != null) {
      definitionMetadata =
          (IMapType<Object, Object>) definitionMetadata.assoc(Keyword.create("doc"), docstring);
    }
    validateSchemaVarReference(definitionMetadata.lookup(Keyword.create("schema")));
    definitionMetadata =
        (IMapType<Object, Object>)
            definitionMetadata.assoc(
                Keyword.create("arglists"), hara.lang.data.Vector.Standard.from(null, signatures));
    Symbol definitionSymbol = symbol.withMeta(definitionMetadata);
    if (privateDefinition) {
      definitionSymbol =
          definitionSymbol.withMeta(
              (IMapType<Object, Object>)
                  definitionMetadata.assoc(Keyword.create("private"), Boolean.TRUE));
    }
    return new HaraNodes.DefineGlobal(definitionSymbol, function);
  }

  private void validateSchemaVarReference(Object schema) {
    if (!(schema instanceof List<?> reference)
        || reference.count() != 2
        || !(reference.nth(0) instanceof Symbol operator)
        || operator.getNamespace() != null
        || !"var".equals(operator.getName())
        || !(reference.nth(1) instanceof Symbol target)) {
      return;
    }
    if (context.resolve(target) == null) {
      throw error("schema Var does not exist: " + target.display());
    }
  }

  private HaraExpressionNode analyzeDeclare(List<?> form) {
    if (form.count() < 2) throw error("declare expects at least one symbol");
    Symbol[] symbols = new Symbol[(int) form.count() - 1];
    for (int i = 1; i < form.count(); i++) {
      Object value = form.nth(i);
      if (!(value instanceof Symbol) || ((Symbol) value).getNamespace() != null) {
        throw error("declare expects unqualified symbols");
      }
      symbols[i - 1] = (Symbol) value;
      context.declareCurrent(symbols[i - 1]);
    }
    return new HaraNodes.Declare(symbols);
  }

  private HaraExpressionNode analyzeDefMulti(List<?> form) {
    requireCount(form, 3, "defmulti");
    Object name = form.nth(1);
    if (!(name instanceof Symbol) || ((Symbol) name).getNamespace() != null) {
      throw error("defmulti name must be an unqualified symbol");
    }
    return new HaraNodes.DefineMulti((Symbol) name, analyze(form.nth(2)));
  }

  private HaraExpressionNode analyzeDefMethod(List<?> form) {
    requireCount(form, 5, "defmethod");
    Object name = form.nth(1);
    if (!(name instanceof Symbol)) {
      throw error("defmethod name must be a symbol");
    }
    if (!isBindingVector(form.nth(3))) {
      throw error("defmethod expects a dispatch value, parameter vector, and body");
    }
    Object[] body = new Object[(int) form.count() - 4];
    for (int i = 4; i < form.count(); i++) body[i - 4] = form.nth(i);
    return new HaraNodes.DefineMethod(
        (Symbol) name, analyze(form.nth(2)), analyzeFunction((ILinearType<?>) form.nth(3), body));
  }

  @SuppressWarnings("unchecked")
  private void addPatternBindings(
      Object pattern,
      HaraExpressionNode source,
      FrameDescriptor.Builder patternFrames,
      Map<Symbol, Integer> patternLocals,
      ArrayList<Integer> patternSlots,
      ArrayList<HaraExpressionNode> patternInitializers) {
    addPatternBindings(
        pattern, source, patternFrames, patternLocals, patternSlots, patternInitializers, null);
  }

  private void addPatternBindings(
      Object pattern,
      HaraExpressionNode source,
      FrameDescriptor.Builder patternFrames,
      Map<Symbol, Integer> patternLocals,
      ArrayList<Integer> patternSlots,
      ArrayList<HaraExpressionNode> patternInitializers,
      IMapType<?, ?> defaults) {
    if (pattern instanceof Symbol) {
      Symbol symbol = (Symbol) pattern;
      if (symbol.getNamespace() != null || patternLocals.containsKey(symbol)) {
        throw error("Invalid or duplicate binding: " + symbol.display());
      }
      HaraExpressionNode value = source;
      Object defaultForm =
          defaults == null ? null : ((IMapType<Object, Object>) defaults).lookup(symbol);
      if (defaultForm != null) {
        value = new HaraNodes.DefaultValue(source, analyze(defaultForm));
      }
      int slot = patternFrames.addSlot(FrameSlotKind.Object, symbol, null);
      patternLocals.put(symbol, slot);
      patternSlots.add(slot);
      patternInitializers.add(value);
      return;
    }
    if (pattern instanceof ILinearType<?> && isBindingVector(pattern)) {
      ILinearType<?> vector = (ILinearType<?>) pattern;
      for (int i = 0; i < vector.count(); i++) {
        Object element = vector.nth(i);
        if (element instanceof Keyword && "as".equals(((Keyword) element).getName())) {
          if (i + 2 != vector.count()) {
            throw error(":as in a destructuring vector must precede its final binding");
          }
          addPatternBindings(
              vector.nth(i + 1),
              source,
              patternFrames,
              patternLocals,
              patternSlots,
              patternInitializers,
              defaults);
          return;
        }
        if (element instanceof Symbol && "&".equals(((Symbol) element).getName())) {
          int remaining = (int) vector.count() - i;
          if (remaining != 2
              && !(remaining == 4
                  && vector.nth(i + 2) instanceof Keyword
                  && "as".equals(((Keyword) vector.nth(i + 2)).getName()))) {
            throw error(
                "& in a destructuring vector must precede its final binding and optional :as");
          }
          addPatternBindings(
              vector.nth(i + 1),
              new HaraNodes.Rest(source, i),
              patternFrames,
              patternLocals,
              patternSlots,
              patternInitializers,
              defaults);
          if (remaining == 4) {
            addPatternBindings(
                vector.nth(i + 3),
                source,
                patternFrames,
                patternLocals,
                patternSlots,
                patternInitializers,
                defaults);
          }
          return;
        }
        addPatternBindings(
            element,
            new HaraNodes.Lookup(source, i),
            patternFrames,
            patternLocals,
            patternSlots,
            patternInitializers,
            defaults);
      }
      return;
    }
    if (pattern instanceof IMapType<?, ?>) {
      IMapType<Object, Object> map = (IMapType<Object, Object>) pattern;
      Object as = map.lookup(Keyword.create(null, "as"));
      if (as != null) {
        addPatternBindings(
            as, source, patternFrames, patternLocals, patternSlots, patternInitializers, defaults);
      }
      Object keys = map.lookup(Keyword.create(null, "keys"));
      if (keys instanceof ILinearType<?>) {
        ILinearType<?> keySymbols = (ILinearType<?>) keys;
        for (int i = 0; i < keySymbols.count(); i++) {
          Object key = keySymbols.nth(i);
          if (!(key instanceof Symbol)) {
            throw error(":keys destructuring expects symbols");
          }
          addPatternBindings(
              key,
              new HaraNodes.Lookup(source, Keyword.create(null, ((Symbol) key).getName())),
              patternFrames,
              patternLocals,
              patternSlots,
              patternInitializers,
              mapDefaults(map));
        }
      }
      Object strs = map.lookup(Keyword.create(null, "strs"));
      if (strs instanceof ILinearType<?>) {
        ILinearType<?> keySymbols = (ILinearType<?>) strs;
        for (int i = 0; i < keySymbols.count(); i++) {
          Object key = keySymbols.nth(i);
          if (!(key instanceof Symbol)) {
            throw error(":strs destructuring expects symbols");
          }
          addPatternBindings(
              key,
              new HaraNodes.Lookup(source, ((Symbol) key).getName()),
              patternFrames,
              patternLocals,
              patternSlots,
              patternInitializers,
              mapDefaults(map));
        }
      }
      Object syms = map.lookup(Keyword.create(null, "syms"));
      if (syms instanceof ILinearType<?>) {
        ILinearType<?> keySymbols = (ILinearType<?>) syms;
        for (int i = 0; i < keySymbols.count(); i++) {
          Object key = keySymbols.nth(i);
          if (!(key instanceof Symbol)) {
            throw error(":syms destructuring expects symbols");
          }
          addPatternBindings(
              key,
              new HaraNodes.Lookup(source, key),
              patternFrames,
              patternLocals,
              patternSlots,
              patternInitializers,
              mapDefaults(map));
        }
      }
      Iterator<?> entries = map.iterator();
      while (entries.hasNext()) {
        Object entryValue = entries.next();
        if (!(entryValue instanceof java.util.Map.Entry<?, ?>)) {
          continue;
        }
        java.util.Map.Entry<?, ?> entry = (java.util.Map.Entry<?, ?>) entryValue;
        Object binding = entry.getKey();
        if (binding instanceof Keyword
            && ("as".equals(((Keyword) binding).getName())
                || "keys".equals(((Keyword) binding).getName())
                || "strs".equals(((Keyword) binding).getName())
                || "syms".equals(((Keyword) binding).getName())
                || "or".equals(((Keyword) binding).getName()))) {
          continue;
        }
        addPatternBindings(
            binding,
            new HaraNodes.Lookup(source, entry.getValue()),
            patternFrames,
            patternLocals,
            patternSlots,
            patternInitializers,
            mapDefaults(map));
      }
      return;
    }
    throw error("unsupported binding pattern");
  }

  @SuppressWarnings("unchecked")
  private IMapType<?, ?> mapDefaults(IMapType<?, ?> pattern) {
    Object defaults = ((IMapType<Object, Object>) pattern).lookup(Keyword.create(null, "or"));
    return defaults instanceof IMapType<?, ?> ? (IMapType<?, ?>) defaults : null;
  }

  private HaraExpressionNode analyzeLoop(List<?> form) {
    if (form.count() < 3 || !isBindingVector(form.nth(1))) {
      throw error("loop expects a binding vector and a body");
    }
    ILinearType<?> bindings = (ILinearType<?>) form.nth(1);
    if (bindings.count() % 2 != 0) {
      throw error("loop expects an even number of binding forms");
    }

    int bindingCount = (int) bindings.count() / 2;
    int[] rawSlots = new int[bindingCount];
    int[] scratchSlots = new int[bindingCount];
    HaraExpressionNode[] initializers = new HaraExpressionNode[bindingCount];
    Map<Symbol, Integer> bodyLocals = new HashMap<>(locals);
    ArrayList<int[]> patternSlots = new ArrayList<>();
    ArrayList<HaraExpressionNode[]> patternInitializers = new ArrayList<>();
    Map<Symbol, Integer> introducedLocals = new HashMap<>();
    for (int i = 0; i < bindingCount; i++) {
      Object pattern = bindings.nth(i * 2L);
      initializers[i] = analyze(bindings.nth(i * 2L + 1));
      rawSlots[i] =
          frames.addSlot(FrameSlotKind.Object, Symbol.create(null, "__hara_loop_" + i), null);
      scratchSlots[i] =
          frames.addSlot(FrameSlotKind.Object, Symbol.create(null, "__hara_recur_" + i), null);
      ArrayList<Integer> slots = new ArrayList<>();
      ArrayList<HaraExpressionNode> values = new ArrayList<>();
      addPatternBindings(
          pattern, new HaraNodes.ReadLocal(rawSlots[i]), frames, introducedLocals, slots, values);
      bodyLocals.putAll(introducedLocals);
      patternSlots.add(slots.stream().mapToInt(Integer::intValue).toArray());
      patternInitializers.add(values.toArray(new HaraExpressionNode[0]));
    }

    HaraNodes.RecurTarget target = new HaraNodes.RecurTarget(rawSlots, scratchSlots);
    for (int i = 2; i < form.count() - 1; i++) {
      validateTailRecurs(form.nth(i), false);
    }
    validateTailRecurs(form.nth(form.count() - 1), true);
    HaraAnalyzer bodyAnalyzer = subScope(bodyLocals, target);
    HaraExpressionNode body = bodyAnalyzer.analyzeDo(form, 2);
    for (int i = bindingCount - 1; i >= 0; i--) {
      body = new HaraNodes.Let(patternSlots.get(i), patternInitializers.get(i), body);
    }
    return new HaraNodes.Loop(target, rawSlots, initializers, body);
  }

  private HaraExpressionNode analyzeRecur(List<?> form) {
    if (recurTarget == null) {
      throw error("recur used outside loop");
    }
    if (form.count() - 1 != recurTarget.arity()) {
      throw error("recur expects " + recurTarget.arity() + " arguments");
    }
    HaraExpressionNode[] values = new HaraExpressionNode[recurTarget.arity()];
    for (int i = 0; i < values.length; i++) {
      values[i] = analyze(form.nth(i + 1));
    }
    return new HaraNodes.Recur(recurTarget, values);
  }

  private HaraExpressionNode analyzeThrow(List<?> form) {
    requireCount(form, 2, "throw");
    return new HaraNodes.Throw(analyze(form.nth(1)));
  }

  private HaraExpressionNode analyzeTry(List<?> form) {
    if (form.count() < 2) {
      throw error("try expects a body");
    }
    ArrayList<Object> bodyForms = new ArrayList<>();
    ArrayList<List<?>> catchForms = new ArrayList<>();
    List<?> finallyForm = null;
    for (int i = 1; i < form.count(); i++) {
      Object clause = form.nth(i);
      if (clause instanceof List<?> && ((List<?>) clause).count() > 0) {
        Object name = ((List<?>) clause).nth(0);
        if (name instanceof Symbol
            && ((Symbol) name).getNamespace() == null
            && "catch".equals(((Symbol) name).getName())) {
          catchForms.add((List<?>) clause);
          continue;
        }
        if (name instanceof Symbol
            && ((Symbol) name).getNamespace() == null
            && "finally".equals(((Symbol) name).getName())) {
          if (finallyForm != null || i != form.count() - 1) {
            throw error("finally must be the last try clause and may appear once");
          }
          finallyForm = (List<?>) clause;
          continue;
        }
      }
      if (!catchForms.isEmpty() || finallyForm != null) {
        throw error("try clauses must follow the body");
      }
      bodyForms.add(clause);
    }
    HaraExpressionNode body = analyzeForms(bodyForms.toArray());
    HaraNodes.Try.CatchClause[] catches = new HaraNodes.Try.CatchClause[catchForms.size()];
    boolean sawUnconditionalCatch = false;
    for (int i = 0; i < catchForms.size(); i++) {
      List<?> catchForm = catchForms.get(i);
      if (sawUnconditionalCatch) {
        throw error("unconditional catch must be the last catch clause", catchForm);
      }
      if (catchForm.count() < 3) {
        throw error("catch expects a binding and body, optionally preceded by an error-code selector", catchForm);
      }
      Object selector = catchForm.nth(1);
      Symbol binding;
      int bodyStart;
      if (catchForm.count() == 3) {
        if (!(selector instanceof Symbol symbol) || symbol.getNamespace() != null) {
          throw error("unconditional catch expects an unqualified binding and body", catchForm);
        }
        selector = null;
        binding = symbol;
        bodyStart = 2;
        sawUnconditionalCatch = true;
      } else {
        if (catchForm.count() < 4 || !(catchForm.nth(2) instanceof Symbol symbol)
            || symbol.getNamespace() != null) {
          throw error("selected catch expects a selector, unqualified binding, and body", catchForm);
        }
        binding = symbol;
        bodyStart = 3;
        validateCatchSelector(selector, catchForm);
      }
      int catchSlot = frames.addSlot(FrameSlotKind.Object, binding, null);
      Map<Symbol, Integer> catchLocals = new HashMap<>(locals);
      catchLocals.put(binding, catchSlot);
      HaraAnalyzer catchAnalyzer = subScope(catchLocals, recurTarget);
      catches[i] =
          new HaraNodes.Try.CatchClause(
              selector, catchSlot, catchAnalyzer.analyzeDo(catchForm, bodyStart));
    }
    HaraExpressionNode finallyBody = null;
    if (finallyForm != null) {
      finallyBody = analyzeDo(finallyForm, 1);
    }
    return new HaraNodes.Try(body, catches, finallyBody);
  }

  private void validateCatchSelector(Object selector, Object form) {
    if (selector instanceof Symbol symbol
        && symbol.getNamespace() == null
        && ("Exception".equals(symbol.getName())
            || "Throwable".equals(symbol.getName())
            || context.hasNativeSymbol(symbol))) {
      return;
    }
    if (selector instanceof Keyword keyword) {
      if (keyword.getNamespace() != null) return;
      throw error("catch error code must be a namespaced keyword", form);
    }
    if (selector instanceof ILinearType selectors) {
      if (selectors.count() == 0) throw error("catch error code vector must not be empty", form);
      for (Object candidate : selectors) {
        if (!(candidate instanceof Keyword keyword) || keyword.getNamespace() == null) {
          throw error("catch error code vector must contain namespaced keywords", form);
        }
      }
      return;
    }
    throw error("catch selector must be a namespaced keyword or non-empty vector of namespaced keywords", form);
  }

  private void validateTailRecurs(Object form, boolean tail) {
    if (!(form instanceof List<?>) || ((List<?>) form).count() == 0) {
      return;
    }
    List<?> list = (List<?>) form;
    Object operator = list.nth(0);
    if (operator instanceof Symbol && "recur".equals(((Symbol) operator).getName())) {
      if (!tail) {
        throw error("recur must appear in tail position");
      }
      return;
    }
    if (operator instanceof Symbol && "if".equals(((Symbol) operator).getName())) {
      if (list.count() >= 2) validateTailRecurs(list.nth(1), false);
      if (list.count() >= 3) validateTailRecurs(list.nth(2), tail);
      if (list.count() >= 4) validateTailRecurs(list.nth(3), tail);
      return;
    }
    if (operator instanceof Symbol && "do".equals(((Symbol) operator).getName())) {
      for (int i = 1; i < list.count(); i++) {
        validateTailRecurs(list.nth(i), tail && i == list.count() - 1);
      }
      return;
    }
    if (operator instanceof Symbol && "cond".equals(((Symbol) operator).getName())) {
      for (int i = 1; i < list.count(); i += 2) {
        validateTailRecurs(list.nth(i), false);
        if (i + 1 < list.count()) validateTailRecurs(list.nth(i + 1), tail);
      }
      return;
    }
    if (operator instanceof Symbol && "let".equals(((Symbol) operator).getName())) {
      if (list.count() >= 2 && list.nth(1) instanceof ILinearType<?>) {
        ILinearType<?> bindings = (ILinearType<?>) list.nth(1);
        for (int i = 1; i < bindings.count(); i += 2) {
          validateTailRecurs(bindings.nth(i), false);
        }
      }
      for (int i = 2; i < list.count(); i++) {
        validateTailRecurs(list.nth(i), tail && i == list.count() - 1);
      }
      return;
    }
    if (operator instanceof Symbol
        && ("fn".equals(((Symbol) operator).getName())
            || "defn".equals(((Symbol) operator).getName())
            || "loop".equals(((Symbol) operator).getName()))) {
      return;
    }
    for (int i = 0; i < list.count(); i++) {
      validateTailRecurs(list.nth(i), false);
    }
  }

  private HaraExpressionNode analyzeAdd(List<?> form) {
    if (form.count() == 1) return new HaraNodes.Literal(0L);
    HaraExpressionNode result = analyze(form.nth(1));
    for (int i = 2; i < form.count(); i++) {
      result = new HaraNodes.Add(result, analyze(form.nth(i)));
    }
    return result;
  }

  private HaraExpressionNode analyzeNumeric(
      List<?> form, HaraNodes.Numeric.Operator operator, String name) {
    requireCount(form, 3, name);
    return new HaraNodes.Numeric(operator, analyze(form.nth(1)), analyze(form.nth(2)));
  }

  private HaraExpressionNode analyzeVariadicNumeric(
      List<?> form, HaraNodes.Numeric.Operator operator, String name, Long identity) {
    int argumentCount = (int) form.count() - 1;
    if (argumentCount == 0) {
      if (identity == null || operator != HaraNodes.Numeric.Operator.MULTIPLY) {
        throw error(name + " expects at least one argument");
      }
      return new HaraNodes.Literal(identity);
    }
    HaraExpressionNode result;
    int start;
    if (argumentCount == 1 && operator != HaraNodes.Numeric.Operator.MULTIPLY) {
      result =
          new HaraNodes.Numeric(operator, new HaraNodes.Literal(identity), analyze(form.nth(1)));
      return result;
    }
    result = analyze(form.nth(1));
    start = 2;
    for (int i = start; i < form.count(); i++) {
      result = new HaraNodes.Numeric(operator, result, analyze(form.nth(i)));
    }
    return result;
  }

  private HaraExpressionNode analyzeCompare(
      List<?> form, HaraNodes.Compare.Operator operator, String name) {
    if (form.count() < 3) throw error(name + " expects at least two arguments");
    if (form.count() == 3) {
      return new HaraNodes.Compare(operator, analyze(form.nth(1)), analyze(form.nth(2)));
    }
    HaraExpressionNode[] values = new HaraExpressionNode[(int) form.count() - 1];
    for (int i = 1; i < form.count(); i++) values[i - 1] = analyze(form.nth(i));
    return new HaraNodes.CompareChain(operator, values);
  }

  private HaraExpressionNode analyzeDef(List<?> form) {
    requireCount(form, 3, "def");
    Object name = form.nth(1);
    if (!(name instanceof Symbol)) {
      throw error("def name must be a symbol");
    }
    Symbol symbol = definitionSymbol((Symbol) name, form);
    if (symbol.getNamespace() != null) {
      throw error("def name must not be qualified");
    }
    context.declareCurrent(symbol);
    return new HaraNodes.DefineGlobal(symbol, analyze(form.nth(2)));
  }

  @SuppressWarnings("unchecked")
  private Symbol definitionSymbol(Symbol symbol, List<?> form) {
    IMapType<Object, Object> metadata =
        symbol.meta() instanceof IMapType<?, ?>
            ? (IMapType<Object, Object>) symbol.meta()
            : hara.lang.data.Map.Standard.EMPTY;
    if (!(form.meta() instanceof IMapType<?, ?>)) return symbol.withMeta(metadata);
    IMapType<Object, Object> source = (IMapType<Object, Object>) form.meta();
    String[] keys = {"file", "line", "column", "end-line", "end-column"};
    for (String key : keys) {
      Keyword keyword = Keyword.create(key);
      Object value = source.lookup(keyword);
      if (value != null) metadata = (IMapType<Object, Object>) metadata.assoc(keyword, value);
    }
    return symbol.withMeta(metadata);
  }

  private HaraExpressionNode analyzeVar(List<?> form) {
    requireCount(form, 2, "var");
    Object name = form.nth(1);
    if (!(name instanceof Symbol)) {
      throw error("var expects a symbol");
    }
    return new HaraNodes.VarReference((Symbol) name);
  }

  private HaraExpressionNode analyzeSetVar(List<?> form) {
    requireCount(form, 3, "set!");
    Object place = form.nth(1);
    if (place instanceof List<?> fieldPlace
        && fieldPlace.count() > 0
        && fieldPlace.nth(0) instanceof Symbol operator
        && operator.getNamespace() == null
        && "field".equals(operator.getName())) {
      requireCount(fieldPlace, 3, "field");
      return new HaraNodes.SetField(
          analyze(fieldPlace.nth(1)),
          fieldName(fieldPlace.nth(2)),
          analyze(form.nth(2)));
    }
    if (!(place instanceof Symbol)) {
      throw error("set! expects a Var symbol or mutable field place");
    }
    return new HaraNodes.SetVar((Symbol) place, analyze(form.nth(2)));
  }

  private HaraExpressionNode analyzeNamedDefinition(List<?> form, boolean mutable) {
    String kind = mutable ? "defmutable" : "defstruct";
    if (form.count() < 3) throw error(kind + " expects a name and field vector");
    Object name = form.nth(1);
    if (!(name instanceof Symbol)) {
      throw error(kind + " name must be a symbol");
    }
    Symbol symbol = (Symbol) name;
    if (symbol.getNamespace() != null) {
      throw error(kind + " name must not be qualified");
    }
    if (!isBindingVector(form.nth(2))) {
      throw error(kind + " expects a field vector");
    }

    ILinearType<?> fields = (ILinearType<?>) form.nth(2);
    HalcSchema.NamedField[] fieldSpecifications =
        new HalcSchema.NamedField[(int) fields.count()];
    Set<String> seen = new HashSet<>();
    for (int i = 0; i < fieldSpecifications.length; i++) {
      Object field = fields.nth(i);
      if (!(field instanceof Symbol) && !isBindingVector(field)) {
        throw error(kind + " fields must be symbols or [name schema] declarations");
      }
      HalcSchema.NamedField specification = HalcSchema.normalizeNamedField(field);
      fieldSpecifications[i] = specification;
      if (!seen.add(specification.name())) {
        throw error("Duplicate " + kind + " field: " + specification.name());
      }
    }

    java.util.ArrayList<HaraExpressionNode> extensions = new java.util.ArrayList<>();
    int index = 3;
    while (index < form.count()) {
      Object protocol = form.nth(index++);
      if (!(protocol instanceof Symbol)) {
        throw error(kind + " protocol clause expects a protocol symbol");
      }
      int start = index;
      while (index < form.count() && form.nth(index) instanceof List<?>) index++;
      if (start == index) {
        throw error(kind + " protocol clause requires method implementations");
      }
      Object[] extension = new Object[index - start + 3];
      extension[0] = Symbol.create("extend-type");
      extension[1] = Symbol.create(symbol.getName());
      extension[2] = protocol;
      for (int i = start; i < index; i++) extension[i - start + 3] = form.nth(i);
      extensions.add(analyzeExtendType(List.Standard.from(null, extension)));
    }
    return new HaraNodes.DefineNamedType(
        symbol,
        fieldSpecifications,
        mutable,
        extensions.toArray(new HaraExpressionNode[0]));
  }

  private HaraExpressionNode analyzeField(List<?> form) {
    requireCount(form, 3, "field");
    return new HaraNodes.ReadField(analyze(form.nth(1)), fieldName(form.nth(2)));
  }

  private String fieldName(Object field) {
    if (field instanceof Keyword) {
      if (((Keyword) field).getNamespace() != null) {
        throw error("field name must not be qualified");
      }
      return ((Keyword) field).getName();
    } else if (field instanceof Symbol) {
      if (((Symbol) field).getNamespace() != null) {
        throw error("field name must not be qualified");
      }
      return ((Symbol) field).getName();
    }
    throw error("field name must be a keyword or symbol");
  }

  private HaraExpressionNode analyzeDefProtocol(List<?> form) {
    if (form.count() < 3) {
      throw error("defprotocol expects a name and method declarations");
    }
    Object name = form.nth(1);
    if (!(name instanceof Symbol) || ((Symbol) name).getNamespace() != null) {
      throw error("defprotocol name must be an unqualified symbol");
    }
    Map<String, Integer> methodArities = new LinkedHashMap<>();
    for (int i = 2; i < form.count(); i++) {
      Object declaration = form.nth(i);
      if (!(declaration instanceof List<?>) || ((List<?>) declaration).count() != 2) {
        throw error("defprotocol method declarations must be (name [arguments])");
      }
      List<?> method = (List<?>) declaration;
      Object methodName = method.nth(0);
      if (!(methodName instanceof Symbol) || ((Symbol) methodName).getNamespace() != null) {
        throw error("protocol method name must be an unqualified symbol");
      }
      if (!isBindingVector(method.nth(1))) {
        throw error("protocol method arguments must be a vector");
      }
      ILinearType<?> parameters = (ILinearType<?>) method.nth(1);
      if (parameters.count() == 0) {
        throw error("protocol methods must take a receiver as their first argument");
      }
      Set<Symbol> parameterNames = new HashSet<>();
      for (int parameterIndex = 0; parameterIndex < parameters.count(); parameterIndex++) {
        Object parameter = parameters.nth(parameterIndex);
        if (!(parameter instanceof Symbol) || !parameterNames.add((Symbol) parameter)) {
          throw error("protocol method arguments must be unique symbols");
        }
      }
      String methodKey = ((Symbol) methodName).getName();
      if (methodArities.put(methodKey, (int) parameters.count()) != null) {
        throw error("Duplicate protocol method: " + methodKey);
      }
    }
    return new HaraNodes.DefineProtocol(
        (Symbol) name, new HaraProtocol(((Symbol) name).getName(), methodArities));
  }

  private HaraExpressionNode analyzeExtendType(List<?> form) {
    if (form.count() < 4) {
      throw error("extend-type expects a type, protocol, and method implementations");
    }
    Object protocol = form.nth(2);
    if (!(protocol instanceof Symbol)) {
      throw error("extend-type protocol must be a symbol");
    }
    HaraNodes.ProtocolMethodImplementation[] methods =
        new HaraNodes.ProtocolMethodImplementation[(int) form.count() - 3];
    Set<String> seen = new HashSet<>();
    for (int i = 3; i < form.count(); i++) {
      Object implementation = form.nth(i);
      if (!(implementation instanceof List<?>) || ((List<?>) implementation).count() < 3) {
        throw error("extend-type implementations must be (name [arguments] body...)");
      }
      List<?> method = (List<?>) implementation;
      Object methodName = method.nth(0);
      if (!(methodName instanceof Symbol) || ((Symbol) methodName).getNamespace() != null) {
        throw error("extended method name must be an unqualified symbol");
      }
      String methodKey = ((Symbol) methodName).getName();
      if (!seen.add(methodKey)) {
        throw error("Duplicate extended method: " + methodKey);
      }
      if (!isBindingVector(method.nth(1))) {
        throw error("extended method arguments must be a vector");
      }
      Object[] body = new Object[(int) method.count() - 2];
      for (int j = 2; j < method.count(); j++) {
        body[j - 2] = method.nth(j);
      }
      methods[i - 3] =
          new HaraNodes.ProtocolMethodImplementation(
              methodKey, analyzeFunction((ILinearType<?>) method.nth(1), body));
    }
    return new HaraNodes.ExtendType(analyze(form.nth(1)), analyze(protocol), methods);
  }

  private HaraExpressionNode analyzeMarkerCall(List<?> form) {
    if (form.count() < 3) throw error("dot expects a receiver and at least one member step");
    HaraExpressionNode result = analyze(form.nth(1));
    for (int i = 2; i < form.count(); i++) {
      Object step = form.nth(i);
      if (step instanceof Symbol) {
        Symbol member = (Symbol) step;
        if (member.getNamespace() != null) throw error("dot field must be an unqualified symbol");
        result = new HaraNodes.NativeReadMember(result, member.getName());
      } else if (step instanceof List<?>) {
        List<?> call = (List<?>) step;
        if (call.count() == 0
            || !(call.nth(0) instanceof Symbol)
            || ((Symbol) call.nth(0)).getNamespace() != null) {
          throw error("dot method must be an unqualified symbol");
        }
        HaraExpressionNode[] arguments = new HaraExpressionNode[(int) call.count() - 1];
        for (int j = 1; j < call.count(); j++) arguments[j - 1] = analyze(call.nth(j));
        result = new HaraNodes.MarkerCall(result, ((Symbol) call.nth(0)).getName(), arguments);
      } else if (step instanceof ILinearType<?>
          && "[".equals(((ILinearType<?>) step).startString())
          && ((ILinearType<?>) step).count() == 1) {
        result = new HaraNodes.NativeIndex(result, analyze(((ILinearType<?>) step).nth(0)));
      } else {
        throw error("dot steps must be fields, method lists, or one-element index vectors");
      }
    }
    return result;
  }

  private HaraExpressionNode analyzeNativeNew(List<?> form) {
    if (form.count() < 2) throw error("new expects a type and optional arguments");
    HaraExpressionNode[] arguments = new HaraExpressionNode[(int) form.count() - 2];
    for (int i = 2; i < form.count(); i++) arguments[i - 2] = analyze(form.nth(i));
    return new HaraNodes.NativeConstruct(analyze(form.nth(1)), arguments);
  }

  private String memberName(Object value) {
    if (value instanceof Keyword) {
      Keyword keyword = (Keyword) value;
      if (keyword.getNamespace() != null) {
        throw error("host member name must not be qualified");
      }
      return keyword.getName();
    }
    if (value instanceof Symbol) {
      Symbol symbol = (Symbol) value;
      if (symbol.getNamespace() != null) {
        throw error("host member name must not be qualified");
      }
      return symbol.getName();
    }
    if (value instanceof String) {
      return (String) value;
    }
    throw error("host member name must be a keyword, symbol, or string");
  }

  private HaraExpressionNode analyzeDefMacro(List<?> form) {
    if (form.count() < 3) {
      throw error("defmacro expects a name, parameter vector or arity clauses, and body");
    }
    Object name = form.nth(1);
    if (!(name instanceof Symbol)) {
      throw error("defmacro name must be a symbol");
    }
    Symbol symbol = definitionSymbol((Symbol) name, form);
    if (symbol.getNamespace() != null) {
      throw error("defmacro name must not be qualified");
    }

    int parametersIndex = 2;
    String docstring = null;
    if (form.nth(parametersIndex) instanceof String) {
      docstring = (String) form.nth(parametersIndex);
      parametersIndex++;
    }
    IMapType<?, ?> attributes = null;
    if (parametersIndex < form.count() && form.nth(parametersIndex) instanceof IMapType<?, ?>) {
      attributes = (IMapType<?, ?>) form.nth(parametersIndex);
      parametersIndex++;
    }
    if (parametersIndex >= form.count()) {
      throw error("defmacro expects a name, parameter vector or arity clauses, and body");
    }

    Object parameterForm = form.nth(parametersIndex);
    boolean singleArity = isBindingVector(parameterForm);
    if (singleArity && parametersIndex + 1 >= form.count()) {
      throw error("defmacro expects a body");
    }
    if (!singleArity
        && (!(parameterForm instanceof List<?>) || ((List<?>) parameterForm).count() < 2)) {
      throw error("defmacro expects a parameter vector or arity clauses");
    }

    ArrayList<Object> signatures = new ArrayList<>();
    ArrayList<HaraFunction> compiledArities = new ArrayList<>();
    int clauseCount = singleArity ? 1 : (int) form.count() - parametersIndex;
    for (int clauseIndex = 0; clauseIndex < clauseCount; clauseIndex++) {
      ILinearType<?> parameters;
      Object[] body;
      if (singleArity) {
        parameters = (ILinearType<?>) parameterForm;
        body = new Object[(int) form.count() - parametersIndex - 1];
        for (int i = parametersIndex + 1; i < form.count(); i++) {
          body[i - parametersIndex - 1] = form.nth(i);
        }
      } else {
        Object clause = form.nth(parametersIndex + clauseIndex);
        if (!(clause instanceof List<?>) || ((List<?>) clause).count() < 2) {
          throw error("defmacro arity clauses must contain a parameter vector and body");
        }
        List<?> clauseList = (List<?>) clause;
        if (!isBindingVector(clauseList.nth(0))) {
          throw error("defmacro arity clause expects a parameter vector");
        }
        parameters = (ILinearType<?>) clauseList.nth(0);
        body = new Object[(int) clauseList.count() - 1];
        for (int i = 1; i < clauseList.count(); i++) {
          body[i - 1] = clauseList.nth(i);
        }
      }
      signatures.add(parameters);
      ArrayList<Object> compiledParameters = new ArrayList<>();
      compiledParameters.add(Symbol.create("&form"));
      compiledParameters.add(Symbol.create("&env"));
      for (Object parameter : parameters) compiledParameters.add(parameter);
      HaraExpressionNode compiled =
          analyzeFunction(
              hara.lang.data.Vector.Standard.from(null, compiledParameters.toArray()), body);
      if (!(compiled instanceof HaraNodes.FunctionLiteral function)) {
        throw error("defmacro body did not compile to a Hara function");
      }
      compiledArities.add(function.instantiateWithoutClosure());
    }

    @SuppressWarnings("unchecked")
    IMapType<Object, Object> metadata =
        symbol.meta() instanceof IMapType<?, ?>
            ? (IMapType<Object, Object>) symbol.meta()
            : hara.lang.data.Map.Standard.EMPTY;
    if (attributes != null) {
      for (Object entryObject : attributes) {
        java.util.Map.Entry<?, ?> entry = (java.util.Map.Entry<?, ?>) entryObject;
        metadata = (IMapType<Object, Object>) metadata.assoc(entry.getKey(), entry.getValue());
      }
    }
    if (docstring != null) {
      metadata = (IMapType<Object, Object>) metadata.assoc(Keyword.create("doc"), docstring);
    }
    metadata =
        (IMapType<Object, Object>)
            metadata
                .assoc(
                    Keyword.create("arglists"),
                    hara.lang.data.Vector.Standard.from(null, signatures.toArray()))
                .assoc(Keyword.create("macro"), Boolean.TRUE);
    if (!locals.isEmpty() || parent != null) {
      throw error("defmacro is only valid at namespace scope");
    }
    Symbol definition = symbol.withMeta(metadata);
    HaraFunction function =
        compiledArities.size() == 1
            ? compiledArities.get(0)
            : new HaraFunction(compiledArities.toArray(new HaraFunction[0]));
    context.defineMacro(
        definition,
        new HaraMacro(
            context,
            context.currentNamespaceName(),
            definition,
            function));
    return new HaraNodes.Literal(null);
  }

  private HaraExpressionNode analyzeMacroExpand(List<?> form, boolean recursive) {
    requireCount(form, 2, recursive ? "macroexpand" : "macroexpand-1");
    return new HaraNodes.MacroExpand(analyze(form.nth(1)), recursive);
  }

  private HaraExpressionNode analyzeRequire(List<?> form) {
    if (form.count() < 2 || form.count() > 3) {
      throw error("require expects a path and optional options map");
    }
    if (form.nth(1) instanceof String
        && (form.count() == 2 || form.nth(2) instanceof IMapType<?, ?>)) {
      Object[] arguments =
          form.count() == 2 ? new Object[] {form.nth(1)} : new Object[] {form.nth(1), form.nth(2)};
      context.requireModule(arguments);
      return new HaraNodes.Literal(null);
    }
    HaraExpressionNode path = analyze(form.nth(1));
    HaraExpressionNode options =
        form.count() == 3 ? new HaraNodes.Literal(form.nth(2)) : new HaraNodes.Literal(null);
    return new HaraNodes.Require(path, options);
  }

  private HaraExpressionNode analyzeNamespace(List<?> form) {
    if (form.count() < 2) throw error("ns expects a namespace name");
    Object name = form.nth(1);
    if (!(name instanceof Symbol) || ((Symbol) name).getNamespace() != null) {
      throw error("ns name must be an unqualified symbol");
    }
    Object[] clauses = new Object[(int) form.count() - 2];
    for (int i = 2; i < form.count(); i++) clauses[i - 2] = form.nth(i);
    context.prepareCurrentNamespace((Symbol) name, clauses);
    return new HaraNodes.SetNamespace((Symbol) name, clauses);
  }

  private HaraExpressionNode analyzeAnonymousNamespace(List<?> form) {
    Object[] clauses = new Object[(int) form.count() - 1];
    for (int i = 1; i < form.count(); i++) {
      Object clause = form.nth(i);
      if (!(clause instanceof List<?>)) {
        throw error("ns+ does not accept a namespace name");
      }
      clauses[i - 1] = clause;
    }
    Symbol name = Symbol.create(context.currentNamespaceName());
    context.prepareCurrentNamespace(name, clauses);
    return new HaraNodes.SetAnonymousNamespace(name, clauses);
  }

  private HaraExpressionNode analyzeAlias(List<?> form) {
    requireCount(form, 3, "alias");
    if (!(form.nth(1) instanceof Symbol) || !(form.nth(2) instanceof Symbol)) {
      throw error("alias expects an alias and namespace symbol");
    }
    return new HaraNodes.DefineAlias((Symbol) form.nth(1), (Symbol) form.nth(2));
  }

  private Object expandMacro(Object form) {
    if (!(form instanceof List<?>)) {
      return form;
    }
    List<?> list = (List<?>) form;
    if (list.count() == 0 || !(list.nth(0) instanceof Symbol)) {
      return form;
    }
    Symbol operator = (Symbol) list.nth(0);
    if (isLexicallyBound(operator)) {
      return form;
    }
    if (context.isSpecialSymbol(operator)) {
      return form;
    }
    if ("defmacro".equals(operator.getName())) {
      return form;
    }
    if ("->".equals(operator.getName())) return expandThread(list, false);
    if ("->>".equals(operator.getName())) return expandThread(list, true);
    HaraMacro macro = context.resolveMacro(operator);
    if (macro == null) return form;
    Object expansion = macro.expand(list, macroEnvironment(list));
    EvaluationJournal.macro(operator.toString(), form, expansion);
    return expansion;
  }

  private Object macroEnvironment(List<?> invocation) {
    ArrayList<Object> localEntries = new ArrayList<>();
    LinkedHashSet<Symbol> visible = new LinkedHashSet<>();
    for (HaraAnalyzer scope = this; scope != null; scope = scope.parent) {
      visible.addAll(scope.locals.keySet());
    }
    for (Symbol local : visible) {
      localEntries.add(local);
      localEntries.add(Boolean.TRUE);
    }
    ArrayList<Object> entries = new ArrayList<>();
    entries.add(Keyword.create("ns"));
    entries.add(Symbol.create(context.currentNamespaceName()));
    entries.add(Keyword.create("locals"));
    entries.add(hara.lang.data.Map.Standard.from(null, localEntries.toArray()));
    entries.add(Keyword.create("aliases"));
    entries.add(context.macroAliases());
    Object metadata = invocation.meta();
    if (metadata instanceof IMapType<?, ?> span) {
      for (String key : new String[] {"file", "line", "column"}) {
        Object value = ((IMapType) span).lookup(Keyword.create(key));
        if (value != null) {
          entries.add(Keyword.create(key));
          entries.add(value);
        }
      }
    }
    return hara.lang.data.Map.Standard.from(null, entries.toArray());
  }

  private Object expandThread(List<?> form, boolean last) {
    if (form.count() < 2) {
      throw error((last ? "->>" : "->") + " expects an initial form");
    }
    Object result = form.nth(1);
    for (int i = 2; i < form.count(); i++) {
      Object step = form.nth(i);
      if (containsThreadPlaceholder(step)) {
        result =
            List.Standard.from(
                null,
                Symbol.create("let"),
                BuiltinStruct.vector(new Object[] {Symbol.create("%"), result}),
                step);
        continue;
      }
      if (step instanceof List<?>) {
        List<?> stepList = (List<?>) step;
        if (stepList.count() == 0) {
          throw error((last ? "->>" : "->") + " cannot thread into an empty list");
        }
        Object[] values = new Object[(int) stepList.count() + 1];
        if (last) {
          for (int j = 0; j < stepList.count(); j++) values[j] = stepList.nth(j);
          values[values.length - 1] = result;
        } else {
          values[0] = stepList.nth(0);
          values[1] = result;
          for (int j = 1; j < stepList.count(); j++) values[j + 1] = stepList.nth(j);
        }
        result = List.Standard.from(null, values);
      } else {
        result = List.Standard.from(null, step, result);
      }
    }
    return result;
  }

  private boolean containsThreadPlaceholder(Object form) {
    if (form instanceof Symbol symbol) {
      return symbol.getNamespace() == null && "%".equals(symbol.getName());
    }
    if (form instanceof List<?> list) {
      if (list.count() > 0 && list.nth(0) instanceof Symbol operator) {
        String name = operator.getName();
        if (operator.getNamespace() == null
            && ("quote".equals(name) || "syntax-quote".equals(name))) {
          return false;
        }
      }
      for (Object value : list) {
        if (containsThreadPlaceholder(value)) return true;
      }
      return false;
    }
    if (form instanceof IMapType<?, ?> map) {
      for (Object rawEntry : map) {
        java.util.Map.Entry<?, ?> entry = (java.util.Map.Entry<?, ?>) rawEntry;
        if (containsThreadPlaceholder(entry.getKey())
            || containsThreadPlaceholder(entry.getValue())) return true;
      }
      return false;
    }
    if (form instanceof ILinearType<?> linear) {
      for (Object value : linear) {
        if (containsThreadPlaceholder(value)) return true;
      }
      return false;
    }
    if (form instanceof ISetType<?> set) {
      for (Object value : set) {
        if (containsThreadPlaceholder(value)) return true;
      }
      return false;
    }
    if (form instanceof TaggedLiteral tagged) {
      return containsThreadPlaceholder(tagged.form());
    }
    return false;
  }

  private HaraExpressionNode analyzeInvocation(List<?> form) {
    HaraExpressionNode target = analyze(form.nth(0));
    HaraExpressionNode[] arguments = new HaraExpressionNode[(int) form.count() - 1];
    for (int i = 1; i < form.count(); i++) {
      arguments[i - 1] = analyze(form.nth(i));
    }
    return new HaraNodes.Invoke(target, arguments);
  }

  /**
   * Specializes get/nth call sites. Falls back to a plain invocation whenever the operator
   * is lexically shadowed or the arity is outside the specialized shape, so error behavior for
   * unsupported arities is exactly that of the generic path.
   */
  private HaraExpressionNode analyzeCollectionOp(List<?> form, HaraNodes.CollectionOp.Kind kind) {
    Symbol operator = (Symbol) form.nth(0);
    long arity = form.count() - 1;
    boolean supported;
    switch (kind) {
      case GET:
        supported = arity == 2 || arity == 3;
        break;
      case NTH:
        supported = arity == 2;
        break;
      default:
        throw new AssertionError(kind);
    }
    if (!supported || isLexicallyBound(operator)) {
      return analyzeInvocation(form);
    }
    HaraExpressionNode[] arguments = new HaraExpressionNode[(int) arity];
    for (int i = 1; i < form.count(); i++) {
      arguments[i - 1] = analyze(form.nth(i));
    }
    return new HaraNodes.CollectionOp(kind, context.canonicalSymbol(operator), arguments);
  }

  /**
   * Specializes first/rest call sites. Falls back to a plain invocation whenever the operator is
   * lexically shadowed or the arity is outside the specialized shape, so error behavior for
   * unsupported arities is exactly that of the generic path.
   */
  private HaraExpressionNode analyzeSequenceAccess(List<?> form, HaraNodes.FirstRest.Kind kind) {
    Symbol operator = (Symbol) form.nth(0);
    if (form.count() != 2 || isLexicallyBound(operator)) {
      return analyzeInvocation(form);
    }
    return new HaraNodes.FirstRest(kind, context.canonicalSymbol(operator), analyze(form.nth(1)));
  }

  private void requireCount(List<?> form, long expected, String name) {
    if (form.count() != expected) {
      throw error(name + " expects " + (expected - 1) + " arguments");
    }
  }

  private HaraException error(String message) {
    return new HaraException(message);
  }

  private HaraException error(String message, Object form) {
    HaraExpressionNode location = new HaraNodes.Literal(null);
    attachSourceSection(location, form);
    return new HaraException(message, location);
  }

  private boolean isBindingVector(Object value) {
    return value instanceof ILinearType<?> && "[".equals(((ILinearType<?>) value).startString());
  }
}
