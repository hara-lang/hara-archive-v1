package hara.truffle;

import hara.lang.data.Symbol;
import hara.lang.data.types.IMapType;
import java.util.LinkedHashMap;
import java.util.Map;

/**
 * Publishes both namespace- and interface-qualified products of every live built-in protocol.
 *
 * <p>The registrar derives the declaration set from runtime Vars rather than repeating a protocol
 * inventory or scanning the classpath. Every product of one declaration maps the exact same
 * {@link HaraVar}, so descriptor roots, method roots, metadata, origin, dynamic state, and protocol
 * dispatch remain shared across both source spellings.
 */
public final class ProtocolProductLibraryProvider implements HaraLibraryProvider {
  private static final String FOUNDATION_NAMESPACE = "std.foundation";

  @Override
  public String namespace() {
    return "hara.lang.protocol-products";
  }

  @Override
  public int order() {
    // Capability providers such as Work add protocols during eager installation. Project the
    // complete live declaration set only after those providers have run.
    return 10_000;
  }

  @Override
  public boolean eager() {
    return true;
  }

  @Override
  public void install(HaraContext context) {
    Object snapshot = protocolSnapshot(context);
    if (!(snapshot instanceof IMapType<?, ?> protocols)) {
      throw new HaraException("Runtime/vars did not return a protocol Var map");
    }

    for (Object entryValue : protocols) {
      if (!(entryValue instanceof Map.Entry<?, ?> entry)
          || !(entry.getKey() instanceof Symbol localName)
          || !(entry.getValue() instanceof HaraVar protocolVar)
          || !(protocolVar.get() instanceof HaraProtocol protocol)) {
        continue;
      }
      publishProducts(context, localName.getName(), protocol);
    }
  }

  private static Object protocolSnapshot(HaraContext context) {
    HaraVar vars = context.resolve(Symbol.create("std.native.Runtime", "vars"));
    if (vars == null) {
      throw new HaraException("std.native.Runtime/vars is unavailable during protocol registration");
    }
    return context.invokeCallable(vars.get(), new Object[] {Symbol.create(FOUNDATION_NAMESPACE)});
  }

  /** Publishes all products of one declaration only after the complete collision preflight. */
  static void publishProducts(
      HaraContext context, String protocolName, HaraProtocol protocol) {
    ProductNamespaces products = ProductNamespaces.from(protocol.name(), protocolName);
    Map<Symbol, HaraVar> projections = new LinkedHashMap<>();
    HaraVar descriptor = requireSourceProduct(context, products, protocolName);
    if (descriptor.get() != protocol) {
      throw new HaraException(
          "Protocol descriptor does not contain its declared protocol: " + protocol.name());
    }
    addProjections(projections, products, protocolName, descriptor);

    for (String methodName : protocol.methods().keySet()) {
      addProjections(
          projections,
          products,
          methodName,
          requireSourceProduct(context, products, methodName));
    }

    context.publishLibraryVarProducts("Protocol product", projections);
  }

  private static HaraVar requireSourceProduct(
      HaraContext context, ProductNamespaces products, String localName) {
    ProductTarget interfaceTarget =
        new ProductTarget(products.interfaceNamespace(), localName);
    ProductTarget ownerTarget = new ProductTarget(products.ownerNamespace(), localName);
    HaraVar interfaceProduct = resolveTarget(context, interfaceTarget);
    HaraVar ownerProduct = resolveTarget(context, ownerTarget);

    if (interfaceProduct != null && ownerProduct != null && interfaceProduct != ownerProduct) {
      throw productCollision(ownerTarget, ownerProduct, interfaceProduct);
    }
    HaraVar source = interfaceProduct != null ? interfaceProduct : ownerProduct;
    if (source == null) {
      throw new HaraException(
          "Missing built-in protocol product: "
              + interfaceTarget.display()
              + " or "
              + ownerTarget.display());
    }
    return source;
  }

  private static void addProjections(
      Map<Symbol, HaraVar> projections,
      ProductNamespaces products,
      String localName,
      HaraVar source) {
    addProjection(projections, Symbol.create(products.interfaceNamespace(), localName), source);
    addProjection(projections, Symbol.create(products.ownerNamespace(), localName), source);
  }

  private static void addProjection(
      Map<Symbol, HaraVar> projections, Symbol target, HaraVar source) {
    HaraVar previous = projections.putIfAbsent(target, source);
    if (previous != null && previous != source) {
      throw new HaraException(
          "Protocol product collision at "
              + target.display()
              + ": existing "
              + previous
              + " is not "
              + source);
    }
  }

  private static HaraVar resolveTarget(HaraContext context, ProductTarget target) {
    return context.resolve(Symbol.create(target.namespace(), target.name()));
  }

  private static HaraException productCollision(
      ProductTarget target, HaraVar existing, HaraVar requested) {
    return new HaraException(
        "Protocol product collision at "
            + target.display()
            + ": existing "
            + existing
            + " is not "
            + requested);
  }

  private record ProductTarget(String namespace, String name) {
    private String display() {
      return namespace + "/" + name;
    }
  }

  private record ProductNamespaces(String ownerNamespace, String interfaceNamespace) {
    private static ProductNamespaces from(String declaration, String protocolName) {
      String dottedSuffix = "." + protocolName;
      if (declaration.endsWith(dottedSuffix)) {
        return new ProductNamespaces(
            declaration.substring(0, declaration.length() - dottedSuffix.length()), declaration);
      }

      String slashSuffix = "/" + protocolName;
      if (declaration.endsWith(slashSuffix)) {
        String owner = declaration.substring(0, declaration.length() - slashSuffix.length());
        return new ProductNamespaces(owner, owner + dottedSuffix);
      }

      throw new HaraException("Malformed built-in protocol declaration: " + declaration);
    }
  }
}
