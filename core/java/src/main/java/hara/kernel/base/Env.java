package hara.kernel.base;

import hara.kernel.protocol.IEnv;
import hara.kernel.protocol.IRuntime;
import hara.lang.base.Ex;
import hara.lang.base.G;
import hara.lang.base.Iter;
import hara.lang.base.fn.ReduceArray;
import hara.lang.base.fn.ReduceCompare;
import hara.lang.base.fn.ReduceInit;
import hara.lang.base.fn.ReduceSelf;
import hara.lang.base.primitive.Array;
import hara.lang.data.*;
import hara.lang.data.List;
import hara.lang.data.Map;
import hara.lang.data.Queue;
import hara.lang.data.Vector;
import hara.lang.protocol.ILinearType;
import hara.lang.data.types.ObjFn;
import hara.lang.protocol.*;

import java.lang.annotation.Annotation;
import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.lang.reflect.Parameter;
import java.util.*;
import java.util.Map.Entry;
import java.util.function.BiFunction;
import java.util.function.Function;
import java.util.function.Predicate;
import java.util.function.Supplier;

import static hara.kernel.builtin.BuiltinBasic.keyword;
import static hara.kernel.builtin.BuiltinBasic.symbol;
import static hara.kernel.builtin.BuiltinCollection.zipmap;
import static hara.kernel.builtin.BuiltinLambda.groupBy;
import static hara.kernel.builtin.BuiltinLambda.mapEntries;
import static hara.kernel.builtin.BuiltinStruct.hashMap;
import static hara.kernel.builtin.BuiltinStruct.pair;
import hara.kernel.builtin.BuiltinStruct;

@SuppressWarnings({"unchecked", "rawtypes"})
public interface Env {

  public interface Invoke {

    public static Predicate<Object> assignable(Method m, int i) {
      var cparam = boxed(m.getParameterTypes()[i]);
      return (arg) -> {
        if (arg == null) {
          return true;
        } else {
          return cparam.isAssignableFrom(arg.getClass());
        }
      };
    }

    public static Class<?> boxed(Class<?> type) {
      if (!type.isPrimitive()) return type;
      if (type == boolean.class) return Boolean.class;
      if (type == byte.class) return Byte.class;
      if (type == char.class) return Character.class;
      if (type == short.class) return Short.class;
      if (type == int.class) return Integer.class;
      if (type == long.class) return Long.class;
      if (type == float.class) return Float.class;
      if (type == double.class) return Double.class;
      return Void.class;
    }

    public static <R, ITR> R invokeMatch(List<Method> ms, ITR args, int len) {
      var arr = Array.toArray(args);
      var l =
          Iter.reduce(
              Iter.range(len),
              ms.iterator(),
              (it, i) ->
                  Iter.filter(it, (m) -> assignable(m, i.intValue()).test(arr[i.intValue()])));

      if (!l.hasNext()) {
        throw new Ex.Unsupported(
            ms.peekFirst().getName() + " cannot be applied to " + G.display(arr));
      }
      Method m = l.next();
      return invokeMethod(m, arr);
    }

    public static <R, ITR> R invokeMatch(List<Method> ms, Object arg) {

      var it = Iter.filter(ms.iterator(), (m) -> assignable(m, 0).test(arg));
      if (!it.hasNext()) {
        throw new Ex.Unsupported(ms.peekFirst().getName() + " cannot be applied to " + arg);
      }
      var method = it.next();
      return invokeMethod(method, new Object[] {arg});
    }

    public static <R> R invokeMethod(Method m, Object[] args) {
      try {
        return (R) m.invoke(null, args);
      } catch (IllegalAccessException | IllegalArgumentException | InvocationTargetException t) {
        throw Ex.Sneaky(t);
      }
    }
  }

  public interface S {

    public static Object reduceInit(Module.ReduceInit type) {
      switch (type) {
        case EMPTY_LIST:
          return List.Standard.EMPTY;
        case EMPTY_MAP:
          return Map.Standard.EMPTY;
        case EMPTY_QUEUE:
          return Queue.Standard.EMPTY;
        case EMPTY_VECTOR:
          return Vector.Standard.EMPTY;
        case NIL:
          return null;
        case ONE:
          return 1;
        case ZERO:
          return 0;
        case NEG_ONE:
          return -1;
        case TRUE:
          return true;
        case FALSE:
          return false;
        default:
          throw new Ex.Unsupported();
      }
    }

    public static IFn reduceFn(IFn f, IMetadata meta, Module.Reduce opts) {
      var init = reduceInit(opts.init());
      switch (opts.type()) {
        case ARRAY:
          return new ReduceArray(meta, init, f);
        case INIT:
          return new ReduceInit(meta, init, f);
        case SELF:
          return new ReduceSelf(meta, init, f);
        case COMPARE:
          return new ReduceCompare(meta, (Boolean) init, f);
        default:
          throw new Ex.Unsupported();
      }
    }

