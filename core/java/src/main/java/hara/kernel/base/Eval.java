package hara.kernel.base;

import hara.kernel.protocol.IEnv;
import hara.kernel.protocol.IRuntime;
import hara.lang.base.Ex;
import hara.lang.base.Fn;
import hara.lang.base.G;
import hara.lang.base.Iter;
import hara.lang.data.Keyword;
import hara.lang.data.List;
import hara.lang.data.Symbol;
import hara.lang.protocol.ILinearType;
import hara.lang.protocol.IMapType;
import hara.lang.protocol.ILookup;
import hara.lang.protocol.IObjType;

import java.util.Iterator;
import java.util.Map.Entry;
import java.util.function.Function;

import static hara.kernel.builtin.BuiltinStruct.*;

@SuppressWarnings({"rawtypes", "unchecked"})
public interface Eval {

  // A simple environment for local bindings
  static class LocalEnv implements IEnv {
    private final IEnv parent;
    private final java.util.Map<Symbol, Object> bindings;

    public LocalEnv(IEnv parent) {
      this.parent = parent;
      this.bindings = new java.util.HashMap<>();
    }

    public void addBinding(Symbol sym, Object val) {
      bindings.put(sym, val);
    }

    @Override
    public Entry find(Object key) {
      if (bindings.containsKey(key)) {
        return new java.util.AbstractMap.SimpleEntry<>((Symbol) key, bindings.get(key));
      }
      return parent.find(key);
    }

    @Override
    public IEnv getParent() {
      return parent;
    }

    @Override
    public ILookup getMap() {
      return new ILookup<Symbol, Object>() {
        @Override
        public Entry<Symbol, Object> find(Symbol key) {
          if (bindings.containsKey(key)) {
            return new java.util.AbstractMap.SimpleEntry<>(key, bindings.get(key));
          }
          return null;
        }

        @Override
        public Iterator<Symbol> keys() {
          return bindings.keySet().iterator();
        }

        @Override
        public Iterator<Object> vals() {
          return bindings.values().iterator();
        }
      };
    }

    @Override
    public IRuntime getRuntime() {
      return parent.getRuntime();
    }

    @Override
    public Iterator keys() {
      return Iter.concat(bindings.keySet().iterator(), parent.keys());
    }

    @Override
    public Iterator vals() {
      return Iter.concat(bindings.values().iterator(), parent.vals());
    }
  }

  public static Object apply(Object f, Iterator it) {
    return Fn.toFn(f).apply(it);
  }

  public static Object apply(Object f, Object[] arr) {
    return Fn.toFn(f).apply(arr);
  }

  public static Object eval(Object ast, IEnv env) {
    if (ast instanceof Symbol) {
      Entry e = env.find(ast);
      if (e == null) {
        throw new Ex.Runtime("Cannot find symbol: " + G.display(ast));
      } else if (e.getValue() instanceof Var) {
        var v = (Var) e.getValue();
        if (v.isMacro() || v.isControl()) {
          throw new Ex.Runtime("Cannot return a macro/control value: " + G.display(ast));
        } else {
          return v.deref();
        }
      } else {
        return e.getValue();
      }
    } else if (ast instanceof List) {
      return evalList((List) ast, env);
    } else if (ast instanceof IMapType) {
      Function<Entry, Entry> mf = (e) -> pair(eval(e.getKey(), env), eval(e.getValue(), env));
      return hara.lang.data.Map.Standard.into(Iter.map(Iter.iter(ast), mf));
    } else if (ast instanceof ILinearType) {
      var v = (ILinearType) ast;
      var it = Iter.map(Iter.iter(ast), obj -> eval(obj, env));
      if (v.count() > 5) {
        return vector(it);
      } else {
        var arr = Iter.toArray(it);
        return tuple(arr);
      }
    } else {
      return ast;
    }
  }

  public static Object evalList(List ast, IEnv env) {
    if (ast.count() == 0) return ast;

    var fst = ast.peekFirst();

    Object f;
    if (fst instanceof Symbol) {
      Entry e = env.find(fst);
      if (e == null) {
        throw new Ex.Runtime("Cannot find symbol: " + G.display(fst));
      } else if (e.getValue() instanceof Var) {
        var v = (Var) e.getValue();
        if (v.isMacro()) {
          f = v.deref();
          var args = Iter.iter(ast.popFirst());
          var m = ((IObjType) f).meta();

          if (m != null) {
            if ((Boolean) ((ILookup) m).lookup(Keyword.create("env"), false)) {
              args = Iter.concat(Iter.objects(env), args);
            } else if ((Boolean) ((ILookup) m).lookup(Keyword.create("rt"), false)) {
              args = Iter.concat(Iter.objects(env.getRuntime()), args);
            }
          }
          var ret = apply(f, args);
          return eval(ret, env);
        } else if (v.isControl()) {
          f = v.deref();
          var args = Iter.iter(ast.popFirst());
          var m = ((IObjType) f).meta();

          if (m != null) {
            if ((Boolean) ((ILookup) m).lookup(Keyword.create("env"), false)) {
              args = Iter.concat(Iter.objects(env), args);
            } else if ((Boolean) ((ILookup) m).lookup(Keyword.create("rt"), false)) {
              args = Iter.concat(Iter.objects(env.getRuntime()), args);
            }
          }
          return apply(f, args);
        }
      } else {
        f = e.getValue();
      }
    }

    Iterator it = Iter.map(Iter.iter(ast), obj -> eval(obj, env));
    return apply(it.next(), Iter.toArray(it));
  }

  /*
   * public static String PATHS =
   * "(do (sys:add-paths [\"file:///Users/chris/.m2/repository/org/codehaus/janino/janino/3.1.2/janino-3.1.2.jar\" \"file:///Users/chris/.m2/repository/org/codehaus/janino/commons-compiler/3.1.2/commons-compiler-3.1.2.jar\"]) (class \"org.codehaus.janino.ExpressionEvaluator\"))"
   * ;
   *
   * public static void main(String [] args) {
   * G.prn(eval(Parser.LispReader.readString("(class \"java.nio.file.Path\")",
   * null),
   * new RT.RootEnv(null, new RT.Instance(null, "test"))));
   * }
   */
}
