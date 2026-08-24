package hara.kernel.base;

import hara.kernel.flavor.NativeFlavorProvider;
import hara.kernel.flavor.NativeFlavorImportSpecs;
import hara.kernel.protocol.IEnv;
import hara.kernel.protocol.IRuntime;
import hara.lang.base.Eq;
import hara.lang.base.Ex;
import hara.lang.base.G;
import hara.lang.base.Iter;
import hara.lang.base.primitive.Array;
import hara.lang.data.Keyword;
import hara.lang.data.List;
import hara.lang.data.Symbol;
import hara.lang.data.Tuple;
import hara.lang.data.Vector;
import hara.lang.protocol.ILinearType;
import hara.lang.protocol.IMapType;
import hara.lang.protocol.IFn;
import hara.lang.protocol.IExInfo;
import hara.lang.protocol.ILookup;
import hara.lang.protocol.INth;
import hara.lang.protocol.IPair;

import java.util.Iterator;
import java.util.Map.Entry;

import static hara.kernel.builtin.BuiltinBasic.atomVolatile;
import static hara.kernel.builtin.BuiltinBasic.symbol;
import static hara.kernel.builtin.BuiltinCheck.isTruthy;
import static hara.kernel.builtin.BuiltinCollection.*;
import static hara.kernel.builtin.BuiltinStruct.list;
import hara.kernel.builtin.BuiltinCollection;
import hara.kernel.builtin.BuiltinRuntime;
import hara.kernel.builtin.BuiltinStruct;

@SuppressWarnings({"rawtypes", "unchecked"})
public interface Macro {

  public static class Recur {
    public final Object[] args;

    public Recur(Object[] args) {
      this.args = args;
    }
  }

  public interface Java {

    public static <R, AST> R dotExpr(IRuntime rt, Object o, AST cmd) {
      if (cmd instanceof Symbol) {
        return (R)
            nativeProvider(rt).readMember(o, G.display(cmd), ((RT.Instance) rt).nativeAccess());
      } else if (cmd instanceof List) {
        var l = (List) cmd;
        var method = l.peekFirst();
        return (R)
            nativeProvider(rt)
                .invokeMember(
                    o,
                    G.display(method),
                    Iter.toArray(Iter.map(Iter.drop(Iter.iter(l), 1), (expr) -> rt.eval(expr))),
                    ((RT.Instance) rt).nativeAccess());

      } else if (cmd instanceof Vector || cmd instanceof Tuple.Tup1) {
        var idx = rt.eval(((INth) cmd).nth(0));
        if (o.getClass().isArray()) {
          return (R) nativeProvider(rt).index(o, idx, ((RT.Instance) rt).nativeAccess());
        } else if (o instanceof ILookup) {
          return (R) BuiltinCollection.get((ILookup) o, idx);
        } else if (o instanceof Iterable) {
          return (R) BuiltinCollection.nth((Iterable) o, (long) idx);
        } else if (o instanceof String) {
          return (R) BuiltinCollection.nth((String) o, (long) idx);
        }
      }
      throw new Ex.Unsupported();
    }

    private static NativeFlavorProvider nativeProvider(IRuntime runtime) {
      if (!(runtime instanceof RT.Instance)) {
        throw new Ex.Runtime("Runtime does not support native flavors");
      }
      NativeFlavorProvider provider = ((RT.Instance) runtime).nativeProvider();
      if (provider == null) {
        throw new Ex.Runtime("Native interop requires an ns :flavor declaration");
      }
      return provider;
    }

    @Module.Fn(name = ".", complete = true, vargs = true, env = true)
    @Module.Var(control = true)
    public static <R, AST, ITR> R dotExpr(IEnv env, AST expr, AST cmd, ITR more) {
      Object o = Eval.eval(expr, env);
      return (R)
          Iter.reduce(
              Iter.iter(more),
              dotExpr(env.getRuntime(), o, cmd),
              (acc, c) -> dotExpr(env.getRuntime(), acc, c));
    }

    @Module.Fn(name = "new", complete = true, vargs = true, env = true)
    @Module.Var(control = true)
    public static <R, ITR> R newExpr(IEnv env, Symbol clsym, ITR args) {
      Object type = Eval.eval(clsym, env);

      // Evaluate args
      Object[] evalArgs = Iter.toArray(Iter.map(Iter.iter(args), arg -> Eval.eval(arg, env)));
      RT.Instance runtime = (RT.Instance) env.getRuntime();
      return (R) nativeProvider(runtime).construct(type, evalArgs, runtime.nativeAccess());
    }
  }

