package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(namespace = "std.protocol.imetadata", name = "IMetadata")
public interface IMetadata {

  @HaraMethod(value = "metatype", arity = 1)
  Constant.MetaType getMetatype();
}