    public static Object getAnnotation(Annotation[] ann, Class c) {
      return Array.some((a) -> a.annotationType() == c, ann);
    }

    public static Object getAnnotation(Method m, Class c) {
      return getAnnotation(m.getAnnotations(), c);
    }

    public static Module.Fn fnOpts(Method m) {
      return (Module.Fn) getAnnotation(m, Module.Fn.class);
    }

    public static Module.Reduce rdOpts(Method m) {
      return (Module.Reduce) getAnnotation(m, Module.Reduce.class);
    }

    public static Module.Var varOpts(Method m) {
      return (Module.Var) getAnnotation(m, Module.Var.class);
    }
  }

  public static class FnEval<AST> extends ObjFn implements IFn {
    class FnEnv implements IEnv {
      final ILookup _map;
      final IEnv _parent;
      final IRuntime _rt;

      FnEnv(IEnv parent, ILookup map, IRuntime rt) {
        _parent = parent;
        _map = map;
        _rt = rt;
      }

      @Override
      public ILookup getMap() {
        return _map;
      }

      @Override
      public IEnv getParent() {
        return _parent;
      }

      @Override
      public IRuntime getRuntime() {
        return _rt;
      }
    }

    final AST _body;
    final ILinearType _params;
    final int _fixedArity;
    final Symbol _restParam;

    final IRuntime _rt;
    final IEnv _env;

    public FnEval(IMetadata meta, IRuntime rt, IEnv env, ILinearType params, AST body) {
      super(meta);
      _rt = rt;
      _env = env;
      _params = params;
      _body = body;
      int fixedArity = Math.toIntExact(params.count());
      Symbol restParam = null;
      for (int i = 0; i < params.count(); i++) {
        Object param = params.nth(i);
        if (param instanceof Symbol && "&".equals(((Symbol) param).getName())) {
          if (i != params.count() - 2 || !(params.nth(i + 1) instanceof Symbol)) {
            throw new Ex.Syntax("fn & must be followed by one final rest parameter");
          }
          fixedArity = i;
          restParam = (Symbol) params.nth(i + 1);
          break;
        }
      }
      _fixedArity = fixedArity;
      _restParam = restParam;
    }

    public void checkArgs(int size) {
      if ((_restParam == null && size != _fixedArity)
          || (_restParam != null && size < _fixedArity)) {
        String expected =
            _restParam == null ? Integer.toString(_fixedArity) : "at least " + _fixedArity;
        throw new Ex.Arity(size, "Expected " + expected + " arguments");
      }
    }

    public boolean accepts(int size) {
      return (_restParam == null && size == _fixedArity)
          || (_restParam != null && size >= _fixedArity);
    }

    @Override
    public Supplier getArg0() {
      checkArgs(0);
      return () -> invokeEval(new LinkedList());
    }

    @Override
    public Function getArg1() {
      checkArgs(1);
      return (arg) -> invokeEval(Arrays.asList(arg));
    }

    @Override
    public BiFunction getArg2() {
      checkArgs(2);
      return (a1, a2) -> invokeEval(Arrays.asList(a1, a2));
    }

    @Override
    public Function getArgN() {
      return (vargs) -> {
        java.util.List args = Iter.toArrayList(Iter.iter(vargs));
        checkArgs(args.size());
        return invokeEval(args);
      };
    }

    public Object invokeEval(java.util.List args) {
      try {
        var map = zipmap(Iter.take(_params.iterator(), _fixedArity), args.subList(0, _fixedArity));
        if (_restParam != null) {
          map = map.assoc(_restParam, BuiltinStruct.vector(args.subList(_fixedArity, args.size())));
        }
        return _rt.eval(_body, new FnEnv(_env, map, _rt));
      } catch (Throwable t) {
        throw Ex.Sneaky(t);
      }
    }
  }

  public static class FnMulti extends ObjFn implements IFn {
    final java.util.List<FnEval> _clauses;

    public FnMulti(java.util.List<FnEval> clauses) {
      super(null);
      _clauses = clauses;
    }

    private FnEval select(int size) {
      FnEval variadic = null;
      for (FnEval clause : _clauses) {
        if (clause._restParam == null && clause._fixedArity == size) return clause;
        if (clause._restParam != null
            && clause.accepts(size)
            && (variadic == null || clause._fixedArity > variadic._fixedArity)) {
          variadic = clause;
        }
      }
      if (variadic != null) return variadic;
      throw new Ex.Arity(size, "No matching function arity");
    }

