package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(
    namespace = "std.protocol.iobjtype", name = "IObjType", parents = {"IHash", "IDisplay"})
public interface IObjType extends IHash, IDisplay {

  default Constant.ObjType getObjType() {
    return Constant.ObjType.CLASS;
  }

  default String getObjName() {
    return getObjType().toString();
  }

  @Override
  default String hashSeed() {
    return "::" + getObjName() + "";
  }

  @HaraMethod(value = "meta", arity = 1)
  IMetadata meta();

  @HaraMethod(value = "with-meta", arity = 2)
  IObjType withMeta(IMetadata meta);
}
