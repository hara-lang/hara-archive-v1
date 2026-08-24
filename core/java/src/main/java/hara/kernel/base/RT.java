package hara.kernel.base;

import hara.kernel.NativeMode;
import hara.kernel.builtin.BuiltinInterop;
import hara.kernel.flavor.NativeCapability;
import hara.kernel.flavor.NativeFlavorAccess;
import hara.kernel.flavor.NativeFlavorProvider;
import hara.kernel.flavor.NativeFlavorRegistry;
import hara.kernel.jvm.JvmFlavorProvider;
import hara.kernel.jvm.JvmRuntimeAccess;
import hara.kernel.jvm.JvmSetFunction;
import hara.kernel.protocol.IEnv;
import hara.kernel.protocol.IRuntime;
import hara.lang.base.Ex;
import hara.lang.base.Iter;
import hara.lang.base.primitive.Array;
import hara.lang.data.*;
import hara.lang.protocol.IMapType;
import hara.lang.protocol.*;

import java.net.MalformedURLException;
import java.net.URL;
import java.net.URLClassLoader;
import java.util.Arrays;
import java.util.EnumSet;
import java.util.HashSet;
import java.util.Iterator;
import java.util.LinkedHashSet;
import java.util.Map.Entry;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.atomic.AtomicReference;
import java.util.function.Function;

import hara.kernel.builtin.BuiltinLambda;
import hara.kernel.builtin.BuiltinStruct;

@SuppressWarnings("unchecked")
public interface RT {

  @SuppressWarnings("rawtypes")
  public class SubLoader extends URLClassLoader {

    final ConcurrentHashMap<String, Class> _cache;

    public SubLoader(URL[] urls, ConcurrentHashMap<String, Class> cache) {
      super(urls, ClassLoader.getSystemClassLoader());
      _cache = cache;
    }

    @Override
    public void addURL(URL url) {
      throw new Ex.Unsupported();
    }

    @Override
    public synchronized Class<?> findClass(String name) throws ClassNotFoundException {
      try {
        Class c = super.findClass(name);
        _cache.put(name, c);
        return c;

      } catch (Throwable t) {
        return null;
      }
    }
  }

  @SuppressWarnings("rawtypes")
  public class Loader extends URLClassLoader implements IWatch<Loader, Class> {

    static final URL[] EMPTY_URLS = new URL[] {};

    final HashSet<URL> _urls = new HashSet();
    final AsSet<URL> _urls_facade = new AsSet<>(_urls);
    final ConcurrentHashMap<String, Class> _cache = new ConcurrentHashMap();
    final AsMap<String, Class> _cache_facade = new AsMap(_cache);
    final ClassLoader _parent;

    public Loader() {
      this(ClassLoader.getSystemClassLoader());
    }

    public Loader(ClassLoader parent) {
      super(EMPTY_URLS, parent);
      _parent = parent;
    }

    @SuppressWarnings("resource")
    @Override
    protected synchronized Class<?> loadClass(String name, boolean resolve)
        throws ClassNotFoundException {
      Class c = null;

      if (c == null) {
        c = _cache.get(name);
        // if(c != null) G.prn("CACHE", c);
      }

      if (c == null) {
        try {
          c = findLoadedClass(name);
          // if(c != null) G.prn("LOADER", c);
        } catch (Throwable t) {
        }
      }

      if (c == null) {
        c = new SubLoader(getURLs(), _cache).findClass(name);
        // if(c != null) G.prn("PATH", c);
      }

      if (c == null) {
        c = super.loadClass(name, true);
      }
      if (resolve) resolveClass(c);
      return c;
    }

    @Override
    public void addURL(URL url) {
      NativeMode.requireDisabled("classpath mutation");
      super.addURL(url);
      _urls.add(url);
    }

    @Override
    public URL[] getURLs() {
      return _urls.toArray(EMPTY_URLS);
    }

    public AsMap<String, Class> getCache() {
      return _cache_facade;
    }

