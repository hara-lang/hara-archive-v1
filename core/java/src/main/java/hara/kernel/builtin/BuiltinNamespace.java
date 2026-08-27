package hara.kernel.builtin;

import hara.lang.base.Ex;
import hara.kernel.base.Module;
import hara.kernel.base.RT;
import hara.kernel.protocol.IRuntime;
import hara.lang.data.AsMap;
import hara.lang.data.Symbol;
import hara.lang.protocol.IMapType;
import hara.kernel.base.Var;

import java.util.Iterator;

@SuppressWarnings({"unchecked", "rawtypes"})
@Module.Ns(name = "global", tag = "ns")
public interface BuiltinNamespace {
  @Module.Fn(name = "ns-name")
  public static Symbol nsName(hara.kernel.base.Namespace ns) {
    return ns.name;
  }

  public static IMapType nsMap(hara.kernel.base.Namespace ns) {
    return new AsMap(ns.mappings);
  }

  @Module.Fn(name = "ns-aliases")
  public static IMapType nsAliases(hara.kernel.base.Namespace ns) {
    return new AsMap(ns.aliases);
  }

  public static IMapType nsImports(hara.kernel.base.Namespace ns) {
    return new AsMap(ns.imports);
  }

  @Module.Fn(name = "ns-find", rt = true)
  public static hara.kernel.base.Namespace nsFind(IRuntime rt, Symbol sym) {
    if (rt instanceof RT.Instance) {
      return ((RT.Instance) rt).getNamespace(sym);
    }
    return null;
  }

  public static Iterator<hara.kernel.base.Namespace> nsList(IRuntime rt) {
    if (rt instanceof RT.Instance) {
      return ((RT.Instance) rt)._userEnv.getNamespaces().values().iterator();
    }
    return null;
  }

  @Module.Fn(name = "ns-create", rt = true)
  public static hara.kernel.base.Namespace nsCreate(IRuntime rt, Symbol sym) {
    if (rt instanceof RT.Instance) {
      return ((RT.Instance) rt).addNamespace(sym);
    }
    return null;
  }
}
