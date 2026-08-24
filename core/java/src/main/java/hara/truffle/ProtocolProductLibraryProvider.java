package hara.truffle;

import hara.lang.data.Symbol;
import hara.lang.data.types.IMapType;
import java.util.Map;

/**
 * Publishes both namespace- and interface-qualified products of every live built-in protocol.
 *
 * <p>The registrar derives the declaration set from runtime Vars rather than repeating a protocol
 * inventory or scanning the classpath. Missing products reuse the existing protocol descriptor and
 * method callable values, so both source spellings observe one {@link HaraProtocol} dispatch
 * registry.
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
      publishProducts(context, localName.getName(), protocolVar, protocol);
    }
  }

  private static Object protocolSnapshot(HaraContext context) {
    HaraVar vars = context.resolve(Symbol.create("std.native.Runtime", "vars"));
    if (vars == null) {
      throw new HaraException("std.native.Runtime/vars is unavailable during protocol registration");
    }
    return context.invokeCallable(vars.get(), new Object[] {Symbol.create(FOUNDATION_NAMESPACE)});
  }

  private static void publishProducts(
      HaraContext context, String protocolName, HaraVar protocolVar, HaraProtocol protocol) {
    ProductNamespaces products = ProductNamespaces.from(protocol.name(), protocolName);

    HaraVar interfaceDescriptor = context.resolve(Symbol.create(products.interfaceNamespace()));
    HaraVar ownerDescriptor =
        context.resolve(Symbol.create(products.ownerNamespace(), protocolName));
    if (interfaceDescriptor == null && ownerDescriptor == null) {
      throw new HaraException("Protocol declaration has no registered descriptor: " + protocol.name());
    }
    if (interfaceDescriptor == null) {
      context.defineLibraryValue(
          products.interfaceNamespace(), protocolName, protocol, protocolVar.meta());
    }
    if (ownerDescriptor == null) {
      context.defineLibraryValue(
          products.ownerNamespace(), protocolName, protocol, protocolVar.meta());
    }

    for (String methodName : protocol.methods().keySet()) {
      HaraVar interfaceMethod =
          context.resolve(Symbol.create(products.interfaceNamespace(), methodName));
      HaraVar ownerMethod =
          context.resolve(Symbol.create(products.ownerNamespace(), methodName));
      HaraVar source = interfaceMethod != null ? interfaceMethod : ownerMethod;
      if (source == null) {
        throw new HaraException(
            "Missing built-in protocol method Var: " + protocol.name() + "/" + methodName);
      }
      if (interfaceMethod == null) {
        context.defineLibraryValue(
            products.interfaceNamespace(), methodName, source.get(), source.meta());
      }
      if (ownerMethod == null) {
        context.defineLibraryValue(
            products.ownerNamespace(), methodName, source.get(), source.meta());
      }
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
