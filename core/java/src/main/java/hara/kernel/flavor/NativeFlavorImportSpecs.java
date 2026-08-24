package hara.kernel.flavor;

import hara.lang.data.Symbol;
import hara.lang.protocol.ILinearType;
import java.util.ArrayList;
import java.util.List;

/** Normalizes the source-level import specifications attached to a host flavor. */
public final class NativeFlavorImportSpecs {
  public record Spec(String localName, String typeName) {}

  private NativeFlavorImportSpecs() {}

  public static List<Spec> parse(Iterable<?> specifications) {
    ArrayList<Spec> result = new ArrayList<>();
    for (Object specification : specifications) {
      if (specification instanceof Symbol symbol) {
        if (symbol.getNamespace() != null) {
          throw invalid(":flavor import symbols must be unqualified");
        }
        add(result, symbol.getName());
      } else if (specification instanceof ILinearType<?> group) {
        parsePackage(result, group);
      } else {
        throw invalid(":flavor expects host import symbols or package vectors");
      }
    }
    return List.copyOf(result);
  }

  private static void parsePackage(List<Spec> result, ILinearType<?> group) {
    if (!"[".equals(group.startString()) || group.count() < 2) {
      throw invalid(":flavor package vector requires a package and at least one class");
    }
    Object packageValue = group.nth(0);
    if (!(packageValue instanceof Symbol packageSymbol)
        || packageSymbol.getNamespace() != null) {
      throw invalid(":flavor package must be an unqualified symbol");
    }
    String packageName = packageSymbol.getName();
    for (int index = 1; index < group.count(); index++) {
      Object classValue = group.nth(index);
      if (!(classValue instanceof Symbol classSymbol)
          || classSymbol.getNamespace() != null
          || classSymbol.getName().indexOf('.') >= 0) {
        throw invalid(":flavor class must be an unqualified symbol");
      }
      add(result, packageName + "." + classSymbol.getName());
    }
  }

  private static void add(List<Spec> result, String typeName) {
    result.add(new Spec(simpleName(typeName), typeName));
  }

  private static String simpleName(String typeName) {
    int separator = Math.max(typeName.lastIndexOf('.'), typeName.lastIndexOf('$'));
    return typeName.substring(separator + 1);
  }

  private static IllegalArgumentException invalid(String message) {
    return new IllegalArgumentException(message);
  }
}
