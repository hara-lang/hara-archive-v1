package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(namespace = "std.protocol.idisplay", name = "IDisplay")
public interface IDisplay {

  @HaraMethod(value = "display", arity = 1)
  String display();
  /*
   * default String display() { return toString(); }
   */
}
