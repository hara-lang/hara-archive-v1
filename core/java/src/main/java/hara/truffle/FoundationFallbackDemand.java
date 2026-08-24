package hara.truffle;

import hara.lang.data.Keyword;
import hara.lang.data.List;
import hara.lang.data.Symbol;
import hara.lang.protocol.ILinearType;
import hara.lang.protocol.IMapType;
import hara.lang.protocol.ISetType;
import java.util.HashSet;
import java.util.Iterator;
import java.util.Map;
import java.util.Set;

/**
 * Conservative pre-analysis check for source that needs the portable Foundation fallback.
 *
 * <p>The check runs before {@code ns} forms are analyzed. It distinguishes lexical and
 * same-unit bindings from portable Foundation Vars, including Vars that currently have an
 * optimized Java implementation but are replaced or completed by the HAL definition.
 */
final class FoundationFallbackDemand {
  private static final Set<String> TOP_LEVEL_DEFINITIONS =
      Set.of(
          "def",
          "defn",
          "defn-",
          "defmacro",
          "defstruct",
          "defmutable",
          "defprotocol",
          "defmulti");

  private final HaraContext context;

  private FoundationFallbackDemand(HaraContext context) {
    this.context = context;
  }

  static boolean requires(Object[] forms, HaraContext context) {
    return new FoundationFallbackDemand(context).requires(forms);
  }

  private boolean requires(Object[] forms) {
    int index = 0;
    while (index < forms.length) {
      if (topLevelForm(forms[index], "ns")) {
        index++;
        continue;
      }
      int end = index;
      while (end < forms.length && !topLevelForm(forms[end], "ns")) end++;
      HashSet<String> globals = new HashSet<>();
      for (int item = index; item < end; item++) predeclare(forms[item], globals);
      HashSet<String> lexical = new HashSet<>();
      while (index < end) {
        if (scan(forms[index], lexical, globals)) return true;
        index++;
      }
    }
    return false;
  }

  private void predeclare(Object form, Set<String> globals) {
    if (!(form instanceof List<?> list)
        || list.count() == 0
        || !(list.nth(0) instanceof Symbol operator)
        || operator.getNamespace() != null) {
      return;
    }
    String operation = operator.getName();
    if ("do".equals(operation)) {
      for (int index = 1; index < list.count(); index++) predeclare(list.nth(index), globals);
      return;
    }
    if ("declare".equals(operation)) {
      addDeclared(list, globals);
      return;
    }
    if (!TOP_LEVEL_DEFINITIONS.contains(operation)
        || list.count() < 2
        || !(list.nth(1) instanceof Symbol name)
        || name.getNamespace() != null) {
      return;
    }
    addDefinitionNames(globals, operation, name.getName());
    if ("defprotocol".equals(operation)) addProtocolMethodNames(list, globals);
  }

  private boolean scan(Object form, Set<String> lexical, Set<String> globals) {
    if (form instanceof Symbol symbol) return needsFallback(symbol, lexical, globals);
    if (form instanceof List<?> list) return scanList(list, lexical, globals);
    if (form instanceof IMapType<?, ?> map) {
      Iterator<?> iterator = map.iterator();
      while (iterator.hasNext()) {
        Object value = iterator.next();
        if (!(value instanceof Map.Entry<?, ?> entry)) continue;
        if (scan(entry.getKey(), lexical, globals)
            || scan(entry.getValue(), lexical, globals)) return true;
      }
      return false;
    }
    if (form instanceof ISetType<?> set) {
      for (Object value : set) if (scan(value, lexical, globals)) return true;
      return false;
    }
    if (form instanceof ILinearType<?> linear) {
      for (Object value : linear) if (scan(value, lexical, globals)) return true;
    }
    return false;
  }

