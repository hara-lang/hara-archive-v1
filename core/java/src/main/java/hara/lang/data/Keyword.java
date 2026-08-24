package hara.lang.data;

import hara.lang.base.Ex;
import hara.lang.base.primitive.Array;
import hara.lang.data.types.IStringType;
import hara.lang.protocol.ISetType;
import hara.lang.protocol.*;

import java.lang.ref.WeakReference;
import java.util.function.BiFunction;
import java.util.function.Function;

public final class Keyword
    implements Comparable<Keyword>, IStringType, INamespaced, IDisplay, IObjType, IOFn, IMetadata {
  public static hara.lang.base.primitive.RefCache<String, Keyword> GLOBAL =
      new hara.lang.base.primitive.RefCache<String, Keyword>();

  private final String _ns;
  private final String _name;
  private final String _full;

  private Keyword(String ns, String name, String full) {
    _ns = ns;
    _name = name;
    _full = full;
  }

  public static Keyword create(String ns, String name) {
    String full = (ns == null) ? name : ns + "/" + name;

    return GLOBAL.getOrCreate(
        full,
        () -> {
          var k = new Keyword(ns, name, full);
          k.hashGet();
          return new WeakReference<Keyword>(k, GLOBAL.getQueue());
        });
  }

  public static Keyword create(String nsname) {
    if (nsname == null || nsname.isEmpty()) {
      throw new IllegalArgumentException("Keyword name cannot be empty.");
    }
    if (nsname.equals("/")) {
      throw new IllegalArgumentException("Keyword name cannot be a single slash.");
    }
    int i = nsname.indexOf('/');
    int j = nsname.lastIndexOf('/');
    if (i == -1) { // No slash
      return create(null, nsname);
    } else if (i != j) { // More than one slash
      throw new IllegalArgumentException("Keyword name can only contain one slash.");
    } else if (i == 0) { // Starts with slash
      throw new IllegalArgumentException("Keyword name cannot start with a slash.");
    } else if (i == nsname.length() - 1) { // Ends with slash
      throw new IllegalArgumentException("Keyword name cannot end with a slash.");
    } else { // Exactly one slash, not at the beginning or end
      return create(nsname.substring(0, i), nsname.substring(i + 1));
    }
  }

  @Override
  public int compareTo(Keyword o) {
    return this._full.compareTo(o._full);
  }

  @Override
  public String display() {
    return ":" + _full;
  }

  @Override
  public String getName() {
    return _name;
  }

  @Override
  public String getNamespace() {
    return _ns;
  }

  @Override
  public Constant.ObjType getObjType() {
    return Constant.ObjType.KEYWORD;
  }

  @Override
  public IMetadata meta() {
    return null;
  }

  @Override
  public Keyword withMeta(IMetadata meta) {
    return this;
  }

  @Override
  public Constant.MetaType getMetatype() {
    return Constant.MetaType.STRING;
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  @Override
  public final Function getArg1() {
    return (obj) -> {
      if (obj instanceof ILookup) {
        return ((ILookup) obj).lookup(this);
      } else if (obj instanceof ISetType) {
        return ((ISetType) obj).find(this);
      } else if (obj instanceof java.util.Set) {
        return ((java.util.Set) obj).contains(this) ? this : null;
      } else if (obj instanceof java.util.Map) {
        return ((java.util.Map) obj).get(this);
      } else if (obj instanceof IContext) {
        return ((IContext) obj).call(this);
      } else if (obj == null) {
        return null;
      } else {
        throw new Ex.Unsupported(
            "Keyword " + display() + " cannot look up " + obj.getClass().getName());
      }
    };
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  @Override
  public final BiFunction getArg2() {
    return (obj, arg) -> {
      if (obj instanceof ILookup) {
        return ((ILookup) obj).lookup(this, arg);
      } else if (obj instanceof ISetType) {
        Object value = ((ISetType) obj).find(this);
        return value == null ? arg : value;
      } else if (obj instanceof java.util.Set) {
        return ((java.util.Set) obj).contains(this) ? this : arg;
      } else if (obj instanceof java.util.Map) {
        return ((java.util.Map) obj).getOrDefault(this, arg);
      } else if (obj instanceof IContext) {
        return ((IContext) obj).call(this, arg);
      } else if (obj == null) {
        return arg;
      } else {
        throw new Ex.Unsupported(
            "Keyword " + display() + " cannot look up " + obj.getClass().getName());
      }
    };
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  @Override
  public final Function getArgN() {
    return (vargs) -> {
      var args = Array.toArray(vargs);
      if (args.length > 0) {
        if (args[0] instanceof IContext) {
          IContext ctx = (IContext) args[0];
          args[0] = this;
          return ctx.call(args);
        }
      }
      throw new Ex.Unsupported();
    };
  }
}