  public interface Control {

    @Module.Fn(name = "ns", complete = true, env = true, vargs = true)
    @Module.Var(macro = true)
    public static Object nsExpr(IEnv env, Symbol name, Object args) {
      if (env.getRuntime() instanceof RT.Instance) {
        RT.Instance rt = (RT.Instance) env.getRuntime();
        rt.setCurrentNs(name);

        Iterator flavorIt = Iter.iter(args);
        while (flavorIt.hasNext()) {
          Object arg = flavorIt.next();
          if (arg instanceof List) {
            List l = (List) arg;
            if (l.count() > 0
                && l.peekFirst() instanceof Keyword
                && "flavor".equals(((Keyword) l.peekFirst()).getName())) {
              processFlavor(rt, l);
            }
          }
        }

        Iterator it = Iter.iter(args);
        while (it.hasNext()) {
          Object arg = it.next();
          if (arg instanceof List) {
            List l = (List) arg;
            Object head = l.peekFirst();
            if (head instanceof Keyword) {
              String kwName = ((Keyword) head).getName();
              if ("import".equals(kwName)) {
                processImport(rt, l);
              } else if ("require".equals(kwName)) {
                processRequire(rt, l);
              } else if ("flavor".equals(kwName)) {
                // Applied in the first pass so clause order does not affect imports.
              } else {
                throw new Ex.Runtime("Unsupported ns option: " + kwName);
              }
            }
          }
        }
        return null;
      } else {
        throw new Ex.Runtime("Runtime does not support namespaces");
      }
    }

    @Module.Fn(name = "ns+", complete = true, env = true, vargs = true)
    @Module.Var(macro = true)
    public static Object nsPlusExpr(IEnv env, Object args) {
      if (!(env.getRuntime() instanceof RT.Instance)) {
        throw new Ex.Runtime("Runtime does not support namespaces");
      }
      Iterator it = Iter.iter(args);
      while (it.hasNext()) {
        Object clause = it.next();
        if (!(clause instanceof List)) {
          throw new Ex.Runtime("ns+ does not accept a namespace name");
        }
      }
      RT.Instance rt = (RT.Instance) env.getRuntime();
      nsExpr(env, rt.getCurrentNs().name, args);
      return null;
    }

    public static void processFlavor(RT.Instance rt, List l) {
      if (l.count() < 2 || !(l.nth(1) instanceof Keyword)) {
        throw new Ex.Runtime(":flavor expects a host keyword followed by import specs");
      }
      Keyword flavor = (Keyword) l.nth(1);
      if (flavor.getNamespace() != null) {
        throw new Ex.Runtime(":flavor expects an unqualified keyword");
      }
      if ("wasm".equals(flavor.getName())) {
        throw new Ex.Runtime(":wasm is not a host flavor; use :import for Wasm modules");
      }
      NativeFlavorProvider provider = rt._nativeFlavors.require(flavor.getName());
      java.util.ArrayList<Object> specifications = new java.util.ArrayList<>();
      for (int i = 2; i < l.count(); i++) {
        specifications.add(l.nth(i));
      }
      java.util.List<NativeFlavorImportSpecs.Spec> imports;
      try {
        imports = NativeFlavorImportSpecs.parse(specifications);
      } catch (IllegalArgumentException error) {
        throw new Ex.Runtime(error.getMessage());
      }
      java.util.LinkedHashMap<Symbol, Class> resolved = new java.util.LinkedHashMap<>();
      for (NativeFlavorImportSpecs.Spec specification : imports) {
        Object value = provider.resolveType(specification.typeName(), rt.nativeAccess());
        if (!(value instanceof Class<?> type)) {
          throw new Ex.Runtime(
              "JVM native import did not resolve to a class: " + specification.typeName());
        }
        Symbol local = Symbol.create(specification.localName());
        Class previous = resolved.putIfAbsent(local, type);
        if (previous != null && previous != type) {
          throw new Ex.Runtime("Native import already exists: " + specification.localName());
        }
      }
      rt.getCurrentNs().nativeFlavor = flavor.getName();
      rt.getCurrentNs().imports.clear();
      rt.getCurrentNs().imports.putAll(resolved);
    }

    public static void processImport(RT.Instance rt, List l) {
      throw new Ex.Runtime(":import is reserved for Wasm modules");
    }