  private boolean scanList(List<?> form, Set<String> lexical, Set<String> globals) {
    if (form.count() == 0) return false;
    Object operatorValue = form.nth(0);
    if (operatorValue instanceof Symbol operator && operator.getNamespace() == null) {
      String name = operator.getName();
      switch (name) {
        case "quote":
        case "comment":
        case "ns":
        case "ns+":
        case "require":
        case "alias":
          return false;
        case "syntax-quote":
          return form.count() > 1 && scanSyntaxQuote(form.nth(1), lexical, globals);
        case "do":
        case "if":
        case "when":
        case "when-not":
        case "cond":
        case "and":
        case "or":
        case "recur":
        case "throw":
        case "->":
        case "->>":
          return scanRange(form, 1, lexical, globals);
        case "let":
        case "loop":
          return scanLetLike(form, lexical, globals);
        case "letfn":
          return scanLetFn(form, lexical, globals);
        case "binding":
          return scanBinding(form, lexical, globals);
        case "try":
          return scanTry(form, lexical, globals);
        case "fn":
          return scanFn(form, lexical, globals, false);
        case "defn":
        case "defn-":
          return scanNamedFunction(form, lexical, globals, false);
        case "defmacro":
          return scanNamedFunction(form, lexical, globals, true);
        case "def":
          return scanDef(form, lexical, globals);
        case "declare":
          addDeclared(form, globals);
          return false;
        case "var":
          return form.count() > 1 && scan(form.nth(1), lexical, globals);
        case "set!":
          return scanSet(form, lexical, globals);
        case "defstruct":
        case "defmutable":
          return scanNamedValue(form, lexical, globals, name);
        case "defprotocol":
          predeclare(form, globals);
          return false;
        case "extend-type":
          return scanExtendType(form, lexical, globals);
        case "defmulti":
          return scanDefMulti(form, lexical, globals);
        case "defmethod":
          return scanDefMethod(form, lexical, globals);
        case "new":
          return scanRange(form, 2, lexical, globals);
        case "field":
          return form.count() > 1 && scan(form.nth(1), lexical, globals);
        case ".":
          return scanDot(form, lexical, globals);
        default:
          break;
      }
    }
    if (scan(operatorValue, lexical, globals)) return true;
    return scanRange(form, 1, lexical, globals);
  }

  private boolean scanRange(
      List<?> form, int start, Set<String> lexical, Set<String> globals) {
    for (int index = start; index < form.count(); index++) {
      if (scan(form.nth(index), lexical, globals)) return true;
    }
    return false;
  }

  private boolean scanLetLike(List<?> form, Set<String> lexical, Set<String> globals) {
    if (form.count() < 2 || !isVector(form.nth(1))) {
      return scanRange(form, 1, lexical, globals);
    }
    ILinearType<?> bindings = (ILinearType<?>) form.nth(1);
    HashSet<String> scope = new HashSet<>(lexical);
    for (int index = 0; index + 1 < bindings.count(); index += 2) {
      Object pattern = bindings.nth(index);
      if (scan(bindings.nth(index + 1), scope, globals)
          || scanPatternDefaults(pattern, scope, globals)) return true;
      addPatternBindings(pattern, scope);
    }
    for (int index = 2; index < form.count(); index++) {
      if (scan(form.nth(index), scope, globals)) return true;
    }
    return false;
  }

  private boolean scanLetFn(List<?> form, Set<String> lexical, Set<String> globals) {
    if (form.count() < 2 || !isVector(form.nth(1))) {
      return scanRange(form, 1, lexical, globals);
    }
    ILinearType<?> definitions = (ILinearType<?>) form.nth(1);
    HashSet<String> scope = new HashSet<>(lexical);
    for (Object value : definitions) {
      if (value instanceof List<?> definition
          && definition.count() > 0
          && definition.nth(0) instanceof Symbol name
          && name.getNamespace() == null) {
        scope.add(name.getName());
      }
    }
    for (Object value : definitions) {
      if (!(value instanceof List<?> definition) || definition.count() < 2) continue;
      if (scanFunctionShape(definition, 1, scope, globals, false)) return true;
    }
    return scanRange(form, 2, scope, globals);
  }

  private boolean scanBinding(List<?> form, Set<String> lexical, Set<String> globals) {
    if (form.count() < 2 || !isVector(form.nth(1))) {
      return scanRange(form, 1, lexical, globals);
    }
    ILinearType<?> bindings = (ILinearType<?>) form.nth(1);
    for (int index = 0; index + 1 < bindings.count(); index += 2) {
      if (scan(bindings.nth(index), lexical, globals)
          || scan(bindings.nth(index + 1), lexical, globals)) return true;
    }
    return scanRange(form, 2, lexical, globals);
  }