    private Object invokeMulti(java.util.List args) {
      return select(args.size()).invokeEval(args);
    }

    @Override
    public Supplier getArg0() {
      return () -> invokeMulti(java.util.List.of());
    }

    @Override
    public Function getArg1() {
      return (arg) -> invokeMulti(java.util.List.of(arg));
    }

    @Override
    public BiFunction getArg2() {
      return (left, right) -> invokeMulti(java.util.List.of(left, right));
    }

    @Override
    public Function getArgN() {
      return (arguments) -> invokeMulti(Iter.toArrayList(Iter.iter(arguments)));
    }
  }

  public static class FnStaticLookup<R> extends ObjFn implements IFn<R, Object, Object> {

    final Map<Integer, List<Method>> _mths;

    public FnStaticLookup(IMetadata meta, Map<Integer, List<Method>> mths) {
      super(meta);
      _mths = mths;
    }

    @Override
    public Supplier<R> getArg0() {
      Method m = getMethods(0).peekFirst();
      return () -> Invoke.invokeMethod(m, Constant.EMPTY_ARRAY);
    }

    @Override
    public Function<Object, R> getArg1() {
      List<Method> mths = _mths.lookup(1);
      return (arg) -> Invoke.invokeMatch(mths, arg);
    }

    @Override
    public BiFunction<Object, Object, R> getArg2() {
      List<Method> mths = _mths.lookup(2);
      return (a0, a1) -> Invoke.invokeMatch(mths, Array.objects(a0, a1), 2);
    }

    @Override
    public Function<Object, R> getArgN() {
      return (vargs) -> {
        var args = Array.toArray(vargs);
        List<Method> mths = _mths.lookup(args.length);
        return Invoke.invokeMatch(mths, args, args.length);
      };
    }

    public List<Method> getMethods(int argc) {
      var l = _mths.lookup(argc);
      if (l == null) {
        throw new Ex.Arity(argc, "");
      }
      return l;
    }
  }

  public static class FnStaticVargs<R> extends ObjFn implements IFn<R, Object, Object> {

    final int _argc;
    final Method _m;

    public FnStaticVargs(IMetadata meta, Method m) {
      super(meta);
      _m = m;
      _argc = m.getParameterCount() - 1;
    }

    @Override
    public Supplier<R> getArg0() {
      if (_argc == 0) {
        return () -> Invoke.invokeMethod(_m, new Object[] {Constant.EMPTY_ARRAY});
      } else {
        throw new Ex.Arity(0, "Need at least " + _argc);
      }
    }

    @Override
    public Function<Object, R> getArg1() {
      if (_argc == 0) {
        return (arg) -> Invoke.invokeMethod(_m, new Object[] {new Object[] {arg}});
      } else if (_argc == 1) {
        return (arg) ->
            Invoke.invokeMethod(_m, new Object[] {new Object[] {arg, Constant.EMPTY_ARRAY}});
      } else {
        throw new Ex.Arity(1, "Need at least " + _argc);
      }
    }

    @Override
    public BiFunction<Object, Object, R> getArg2() {
      if (_argc == 0) {
        return (a0, a1) -> Invoke.invokeMethod(_m, new Object[] {new Object[] {a0, a1}});
      } else if (_argc == 1) {
        return (a0, a1) -> Invoke.invokeMethod(_m, new Object[] {a0, new Object[] {a1}});
      } else if (_argc == 2) {
        return (a0, a1) -> Invoke.invokeMethod(_m, new Object[] {a0, a1, Constant.EMPTY_ARRAY});
      } else {
        throw new Ex.Arity(2, "Need at least " + _argc);
      }
    }

    @Override
    public Function<Object, R> getArgN() {
      return (vargs) -> {
        var it = Iter.iter(vargs);
        var pargs = new ArrayList<Object>();
        for (int index = 0; index < _argc; index++) {
          if (!it.hasNext()) throw new Ex.Arity(index, "Need at least " + _argc);
          pargs.add(it.next());
        }
        var args = Iter.toArray(it);
        pargs.add(args);
        return Invoke.invokeMethod(_m, pargs.toArray());
      };
    }
  }

  public static HashMap<String, ArrayList<Method>> getMethods(
      Class cls, HashMap<String, ArrayList<Method>> map) {

    Function pairs =
        Iter.keep(
            (m) -> {
              Module.Fn v = S.fnOpts((Method) m);
              return (v != null && !v.helper()) ? BuiltinStruct.pair(v.name(), m) : null;
            });

    return Iter.groupBy(
        (Iterator) pairs.apply(Iter.iter(cls.getDeclaredMethods())),
        (Function) p -> ((IPair) p).getKey(),
        (Function) p -> ((IPair) p).getValue(),
        (HashMap) map);
  }

