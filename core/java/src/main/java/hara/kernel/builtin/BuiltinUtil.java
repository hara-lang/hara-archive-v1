package hara.kernel.builtin;

import hara.kernel.base.Module;
import hara.lang.base.G;
import hara.lang.base.Iter;
import hara.lang.protocol.IMapType;
import hara.lang.protocol.IDisplay;
import hara.lang.protocol.IObjType;

@SuppressWarnings({"unchecked", "rawtypes"})
@Module.Ns(name = "global", tag = "lambda")
public interface BuiltinUtil {
  //
  // Print
  //

  @Module.Fn(name = "pr-str", complete = true)
  public static String prStr(Object e) {
    return G.display(e);
  }

  @Module.Fn(name = "str", vargs = true, complete = true)
  public static <ITR> String str(ITR args) {
    return Iter.toString(
        Iter.iter(args),
        "",
        "",
        "",
        (e) -> {
          if (e == null) {
            return "";
          } else if (e instanceof String) {
            return (String) e;
          } else if (e instanceof IDisplay) {
            return ((IDisplay) e).display();
          } else {
            return e.toString();
          }
        });
  }

  @Module.Fn(name = "println", vargs = true, complete = true)
  public static void println(Object args) {
    var current = hara.kernel.base.RT.Instance.CURRENT.get();
    var out = (current != null) ? current.out : System.out;
    out.println(
        Iter.toString(
            Iter.iter(args),
            "",
            "",
            " ",
            (e) -> {
              if (e == null) {
                return "nil";
              } else {
                return String.valueOf(e);
              }
            }));
  }

  @Module.Fn(name = "prn", vargs = true, complete = true)
  public static void prn(Object args) {
    var current = hara.kernel.base.RT.Instance.CURRENT.get();
    var out = (current != null) ? current.out : System.out;
    out.println(Iter.toString(Iter.iter(args), "", "", " ", G::display));
  }

  @Module.Fn(name = "doc", complete = true)
  public static void doc(Object obj) {
    var current = hara.kernel.base.RT.Instance.CURRENT.get();
    var out = (current != null) ? current.out : System.out;
    if (obj instanceof IObjType) {
      var meta = ((IObjType) obj).meta();
      if (meta != null) {
        var metaMap = (IMapType) meta;
        var doc = metaMap.lookup(BuiltinBasic.keyword("doc"));
        if (doc != null) {
          out.println("-------------------------");
          out.println(metaMap.lookup(BuiltinBasic.keyword("name")));
          out.println(metaMap.lookup(BuiltinBasic.keyword("arglists")));
          out.println("  " + doc);
        }
      }
    }
  }
}