  private boolean scanTry(List<?> form, Set<String> lexical, Set<String> globals) {
    for (int index = 1; index < form.count(); index++) {
      Object value = form.nth(index);
      if (!(value instanceof List<?> clause)
          || clause.count() == 0
          || !(clause.nth(0) instanceof Symbol clauseName)
          || clauseName.getNamespace() != null) {
        if (scan(value, lexical, globals)) return true;
        continue;
      }
      if ("catch".equals(clauseName.getName())) {
        HashSet<String> scope = new HashSet<>(lexical);
        if (clause.count() > 2
            && clause.nth(2) instanceof Symbol binding
            && binding.getNamespace() == null) {
          scope.add(binding.getName());
        }
        if (scanRange(clause, 3, scope, globals)) return true;
      } else if ("finally".equals(clauseName.getName())) {
        if (scanRange(clause, 1, lexical, globals)) return true;
      } else if (scan(value, lexical, globals)) {
        return true;
      }
    }
    return false;
  }

  private boolean scanFn(
      List<?> form, Set<String> lexical, Set<String> globals, boolean macro) {
    int shape = 1;
    HashSet<String> scope = new HashSet<>(lexical);
    if (shape < form.count()
        && form.nth(shape) instanceof Symbol name
        && name.getNamespace() == null) {
      scope.add(name.getName());
      shape++;
    }
    return scanFunctionShape(form, shape, scope, globals, macro);
  }

  private boolean scanNamedFunction(
      List<?> form, Set<String> lexical, Set<String> globals, boolean macro) {
    if (form.count() < 2 || !(form.nth(1) instanceof Symbol name)) {
      return scanRange(form, 1, lexical, globals);
    }
    addDefinitionNames(globals, macro ? "defmacro" : "defn", name.getName());
    int shape = 2;
    if (shape < form.count() && form.nth(shape) instanceof String) shape++;
    if (shape < form.count() && form.nth(shape) instanceof IMapType<?, ?>) shape++;
    return scanFunctionShape(form, shape, lexical, globals, macro);
  }

  private boolean scanFunctionShape(
      List<?> form,
      int shape,
      Set<String> lexical,
      Set<String> globals,
      boolean macro) {
    if (shape >= form.count()) return false;
    Object parameterForm = form.nth(shape);
    if (isVector(parameterForm)) {
      return scanFunctionBody(
          (ILinearType<?>) parameterForm, form, shape + 1, lexical, globals, macro);
    }
    for (int index = shape; index < form.count(); index++) {
      Object value = form.nth(index);
      if (!(value instanceof List<?> clause)
          || clause.count() == 0
          || !isVector(clause.nth(0))) continue;
      if (scanFunctionBody(
          (ILinearType<?>) clause.nth(0), clause, 1, lexical, globals, macro)) return true;
    }
    return false;
  }

  private boolean scanFunctionBody(
      ILinearType<?> parameters,
      List<?> body,
      int start,
      Set<String> lexical,
      Set<String> globals,
      boolean macro) {
    HashSet<String> scope = new HashSet<>(lexical);
    if (macro) {
      scope.add("&form");
      scope.add("&env");
    }
    for (Object parameter : parameters) {
      if (scanPatternDefaults(parameter, scope, globals)) return true;
      addPatternBindings(parameter, scope);
    }
    return scanRange(body, start, scope, globals);
  }

  private boolean scanDef(List<?> form, Set<String> lexical, Set<String> globals) {
    if (form.count() > 1
        && form.nth(1) instanceof Symbol name
        && name.getNamespace() == null) {
      globals.add(name.getName());
    }
    return form.count() > 2 && scan(form.nth(2), lexical, globals);
  }

  private void addDeclared(List<?> form, Set<String> globals) {
    for (int index = 1; index < form.count(); index++) {
      if (form.nth(index) instanceof Symbol name && name.getNamespace() == null) {
        globals.add(name.getName());
      }
    }
  }