    public AsSet<URL> getPaths() {
      return _urls_facade;
    }
  }

  public class RootEnv<AST> implements IEnv<Symbol, Var> {

    final IEnv<Symbol, Var> _parent;
    final IRuntime<AST, Symbol, Var> _rt;
    final Map<Symbol, Var> _methods;

    public RootEnv(IEnv<Symbol, Var> parent, IRuntime<AST, Symbol, Var> rt) {
      _parent = parent;
      _rt = rt;
      _methods = loadMethods(rt);
    }

    @Override
    public IRuntime getRuntime() {
      return _rt;
    }

    @SuppressWarnings("rawtypes")
    public Map<Symbol, Var> loadMethods(IRuntime<AST, Symbol, Var> rt) {
      return BuiltinLambda.mapVals(
          (Function)
              (obj) -> {
                var v = (Var) obj;
                if ((Boolean) Keyword.create("rt").invoke(v.meta())) {
                  v.reset(BuiltinLambda.partial(v.deref(), Array.objects(rt)));
                }
                return v;
              },
          Env.loadStatic());
    }

    @Override
    public IEnv<Symbol, Var> getParent() {
      return _parent;
    }

    @Override
    public Map<Symbol, Var> getMap() {
      return _methods;
    }

    @Override
    public Iterator<Symbol> keys() {
      return Iter.map(_methods.iterator(), e -> e.getKey());
    }

    @Override
    public Iterator<Var> vals() {
      return Iter.map(_methods.iterator(), e -> e.getValue());
    }
  }

  @SuppressWarnings("rawtypes")
  public class ClassEnv implements IEnv<Symbol, Object> {

    final IRuntime _rt;
    final ConcurrentHashMap<String, Class> _alias = new ConcurrentHashMap();
    public final AsMap<String, Class> _alias_facade = new AsMap<>(_alias);

    public ClassEnv(IRuntime rt) {
      _rt = rt;
    }

    @Override
    public IRuntime getRuntime() {
      return _rt;
    }

    @Override
    public Entry<Symbol, Object> find(Symbol sym) {
      if (!(_rt instanceof Instance)
          || ((Instance) _rt).getCurrentNs() == null) {
        return null;
      }
      if (sym.getNamespace() == null) {
        var s = sym.getName();
        Class c = null;
        if (_rt instanceof Instance) {
          Namespace ns = ((Instance) _rt).getCurrentNs();
          if (ns != null) c = ns.imports.get(Symbol.create(s));
        }

        if (c == null) {
          c = _alias.getOrDefault(s, null);
        }
        return (c != null) ? BuiltinStruct.pair(sym, c) : null;
      } else {
        var s = sym.getNamespace();
        Class c = null;

        if (_rt instanceof Instance) {
          Namespace ns = ((Instance) _rt).getCurrentNs();
          if (ns != null) c = ns.imports.get(Symbol.create(s));
        }

        if (c == null) {
          c = _alias.getOrDefault(s, null);
        }
        if (c != null) {

          try {
            return BuiltinStruct.pair(sym, BuiltinInterop.invokeGetStatic(c, sym.getName()));
          } catch (Throwable t) {
          }

          try {
            return BuiltinStruct.pair(sym, BuiltinInterop.invokeFn(c, sym.getName()));
          } catch (Throwable t) {
          }

          throw new Ex.Runtime("No method or field found: " + sym.getName());
        }
        return null;
      }
    }

    @Override
    public IEnv getParent() {
      return null;
    }

    @Override
    public ILookup getMap() {
      throw new Ex.Unsupported();
    }

    public Class addAlias(Symbol sym, Class c) {
      var s = sym.getName();
      var p = _alias.getOrDefault(s, null);
      _alias.put(sym.getName(), c);
      return p;
    }

    public Class removeAlias(Symbol sym) {
      var s = sym.getName();
      var c = _alias.getOrDefault(s, null);
      _alias.remove(s);
      return c;
    }
  }

