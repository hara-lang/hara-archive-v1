package hara.lang.protocol;

import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(namespace = "std.protocol.iofn", name = "IOFn", parents = {"IFn"})
public interface IOFn extends IFn<Object, Object, Object> {}