  private boolean scanSet(List<?> form, Set<String> lexical, Set<String> globals) {
    if (form.count() > 1) {
      Object place = form.nth(1);
      if (place instanceof List<?> field
          && field.count() > 1
          && field.nth(0) instanceof Symbol operation
          && operation.getNamespace() == null
          && "field".equals(operation.getName())) {
        if (scan(field.nth(1), lexical, globals)) return true;
      } else if (scan(place, lexical, globals)) {
        return true;
      }
    }
    return form.count() > 2 && scan(form.nth(2), lexical, globals);
  }

  private boolean scanNamedValue(
      List<?> form, Set<String> lexical, Set<String> globals, String operation) {
    if (form.count() > 1
        && form.nth(1) instanceof Symbol name
        && name.getNamespace() == null) {
      addDefinitionNames(globals, operation, name.getName());
    }
    int index = 3;
    while (index < form.count()) {
      Object protocol = form.nth(index++);
      if (scan(protocol, lexical, globals)) return true;
      while (index < form.count() && form.nth(index) instanceof List<?> method) {
        if (scanMethod(method, lexical, globals)) return true;
        index++;
      }
    }
    return false;
  }

  private void addProtocolMethodNames(List<?> form, Set<String> globals) {
    for (int index = 2; index < form.count(); index++) {
      if (form.nth(index) instanceof List<?> method
          && method.count() > 0
          && method.nth(0) instanceof Symbol name
          && name.getNamespace() == null) {
        globals.add(name.getName());
      }
    }
  }

  private boolean scanExtendType(List<?> form, Set<String> lexical, Set<String> globals) {
    if (form.count() > 1 && scan(form.nth(1), lexical, globals)) return true;
    int index = 2;
    while (index < form.count()) {
      if (scan(form.nth(index++), lexical, globals)) return true;
      while (index < form.count() && form.nth(index) instanceof List<?> method) {
        if (scanMethod(method, lexical, globals)) return true;
        index++;
      }
    }
    return false;
  }

  private boolean scanMethod(List<?> method, Set<String> lexical, Set<String> globals) {
    if (method.count() < 2 || !isVector(method.nth(1))) return false;
    return scanFunctionBody(
        (ILinearType<?>) method.nth(1), method, 2, lexical, globals, false);
  }

  private boolean scanDefMulti(List<?> form, Set<String> lexical, Set<String> globals) {
    if (form.count() > 1
        && form.nth(1) instanceof Symbol name
        && name.getNamespace() == null) {
      globals.add(name.getName());
    }
    for (int index = 2; index < form.count(); index++) {
      Object value = form.nth(index);
      if (value instanceof String || value instanceof IMapType<?, ?>) continue;
      if (scan(value, lexical, globals)) return true;
    }
    return false;
  }

  private boolean scanDefMethod(List<?> form, Set<String> lexical, Set<String> globals) {
    if (form.count() > 1 && scan(form.nth(1), lexical, globals)) return true;
    if (form.count() > 2 && scan(form.nth(2), lexical, globals)) return true;
    if (form.count() > 3 && isVector(form.nth(3))) {
      return scanFunctionBody(
          (ILinearType<?>) form.nth(3), form, 4, lexical, globals, false);
    }
    return scanRange(form, 3, lexical, globals);
  }

  private boolean scanDot(List<?> form, Set<String> lexical, Set<String> globals) {
    if (form.count() > 1 && scan(form.nth(1), lexical, globals)) return true;
    for (int index = 2; index < form.count(); index++) {
      Object step = form.nth(index);
      if (step instanceof List<?> method) {
        if (scanRange(method, 1, lexical, globals)) return true;
      } else if (step instanceof ILinearType<?> vector) {
        for (Object value : vector) if (scan(value, lexical, globals)) return true;
      }
    }
    return false;
  }

