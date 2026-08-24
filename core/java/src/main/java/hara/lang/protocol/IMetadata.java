package hara.lang.protocol;

import hara.lang.declaration.HaraHostSupport;

@HaraHostSupport(reason = "Java metadata carrier used by host adapters; not a guest protocol")
public interface IMetadata {

  Constant.MetaType getMetatype();
}
