package hara.kernel.builtin;

import hara.lang.base.Ex;
import hara.kernel.base.Module;
import hara.kernel.base.Var;
import hara.kernel.protocol.IRuntime;
import hara.lang.base.Fn;
import hara.lang.base.Iter;
import hara.lang.base.primitive.Array;
import hara.lang.data.*;
import hara.lang.protocol.IMapType;
import hara.lang.protocol.*;

import java.net.MalformedURLException;
import java.net.URL;
import java.util.Iterator;
import java.util.Map.Entry;

@SuppressWarnings({"unchecked", "rawtypes"})
public interface BuiltinRuntime {
  @Module.Fn(name = "var:create", rt = true)
  public static Var varCreate(IRuntime rt, Symbol sym) {
    return (Var) rt.setObj(sym, new Var(sym.pathString(), null));
  }

  @Module.Fn(name = "var:get", rt = true)
  public static Var varGet(IRuntime rt, Symbol sym) {
    return (Var) rt.getObj(sym);
  }

  @Module.Fn(name = "var:set", rt = true)
  public static Var varSet(IRuntime rt, Symbol sym, Object val) {
    Var v = (Var) rt.getObj(sym);
    if (v == null) {
      v = new Var(sym.pathString(), val);
      rt.setObj(sym, v);
    } else {
      v.reset(val);
    }
    return v;
  }

  @Module.Fn(name = "var:list", rt = true)
  public static Iterator varList(IRuntime rt) {
    return rt.getEnv().vals();
  }

  @Module.Fn(name = "eval", rt = true)
  public static <AST> Object eval(IRuntime rt, AST input) {
    return rt.eval(input);
  }

  @Module.Fn(name = "read-string", rt = true)
  public static <AST> AST readString(IRuntime<AST, ?, ?> rt, String input) {
    return rt.readString(input);
  }

  @Module.Fn(name = "load-resource", rt = true)
  public static Object loadResource(IRuntime rt, String path) {
    if (path == null || (!path.endsWith(".hal") && !path.endsWith(".hrl"))) {
      throw new Ex.Runtime("load-resource expects a .hal or .hrl resource path");
    }
    java.io.InputStream stream = rt.classLoader().getResourceAsStream(path);
    if (stream == null) {
      throw new Ex.Runtime("Could not find .hal or .hrl resource: " + path);
    }
    try (stream) {
      String source = new String(stream.readAllBytes(), java.nio.charset.StandardCharsets.UTF_8);
      hara.kernel.base.Reader reader =
          new hara.kernel.base.Reader(new java.io.StringReader(source));
      Object eof = new Object();
      Object result = null;
      while (true) {
        Object form = hara.kernel.base.Parser.LispReader.read(reader, false, eof, false, null);
        if (form == eof) return result;
        result = rt.eval(form);
      }
    } catch (java.io.IOException error) {
      throw Ex.Sneaky(error);
    }
  }

  @Module.Fn(name = "require", rt = true)
  public static Object require(IRuntime rt, String path) {
    loadResource(rt, path);
    return null;
  }

  @Module.Fn(name = "ctl", vargs = true, rt = true)
  public static <ITR> Object ctl(IRuntime rt, ITR args) {
    return rt.getRoot().call(Array.toArray(args));
  }

  @Module.Fn(name = "sys:path", rt = true)
  public static IColl<URL> sysPath(IRuntime rt) {
    return rt.pathCache();
  }

  @Module.Fn(name = "sys:path-add", rt = true)
  public static <ITR> IColl<String> sysPathAdd(IRuntime rt, ITR paths) {
    return rt.pathAdd((String[]) Iter.toArray(Iter.iter(paths), String.class));
  }

  @Module.Fn(name = "sys:path-remove", rt = true)
  public static <ITR> IColl<String> sysPathRemove(IRuntime rt, ITR paths) {
    return rt.pathRemove((String[]) Iter.toArray(Iter.iter(paths), String.class));
  }

  @Module.Fn(name = "sys:path-purge", rt = true)
  public static IColl<String> sysPathPurge(IRuntime rt) {
    return (IColl) rt.pathCache().empty();
  }

  @Module.Fn(name = "sys:globals", rt = true)
  public static IFind sysGlobals(IRuntime rt) {
    return rt.getEnv().getMap();
  }

  @Module.Fn(name = "sys:loader", rt = true)
  public static ClassLoader sysloader(IRuntime rt) {
    return rt.classLoader();
  }

  @Module.Fn(name = "sys:root", rt = true)
  public static IContext sysRoot(IRuntime rt) {
    return rt.getRoot();
  }

  @Module.Fn(name = "sys:call", vargs = true, rt = true)
  public static <ITR> Object sysCall(IRuntime rt, ITR inputs) {
    return rt.call(Array.toArray(inputs));
  }

  @Module.Fn(name = "sys:alias-add", rt = true)
  public static Class sysAddAlias(IRuntime rt, Symbol sym, Class c) {
    return rt.aliasAdd(sym, c);
  }

  @Module.Fn(name = "sys:alias-remove", rt = true)
  public static Class sysRemoveAlias(IRuntime rt, Symbol sym) {
    return rt.aliasRemove(sym);
  }

  @Module.Fn(name = "sys:alias-purge", rt = true)
  public static IColl sysAliasPurge(IRuntime rt, Symbol sym) {
    return (IColl) rt.aliasCache().empty();
  }

  @Module.Fn(name = "sys:alias", rt = true)
  public static IColl sysListAlias(IRuntime rt) {
    return rt.aliasCache();
  }

  @Module.Fn(name = "sys:import", vargs = true, rt = true)
  public static <ITR> Class[] sysImport(IRuntime rt, ITR inputs) {
    Iterator<Entry> it = Iter.partitionPair(Iter.iter(inputs));
    return (Class[])
        Iter.toArray(
            (Iterator) Iter.map(it, (p) -> rt.aliasAdd(p.getKey(), (Class) p.getValue())),
            Class.class);
  }

  @Module.Fn(name = "sys:cache", rt = true)
  public static IColl sysCache(IRuntime rt) {
    return rt.classCache();
  }

  @Module.Fn(name = "sys:cache-add", rt = true)
  public static IColl sysCacheAdd(IRuntime rt, String name, Class cls) {
    return (IColl) ((IAssoc) rt.classCache()).assoc(name, cls);
  }

  @Module.Fn(name = "sys:cache-purge", rt = true)
  public static IColl sysCachePurge(IRuntime rt) {
    return (IColl) rt.classCache().empty();
  }

  @Module.Fn(name = "sys:cache-remove", rt = true, vargs = true)
  public static <ITR> IColl sysCacheRemove(IRuntime rt, ITR names) {
    return Iter.reduce(
        Iter.iter(names), rt.classCache(), (m, n) -> (IMapType) ((IMapType) m).dissoc(n));
  }
}