  private boolean scanSyntaxQuote(
      Object value, Set<String> lexical, Set<String> globals) {
    if (value instanceof Symbol symbol) return needsFallback(symbol, lexical, globals);
    if (value instanceof List<?> list) {
      if (list.count() > 0
          && list.nth(0) instanceof Symbol operation
          && operation.getNamespace() == null
          && ("unquote".equals(operation.getName())
              || "unquote-splicing".equals(operation.getName()))) {
        return list.count() > 1 && scan(list.nth(1), lexical, globals);
      }
      for (Object item : list) if (scanSyntaxQuote(item, lexical, globals)) return true;
      return false;
    }
    if (value instanceof IMapType<?, ?> map) {
      for (Object entryValue : map) {
        if (!(entryValue instanceof Map.Entry<?, ?> entry)) continue;
        if (scanSyntaxQuote(entry.getKey(), lexical, globals)
            || scanSyntaxQuote(entry.getValue(), lexical, globals)) return true;
      }
      return false;
    }
    if (value instanceof ISetType<?> set) {
      for (Object item : set) if (scanSyntaxQuote(item, lexical, globals)) return true;
      return false;
    }
    if (value instanceof ILinearType<?> linear) {
      for (Object item : linear) if (scanSyntaxQuote(item, lexical, globals)) return true;
    }
    return false;
  }

  private boolean scanPatternDefaults(
      Object pattern, Set<String> lexical, Set<String> globals) {
    if (pattern instanceof IMapType<?, ?> map) {
      for (Object entryValue : map) {
        if (!(entryValue instanceof Map.Entry<?, ?> entry)) continue;
        Object key = entry.getKey();
        Object value = entry.getValue();
        if (key instanceof Keyword keyword
            && keyword.getNamespace() == null
            && "or".equals(keyword.getName())
            && value instanceof IMapType<?, ?> defaults) {
          for (Object defaultValue : defaults) {
            if (defaultValue instanceof Map.Entry<?, ?> defaultEntry
                && scan(defaultEntry.getValue(), lexical, globals)) return true;
          }
        } else if (scanPatternDefaults(value, lexical, globals)) {
          return true;
        }
      }
    } else if (pattern instanceof ILinearType<?> linear) {
      for (Object value : linear) {
        if (scanPatternDefaults(value, lexical, globals)) return true;
      }
    }
    return false;
  }

  private void addPatternBindings(Object pattern, Set<String> lexical) {
    if (pattern instanceof Symbol symbol) {
      if (symbol.getNamespace() == null && !"&".equals(symbol.getName())) {
        lexical.add(symbol.getName());
      }
      return;
    }
    if (pattern instanceof IMapType<?, ?> map) {
      for (Object entryValue : map) {
        if (!(entryValue instanceof Map.Entry<?, ?> entry)) continue;
        if (entry.getKey() instanceof Keyword keyword
            && keyword.getNamespace() == null
            && "or".equals(keyword.getName())) continue;
        addPatternBindings(entry.getKey(), lexical);
        addPatternBindings(entry.getValue(), lexical);
      }
      return;
    }
    if (pattern instanceof ILinearType<?> linear) {
      for (Object value : linear) addPatternBindings(value, lexical);
    }
  }

  private boolean needsFallback(
      Symbol symbol, Set<String> lexical, Set<String> globals) {
    if (symbol.getNamespace() != null) {
      return context.namespaceQualifierTargets(symbol.getNamespace(), "std.foundation")
          && FoundationFallbackDefinitions.defines(symbol.getName());
    }
    String name = symbol.getName();
    if (lexical.contains(name)
        || globals.contains(name)
        || "&".equals(name)
        || context.isSpecialSymbol(symbol)) return false;
    if (FoundationFallbackDefinitions.defines(name)) return true;
    return context.resolve(symbol) == null && context.resolveMacro(symbol) == null;
  }

  private void addDefinitionNames(Set<String> globals, String operation, String name) {
    globals.add(name);
    if ("defstruct".equals(operation) || "defmutable".equals(operation)) {
      globals.add("->" + name);
      globals.add("map->" + name);
    }
  }

  private boolean isVector(Object value) {
    return value instanceof ILinearType<?> linear && "[".equals(linear.startString());
  }

  private boolean topLevelForm(Object form, String name) {
    return form instanceof List<?> list
        && list.count() > 0
        && list.nth(0) instanceof Symbol operator
        && operator.getNamespace() == null
        && name.equals(operator.getName());
  }
}