    public static void processRequire(RT.Instance rt, List l) {
      Iterator it = Iter.drop(l.iterator(), 1);
      while (it.hasNext()) {
        Object spec = it.next();
        Symbol nsName = null;
        Symbol alias = null;

        if (spec instanceof Symbol) {
          // [my.lib] or just my.lib (if it works like that, usually vector)
          nsName = (Symbol) spec;
        } else if (spec instanceof Iterable) {
          // [my.lib :as l]
          Iterator specIt = Iter.iter(spec);
          if (specIt.hasNext()) {
            Object first = specIt.next();
            if (first instanceof Symbol) {
              nsName = (Symbol) first;
            }
            while (specIt.hasNext()) {
              Object next = specIt.next();
              if (next instanceof Keyword && ((Keyword) next).getName().equals("as")) {
                if (specIt.hasNext()) {
                  Object aliasObj = specIt.next();
                  if (aliasObj instanceof Symbol) {
                    alias = (Symbol) aliasObj;
                  }
                }
              }
            }
          }
        }

        if (nsName != null) {
          loadNamespace(rt, nsName);
          if (alias != null) {
            Namespace ns = rt.getNamespace(nsName);
            if (ns != null) {
              rt.getCurrentNs().aliases.put(alias, ns);
            }
          }
        }
      }
    }

    public static void loadNamespace(RT.Instance rt, Symbol nsName) {
      if (rt.getNamespace(nsName) != null) {
        return;
      }

      String path = nsName.getName().replace('.', '/') + ".hal";
      BuiltinRuntime.loadResource(rt, path);
    }

    @Module.Fn(name = "quote")
    @Module.Var(control = true)
    public static Object quoteExpr(Object expr) {
      return expr;
    }

    @Module.Fn(name = "syntax-quote")
    @Module.Var(macro = true)
    public static Object syntaxQuoteExpr(Object form) {
      return sqExpand(form);
    }

    public static Object sqExpand(Object form) {
      if (form instanceof Symbol) {
        return list(Array.objects(symbol("quote"), form));
      } else if (form instanceof List) {
        List l = (List) form;
        if (l.count() == 2 && Eq.eq(l.peekFirst(), Symbol.create("unquote"))) {
          return l.nth(1);
        }
        return sqExpandCollection(form);
      } else if (form instanceof ILinearType) {
        // Vectors, Tuples
        return sqExpandCollection(form);
      } else if (form instanceof IMapType) {
        return sqExpandMap((IMapType) form);
      } else if (form instanceof java.util.Map) {
        // Maps
        return sqExpandMap(((java.util.Map) form).entrySet());
      } else {
        return form;
      }
    }

    public static Object sqExpandMap(Iterable<Entry> entries) {
      java.util.List<Object> args = new java.util.ArrayList<>();
      args.add(symbol("hash-map"));

      Iterator<Entry> it = entries.iterator();
      while (it.hasNext()) {
        Entry e = it.next();
        args.add(sqExpand(e.getKey()));
        args.add(sqExpand(e.getValue()));
      }

      // Use vector to preserve order (List.into reverses), then convert to list
      return list(Iter.iter(args));
    }

    public static Object sqExpandCollection(Object form) {
      Iterator it = Iter.iter(form);

      // Determine if we should build a vector or a list
      boolean isVector =
          (form instanceof Vector || (form instanceof Tuple.Tup1 || form instanceof Tuple.Tup0));
      if (form instanceof Tuple) {
        isVector = true;
      }

      java.util.List<Object> parts = new java.util.ArrayList<>();

      while (it.hasNext()) {
        Object item = it.next();
        if (item instanceof List) {
          List l = (List) item;
          if (l.count() > 0 && Eq.eq(l.peekFirst(), Symbol.create("unquote"))) {
            parts.add(list(Array.objects(symbol("list"), l.nth(1))));
            continue;
          } else if (l.count() > 0 && Eq.eq(l.peekFirst(), Symbol.create("unquote-splicing"))) {
            parts.add(l.nth(1));
            continue;
          }
        }

        // Recursively syntax-quote the item
        parts.add(list(Array.objects(symbol("list"), sqExpand(item))));
      }

      Object concatExpr = list(Array.objects(symbol("concat")));
      for (Object part : parts) {
        concatExpr = ((List) concatExpr).conj(part);
      }
      // List.conj adds to the front, so we need to reverse or use a different
      // approach.
      // Actually, List.Standard.conj adds to the front.
      // So if we iterate parts and conj, we get (concat partN ... part1).
      // We want (concat part1 ... partN).
      // So we should iterate parts in reverse or just build the list from array.

      java.util.List<Object> concatArgs = new java.util.ArrayList<>();
      concatArgs.add(symbol("concat"));
      concatArgs.addAll(parts);
      concatExpr = list(Iter.iter(concatArgs));

      Object target =
          isVector ? list(Array.objects(symbol("vector"))) : list(Array.objects(symbol("list")));

      return list(Array.objects(symbol("into"), target, concatExpr));
    }