  public class UserEnv<AST> implements IEnv<Symbol, Var> {

    final IEnv<Symbol, Var> _parent;
    public final ConcurrentHashMap<Symbol, Namespace> _namespaces;
    final AtomicReference<Namespace> _currentNs;
    public final ClassEnv _class;

    @SuppressWarnings("rawtypes")
    public UserEnv(IEnv<Symbol, Var> parent, IRuntime rt) {
      _parent = parent;
      _namespaces = new ConcurrentHashMap<>();
      Namespace userNs = new Namespace(Symbol.create("user"));
      _namespaces.put(userNs.name, userNs);
      _currentNs = new AtomicReference<>(userNs);
      _class = new ClassEnv(rt);
    }

    @Override
    public IRuntime getRuntime() {
      return getParent().getRuntime();
    }

    @Override
    public IEnv<Symbol, Var> getParent() {
      return _parent;
    }

    @Override
    public ILookup<Symbol, Var> getMap() {
      return new AsMap<>(_currentNs.get().mappings);
    }

    @Override
    public Iterator<Symbol> keys() {
      return Iter.concat(
          Iter.map(_currentNs.get().mappings.entrySet().iterator(), e -> e.getKey()),
          _parent.keys());
    }

    @Override
    public Iterator<Var> vals() {
      return Iter.concat(
          Iter.map(_currentNs.get().mappings.entrySet().iterator(), e -> e.getValue()),
          _parent.vals());
    }

    @Override
    @SuppressWarnings("rawtypes")
    public Entry find(Symbol sym) {
      if (sym.getNamespace() != null) {
        String nsName = sym.getNamespace();
        Namespace ns =
            "-".equals(nsName) ? _currentNs.get() : _namespaces.get(Symbol.create(nsName));
        if (ns == null) {
          Namespace curr = _currentNs.get();
          Namespace aliased = curr.aliases.get(Symbol.create(nsName));
          if (aliased != null) ns = aliased;
        }

        if (ns != null) {
          Var v = ns.mappings.get(Symbol.create(sym.getName()));
          if (v != null) return BuiltinStruct.pair(sym, v);
        }
      } else {
        Var v = _currentNs.get().mappings.get(sym);
        if (v != null) return BuiltinStruct.pair(sym, v);
      }

      Entry e = _class.find(sym);
      if (e == null) e = _parent.find(sym);
      return e;
    }

    public Var getObj(Symbol key) {
      if (key.getNamespace() != null) {
        String nsName = key.getNamespace();
        Namespace ns =
            "-".equals(nsName) ? _currentNs.get() : _namespaces.get(Symbol.create(nsName));
        if (ns == null) {
          Namespace curr = _currentNs.get();
          Namespace aliased = curr.aliases.get(Symbol.create(nsName));
          if (aliased != null) ns = aliased;
        }
        if (ns != null) {
          return ns.mappings.get(Symbol.create(key.getName()));
        }
        return null;
      }
      return _currentNs.get().mappings.get(key);
    }

    public Var setObj(Symbol key, Var val) {
      if (key.getNamespace() != null) {
        String nsName = key.getNamespace();
        Namespace ns = _namespaces.get(Symbol.create(nsName));
        if (ns == null) {
          throw new Ex.Runtime("Namespace not found: " + nsName);
        }
        ns.mappings.put(Symbol.create(key.getName()), val);
      } else {
        _currentNs.get().mappings.put(key, val);
      }
      return val;
    }

    public ConcurrentHashMap<Symbol, Namespace> getNamespaces() {
      return _namespaces;
    }
  }

  @SuppressWarnings("rawtypes")
  public static class PrefixStream extends java.io.PrintStream {
    public final String prefix;

    public PrefixStream(java.io.OutputStream out, String prefix) {
      super(out);
      this.prefix = prefix;
    }

    @Override
    public void println(String x) {
      super.println(prefix + x);
    }

    @Override
    public void println(Object x) {
      super.println(prefix + x);
    }
  }

