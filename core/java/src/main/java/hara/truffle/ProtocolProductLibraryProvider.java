package hara.truffle;

import hara.lang.data.Symbol;
import hara.lang.data.types.IMapType;
import java.util.Map;

/**
 * Publishes the interface-qualified products of every live built-in protocol declaration.
 *
 * <p>The slash-qualified protocol and method Vars remain the declaration owners. The generated
 * dotted products reuse the same protocol descriptor and method callable values, so both source
 * spellings observe one protocol dispatch registry rather than independent implementations.
 */
public final class ProtocolProductLibraryProvider implements HaraLibraryProvider {
  private static final String FOUNDATION_NAMESPACE = "std.foundation";
  private static final String INTRINSIC_NAMESPACE = "hara.lang.intrinsic";

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
    String declaration = protocol.name();
    int separator = declaration.lastIndexOf('/');
    if (separator <= 0 || !declaration.substring(separator + 1).equals(protocolName)) {
      throw new HaraException("Malformed built-in protocol declaration: " + declaration);
    }

    String ownerNamespace = declaration.substring(0, separator);
    String interfaceNamespace = ownerNamespace + "." + protocolName;

    // Dotted type/interface identities follow the same intrinsic publication model as
    // std.native.<Type>: they are visible in fresh and subsequently declared namespaces.
    context.defineLibraryValue(
        INTRINSIC_NAMESPACE, interfaceNamespace, protocol, protocolVar.meta());
    context.defineLibraryValue(
        interfaceNamespace, protocolName, protocol, protocolVar.meta());

    for (String methodName : protocol.methods().keySet()) {
      HaraVar methodVar = context.resolve(Symbol.create(ownerNamespace, methodName));
      if (methodVar == null) {
        throw new HaraException(
            "Missing built-in protocol method Var: " + ownerNamespace + "/" + methodName);
      }
      context.defineLibraryValue(
          interfaceNamespace, methodName, methodVar.get(), methodVar.meta());
    }
  }
}