    @Module.Fn(name = "def", complete = true, env = true)
    @Module.Var(control = true)
    public static Var defExpr(IEnv env, Symbol sym, Object expr) {
      var val = Eval.eval(expr, env);
      RT.Instance<?> runtime = (RT.Instance<?>) env.getRuntime();
      String namespace = sym.getNamespace();
      if (namespace == null || "-".equals(namespace)) {
        namespace = runtime.getCurrentNs().name.getName();
      }
      String path = namespace + "/" + sym.getName();
      Var v = runtime.getObj(sym);
      if (v == null || !path.equals(v.pathString())) {
        v = new Var(path, val);
      } else {
        v.reset(val);
      }
      v.withMeta(sym.meta());
      runtime.setObj(sym, v);
      return v;
    }

    @Module.Fn(name = "do", complete = true, env = true, vargs = true)
    @Module.Var(control = true)
    public static <ITR> Object doExpr(IEnv env, ITR exprs) {
      return Iter.reduce(Iter.iter(exprs), null, (out, expr) -> Eval.eval(expr, env));
    }

    @Module.Fn(name = "fn", complete = true, env = true, vargs = true)
    @Module.Var(control = true)
    public static <ITR> IFn fnExpr(IEnv env, Object bindings, ITR args) {
      if (!(bindings instanceof hara.lang.data.List)) {
        if (!(bindings instanceof ILinearType)) {
          throw new Ex.Syntax("fn expects a parameter vector or arity clauses");
        }
        return new Env.FnEval(
            null, env.getRuntime(), env, (ILinearType) bindings, list(args).cons(symbol("do")));
      }
      java.util.List<Env.FnEval> functions = new java.util.ArrayList<>();
      Iterator clauses = Iter.concat(Iter.objects(bindings), Iter.iter(args));
      while (clauses.hasNext()) {
        Object clauseValue = clauses.next();
        if (!(clauseValue instanceof hara.lang.data.List)) {
          throw new Ex.Syntax("fn multi-arity clauses must be lists");
        }
        ILinearType clause = (ILinearType) clauseValue;
        if (clause.count() < 2 || !(clause.nth(0) instanceof ILinearType)) {
          throw new Ex.Syntax("fn clause expects parameters and a body");
        }
        functions.add(
            new Env.FnEval(
                null,
                env.getRuntime(),
                env,
                (ILinearType) clause.nth(0),
                list(Iter.drop(clause.iterator(), 1)).cons(symbol("do"))));
      }
      return new Env.FnMulti(functions);
    }

    @Module.Fn(name = "defn", complete = true, env = true, vargs = true)
    @Module.Var(control = true)
    public static <ITR> Var defnExpr(IEnv env, Symbol sym, ILinearType bindings, ITR body) {
      IFn function =
          new Env.FnEval(null, env.getRuntime(), env, bindings, list(body).cons(symbol("do")));
      Var var = (Var) new Var(sym.getName(), function).withMeta(sym.meta());
      env.getRuntime().setObj(sym, var);
      return var;
    }

    @Module.Fn(name = "let", complete = true, env = true, vargs = true)
    @Module.Var(control = true)
    public static <ITR> Object letExpr(IEnv env, ILinearType bindings, ITR body) {
      if (bindings.count() % 2 != 0) {
        throw new Ex.Runtime("let bindings must have an even number of forms");
      }

      Eval.LocalEnv localEnv = new Eval.LocalEnv(env);
      java.util.List<Object> values = new java.util.ArrayList<>();
      for (int i = 1; i < bindings.count(); i += 2) {
        values.add(Eval.eval(bindings.nth(i), env));
      }

      for (int i = 0; i < bindings.count(); i += 2) {
        Symbol symbol = (Symbol) bindings.nth(i);
        localEnv.addBinding(symbol, values.get(i / 2));
      }

      Object result = null;
      Iterator it = Iter.iter(body);
      while (it.hasNext()) {
        result = Eval.eval(it.next(), localEnv);
      }
      return result;
    }