  public class Instance<AST> implements IRuntime<AST, Symbol, Var> {

    public static final ThreadLocal<Instance> CURRENT = new ThreadLocal<>();

    public final IContext _root;
    public final String _key;
    public final Loader _loader;
    public final RootEnv<AST> _rootEnv;
    public final UserEnv<AST> _userEnv;
    public final NativeFlavorRegistry _nativeFlavors;
    public final NativeFlavorAccess _nativeAccess;
    public final ThreadLocal<List<IEnv<Symbol, Var>>> _stack;
    public final java.io.PrintStream out;

    public Instance(IContext root, String key) {
      this(root, key, EnumSet.of(NativeCapability.REFLECTION));
    }

    public Instance(IContext root, String key, java.util.Set<NativeCapability> nativeCapabilities) {
      _root = root;
      _key = key;
      this.out = new PrefixStream(System.out, "[" + key + "] ");
      _loader = new Loader();
      _nativeFlavors = new NativeFlavorRegistry().register(JvmFlavorProvider.INSTANCE);
      _nativeAccess = new JvmRuntimeAccess(_loader, nativeCapabilities);
      _rootEnv = new RootEnv<>(null, this);
      _userEnv = new UserEnv<>(_rootEnv, this);
      installJvmLibraries();
      _stack =
          new ThreadLocal() {
            @Override
            protected List<IEnv<Symbol, Var>> initialValue() {
              return BuiltinStruct.list(Array.objects(_userEnv));
            }
          };
    }

    private void installJvmLibraries() {
      Namespace jvm = addNamespace(Symbol.create("hara.native.jvm"));
      jvm.mappings.put(Symbol.create("set!"), new Var("set!", new JvmSetFunction(this)));
      addNamespace(Symbol.create("hara.native.jvm.reflect"));
      addNamespace(Symbol.create("hara.native.jvm.classpath"));
      JvmNativeLibraries.install(this);
    }

    public Namespace getCurrentNs() {
      return _userEnv._currentNs.get();
    }

    public Namespace setCurrentNs(Symbol name) {
      Namespace ns = _userEnv._namespaces.get(name);
      if (ns == null) {
        ns = new Namespace(name);
        _userEnv._namespaces.put(name, ns);
      }
      _userEnv._currentNs.set(ns);
      return ns;
    }

    public Namespace getNamespace(Symbol name) {
      return _userEnv._namespaces.get(name);
    }

    public Namespace addNamespace(Symbol name) {
      Namespace ns = new Namespace(name);
      _userEnv._namespaces.putIfAbsent(name, ns);
      return _userEnv._namespaces.get(name);
    }

    public java.util.List<String> currentSymbolNames() {
      LinkedHashSet<String> names = new LinkedHashSet<>();
      Iterator<Symbol> visible = _userEnv.keys();
      while (visible.hasNext()) names.add(visible.next().display());
      Namespace current = getCurrentNs();
      if (current != null) {
        current.imports.forEach(
            (symbol, type) -> {
              names.add(symbol.display());
              Arrays.stream(type.getFields())
                  .filter(field -> java.lang.reflect.Modifier.isStatic(field.getModifiers()))
                  .forEach(field -> names.add(symbol.display() + "/" + field.getName()));
              Arrays.stream(type.getMethods())
                  .filter(method -> java.lang.reflect.Modifier.isStatic(method.getModifiers()))
                  .forEach(method -> names.add(symbol.display() + "/" + method.getName()));
            });
      }
      _userEnv._namespaces.forEach(
          (namespaceName, namespace) -> {
            if (!namespaceName.getName().startsWith("hara.native.")) return;
            namespace
                .mappings
                .keySet()
                .forEach(symbol -> names.add(namespaceName.display() + "/" + symbol.display()));
          });
      java.util.ArrayList<String> result = new java.util.ArrayList<>(names);
      result.sort(
          (left, right) -> {
            boolean leftPublic = isPublicCompletionSymbol(left);
            boolean rightPublic = isPublicCompletionSymbol(right);
            if (leftPublic != rightPublic) return leftPublic ? -1 : 1;
            return left.compareTo(right);
          });
      return result;
    }

