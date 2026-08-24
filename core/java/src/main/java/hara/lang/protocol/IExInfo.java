package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(namespace = "std.protocol.iexinfo", name = "IExInfo")
public interface IExInfo {
  @HaraMethod(value = "data", arity = 1)
  public IMetadata getData();
}