    @Module.Fn(name = "if", complete = true, env = true)
    @Module.Var(control = true)
    public static <EXPR> Object ifExpr(IEnv env, EXPR check, EXPR then, EXPR otherwise) {
      Object val = Eval.eval(check, env);
      if (isTruthy(val)) {
        return Eval.eval(then, env);
      }
      return Eval.eval(otherwise, env);
    }

    @Module.Fn(name = "if", complete = true, env = true)
    @Module.Var(control = true)
    public static <EXPR> Object ifExpr(IEnv env, EXPR check, EXPR then) {
      return ifExpr(env, check, then, null);
    }

    @Module.Fn(name = "cond", complete = true, env = true, vargs = true)
    @Module.Var(control = true)
    public static <ITR> Object conjExpr(IEnv env, ITR pairs) {
      Iterator<IPair> branches = Iter.partitionPair(Iter.iter(pairs));
      IPair p = Iter.some(branches, (e) -> isTruthy(Eval.eval(e.getKey(), env)));
      return (p != null) ? Eval.eval(p.getValue(), env) : null;
    }

    @Module.Fn(name = "->", complete = true, vargs = true)
    @Module.Var(macro = true)
    public static <R, ITR> R threadFirst(Object any, ITR args) {
      return (R)
          Iter.reduce(
              Iter.iter(args),
              any,
              (acc, e) -> {
                if (!(e instanceof List.Standard)) {
                  return list(Array.objects(e, acc));
                } else {
                  var l = (List.Standard) e;
                  var changed = atomVolatile(false);
                  var ret =
                      list(
                          Iter.map(
                              Iter.iter(l),
                              (i) -> {
                                if (Eq.eq(i, Symbol.create("%"))) {
                                  changed.reset(true);
                                  return acc;
                                }
                                return i;
                              }));
                  if (changed.deref()) {
                    return ret;
                  } else {
                    return l.popFirst().cons(acc).cons(l.peekFirst());
                  }
                }
              });
    }

    @Module.Fn(name = "->>", complete = true, vargs = true)
    @Module.Var(macro = true)
    public static <R, ITR> R threadLast(Object any, ITR args) {
      return (R)
          Iter.reduce(
              Iter.iter(args),
              any,
              (acc, e) -> {
                if (!(e instanceof List.Standard)) {
                  return list(Array.objects(e, acc));
                } else {
                  var l = (List.Standard) e;
                  return l.conj(acc);
                }
              });
    }

    @Module.Fn(name = "try", complete = true, env = true, vargs = true)
    @Module.Var(control = true)
    public static <ITR> Object tryExpr(IEnv env, ITR body) {
      Iterator it = Iter.iter(body);
      Object res = null;
      List.Standard catches = List.Standard.EMPTY;
      List.Standard finallies = List.Standard.EMPTY;
      List.Standard exprs = List.Standard.EMPTY;

      while (it.hasNext()) {
        Object elem = it.next();
        if (elem instanceof List) {
          List l = (List) elem;
          Object head = l.peekFirst();
          if (head instanceof Symbol && ((Symbol) head).getName().equals("catch")) {
            catches = (List.Standard) catches.conj(l);
            continue;
          } else if (head instanceof Symbol && ((Symbol) head).getName().equals("finally")) {
            finallies = (List.Standard) finallies.conj(l);
            continue;
          }
        }
        exprs = (List.Standard) exprs.conj(elem);
      }

      try {
        Iterator eit = exprs.iterator();
        while (eit.hasNext()) {
          res = Eval.eval(eit.next(), env);
        }
      } catch (Throwable t) {
        Throwable cause = (t instanceof Exception) ? Reflect.getCauseOrElse((Exception) t) : t;

        Iterator cit = catches.iterator();
        boolean caught = false;
        while (cit.hasNext()) {
          List clause = (List) cit.next();
          Object selector = clause.nth(1);
          Symbol bindSym;
          int bodyStart;
          if (selector instanceof Symbol symbol
              && !symbol.getName().equals("Exception")
              && !symbol.getName().equals("Throwable")) {
            bindSym = symbol;
            selector = null;
            bodyStart = 2;
          } else {
            bindSym = (Symbol) clause.nth(2);
            bodyStart = 3;
          }

          if (catchMatches(cause, selector, env)) {
            Eval.LocalEnv localEnv = new Eval.LocalEnv(env);
            localEnv.addBinding(bindSym, cause);

            Iterator bit = Iter.drop(clause.iterator(), bodyStart);
            while (bit.hasNext()) {
              res = Eval.eval(bit.next(), localEnv);
            }
            caught = true;
            break;
          }
        }
        if (!caught) {
          throw Ex.Sneaky(cause);
        }
      } finally {
        Iterator fit = finallies.iterator();
        while (fit.hasNext()) {
          List clause = (List) fit.next();
          Iterator bit = Iter.drop(clause.iterator(), 1);
          while (bit.hasNext()) {
            Eval.eval(bit.next(), env);
          }
        }
      }
      return res;
    }