    private boolean isPublicCompletionSymbol(String name) {
      Var variable = getObj(Symbol.create(name));
      if (variable == null) return false;
      return Boolean.TRUE.equals(Keyword.create("public").invoke(variable.meta(), false));
    }

    @Override
    public Object call(Object... args) {
      return _root.call(args);
    }

    @Override
    public IContext getRoot() {
      return _root;
    }

    public NativeFlavorProvider nativeProvider() {
      Namespace namespace = getCurrentNs();
      String flavor = namespace == null ? null : namespace.nativeFlavor;
      return _nativeFlavors.require(flavor == null ? "jvm" : flavor);
    }

    public NativeFlavorAccess nativeAccess() {
      return _nativeAccess;
    }

    @Override
    public Loader classLoader() {
      return _loader;
    }

    @Override
    public IMapType<String, Class> classCache() {
      return _loader.getCache();
    }

    @Override
    public Object eval(AST input) {
      Instance prev = CURRENT.get();
      CURRENT.set(this);
      try {
        Thread.currentThread().setContextClassLoader(_loader);
        return Eval.eval(input, _stack.get().peekFirst());
      } finally {
        CURRENT.set(prev);
      }
    }

    @Override
    public Object eval(AST input, IEnv env) {
      try {
        _stack.set((List) _stack.get().pushFirst(env));
        return eval(input);
      } finally {
        _stack.set((List) _stack.get().popFirst());
      }
    }

    @Override
    public AST readString(String input) {
      return (AST) Parser.LispReader.readString(input, null);
    }

    @Override
    public IFn findFn(Class cls, String name) {
      return null;
    }

    @Override
    public IFn findFn(Class cls, String name, int args) {
      return null;
    }

    @Override
    public IEnv getEnv() {
      return _userEnv;
    }

    @Override
    public Var getObj(Symbol key) {
      return _userEnv.getObj(key);
    }

    @Override
    public Var setObj(Symbol key, Var value) {
      return _userEnv.setObj(key, value);
    }

    @Override
    public Class classFor(String name) {
      try {
        return _loader.loadClass(name, true);
      } catch (ClassNotFoundException t) {
        try {
          return _loader.loadClass("java.lang." + name, true);
        } catch (ClassNotFoundException t2) {
          return null;
        }
      }
    }

    @Override
    public IColl<URL> pathAdd(URL url) {
      _loader.addURL(url);
      return _loader.getPaths();
    }

    @Override
    public IColl<URL> pathAdd(String[] paths) {
      NativeMode.requireDisabled("classpath mutation");
      Array.toIter(paths)
          .forEachRemaining(
              (path) -> {
                try {
                  _loader.getPaths().conj(new URL(path));
                } catch (MalformedURLException t) {
                  throw Ex.Sneaky(t);
                }
              });
      return _loader.getPaths();
    }

    @Override
    public IColl<URL> pathRemove(String[] paths) {
      NativeMode.requireDisabled("classpath mutation");
      Array.toIter(paths)
          .forEachRemaining(
              (path) -> {
                try {
                  _loader.getPaths().dissoc(new URL(path));
                } catch (MalformedURLException t) {
                  throw Ex.Sneaky(t);
                }
              });
      return _loader.getPaths();
    }

    @Override
    public Class aliasAdd(Symbol key, Class v) {
      return _userEnv._class.addAlias(key, v);
    }

    @Override
    public Class aliasRemove(Symbol key) {
      return _userEnv._class.removeAlias(key);
    }

    @Override
    public AsMap<String, Class> aliasCache() {
      return _userEnv._class._alias_facade;
    }

    @Override
    public IColl<URL> pathCache() {
      return _loader.getPaths();
    }
  }
}