  public static HashMap<String, ArrayList<Method>> getMethods(Class cls) {
    return getMethods(cls, new HashMap());
  }

  public static IFn createFn(Entry<String, ArrayList<Method>> p) {

    ArrayList<Method> all = p.getValue();
    all.sort((m1, m2) -> Integer.compare(m1.getParameterCount(), m2.getParameterCount()));

    BiFunction<ArrayList<Method>, Predicate<Method>, ArrayList<Method>> filterList =
        (list, pred) -> Iter.toArrayList(Iter.filter(Iter.iter(list), pred));
    BiFunction<ArrayList<Method>, Function<Method, ?>, ArrayList> keepList =
        (list, f) -> Iter.toArrayList(Iter.keep(Iter.iter(list), f));

    var vArgs = filterList.apply(all, (m) -> S.fnOpts(m).vargs());
    var nArgs = filterList.apply(all, (m) -> !S.fnOpts(m).vargs());
    ArrayList<Module.Reduce> ropts = keepList.apply(all, (m) -> S.rdOpts(m));
    ArrayList<Module.Var> vopts = keepList.apply(all, (m) -> S.varOpts(m));

    boolean single = all.size() == 1;

    var meta =
        hashMap(
            Array.objects(
                keyword("name"), p.getKey(),
                keyword("rt"), S.fnOpts(all.get(0)).rt(),
                keyword("env"), S.fnOpts(all.get(0)).env(),
                keyword("doc"), S.fnOpts(all.get(0)).doc()));

    Function<Method, ILinearType> toArgs =
        (m) -> {
          Parameter[] params = m.getParameters();
          return Vector.Standard.into(
              Iter.map(Iter.iter(params), (pm) -> symbol(((Parameter) pm).getName())));
        };

    var arglists = Vector.Standard.into(Iter.map(Iter.iter(all), toArgs));
    meta = meta.assoc(keyword("arglists"), arglists);

    if (!ropts.isEmpty()) {
      meta = meta.assoc(keyword("reduce"), ropts.get(0));
    }
    if (!vopts.isEmpty()) {
      if (vopts.get(0).macro()) {
        meta = meta.assoc(keyword("macro"), true);
      }
      if (vopts.get(0).dynamic()) {
        meta = meta.assoc(keyword("dynamic"), true);
      }
      if (vopts.get(0).control()) {
        meta = meta.assoc(keyword("control"), true);
      }
    }

    if (vArgs.isEmpty()) {
      var options = groupBy((Function) (m) -> ((Method) m).getParameterCount(), all);
      var f = new FnStaticLookup(meta, options);
      if (ropts.isEmpty()) {
        return f;
      } else {
        return S.reduceFn(f, meta, ropts.get(0));
      }
    } else if (nArgs.isEmpty()) {
      if (vArgs.size() >= 1) {
        return new FnStaticVargs(meta, vArgs.get(0));
      } else {
        throw new Ex.Runtime("MORE than one VARG " + meta);
      }
    } else {
      throw new Ex.Runtime("TODO: " + meta);
    }
  }

  public static Map<Symbol, Var> loadStatic() {

    var raw = new HashMap();

    Array.reduce(
        (map, cls) -> getMethods(cls, map),
        raw,
        new Class[] {
          hara.kernel.builtin.BuiltinBasic.class,
          hara.kernel.builtin.BuiltinCheck.class,
          hara.kernel.builtin.BuiltinRef.class,
          hara.kernel.builtin.BuiltinCollection.class,
          hara.kernel.builtin.BuiltinInterop.class,
          hara.kernel.builtin.BuiltinLambda.class,
          hara.kernel.builtin.BuiltinOps.class,
          hara.kernel.builtin.BuiltinRuntime.class,
          hara.kernel.builtin.BuiltinStruct.class,
          hara.kernel.builtin.BuiltinTime.class,
          hara.kernel.builtin.BuiltinNamespace.class,
          hara.kernel.builtin.BuiltinUtil.class
        });

    Array.reduce((map, cls) -> getMethods(cls, map), raw, Macro.class.getDeclaredClasses());

    Map<Symbol, Var> fns =
        mapEntries(
            (Function)
                (obj) -> {
                  var p = (Entry) obj;
                  var k = (String) p.getKey();
                  var f = createFn(p);
                  return pair(symbol(k), new Var(k, f).withMeta(((IObjType) f).meta()));
                },
            raw);

    return fns;
  }

  /*
   * public static void main(String[] args) {
   * var map = loadStatic();
   * G.prn(map);
   * }
   */
}