    private static boolean catchMatches(Throwable error, Object selector, IEnv env) {
      if (selector == null) return true;
      if (selector instanceof Symbol symbol) {
        Object type = Eval.eval(symbol, env);
        return type instanceof Class && ((Class) type).isInstance(error);
      }
      Object data = error instanceof IExInfo info ? info.getData() : null;
      Object code =
          data instanceof IMapType map ? map.lookup(Keyword.create("ex", "code")) : null;
      if (selector instanceof Keyword keyword) {
        if (keyword.getNamespace() == null) {
          throw new Ex.Syntax("catch error code must be a namespaced keyword");
        }
        return keyword.equals(code);
      }
      if (selector instanceof ILinearType selectors) {
        if (selectors.count() == 0) {
          throw new Ex.Syntax("catch error code vector must not be empty");
        }
        for (int index = 0; index < selectors.count(); index++) {
          Object candidate = selectors.nth(index);
          if (!(candidate instanceof Keyword keyword) || keyword.getNamespace() == null) {
            throw new Ex.Syntax(
                "catch error code vector must contain namespaced keywords");
          }
          if (candidate.equals(code)) return true;
        }
        return false;
      }
      throw new Ex.Syntax(
          "catch selector must be a namespaced keyword, a non-empty vector of namespaced keywords, or omitted");
    }

    @Module.Fn(name = "throw", complete = true, env = true)
    @Module.Var(control = true)
    public static Object throwExpr(IEnv env, Object expr) {
      Object ex = Eval.eval(expr, env);
      if (ex instanceof Throwable && ex instanceof IExInfo) {
        throw Ex.Sneaky((Throwable) ex);
      } else {
        throw new Ex.Runtime("throw expects an Exception value created by ex");
      }
    }

    @Module.Fn(name = "recur", complete = true, env = true, vargs = true)
    @Module.Var(control = true)
    public static <ITR> Object recurExpr(IEnv env, ITR args) {
      Object[] newVals = Iter.toArray(Iter.map(Iter.iter(args), arg -> Eval.eval(arg, env)));
      return new Recur(newVals);
    }

    @Module.Fn(name = "loop", complete = true, env = true, vargs = true)
    @Module.Var(control = true)
    public static <ITR> Object loopExpr(IEnv env, ILinearType bindings, ITR body) {
      if (bindings.count() % 2 != 0) {
        throw new Ex.Runtime("loop bindings must have an even number of forms");
      }

      Eval.LocalEnv localEnv = new Eval.LocalEnv(env);
      java.util.List<Object> values = new java.util.ArrayList<>();
      for (int i = 1; i < bindings.count(); i += 2) {
        values.add(Eval.eval(bindings.nth(i), env));
      }

      int bindingCount = (int) bindings.count() / 2;
      Symbol[] syms = new Symbol[bindingCount];
      for (int i = 0; i < bindings.count(); i += 2) {
        Symbol symbol = (Symbol) bindings.nth(i);
        syms[i / 2] = symbol;
        localEnv.addBinding(symbol, values.get(i / 2));
      }

      Object result = null;
      List bodyList =
          List.Standard.into(Iter.iter(body)); // materialize body to iterate multiple times

      while (true) {
        Iterator it = bodyList.iterator();
        boolean recurred = false;
        while (it.hasNext()) {
          result = Eval.eval(it.next(), localEnv);
          if (result instanceof Recur) {
            Recur r = (Recur) result;
            if (r.args.length != bindingCount) {
              throw new Ex.Arity(r.args.length, "loop requires " + bindingCount + " args");
            }
            for (int i = 0; i < bindingCount; i++) {
              localEnv.addBinding(syms[i], r.args[i]);
            }
            recurred = true;
            break; // break inner loop, continue outer while(true)
          }
        }
        if (!recurred) {
          return result;
        }
      }
    }
  }
}
