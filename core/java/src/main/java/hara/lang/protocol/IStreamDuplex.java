package hara.lang.protocol;

import hara.lang.declaration.HaraAvailability;
import hara.lang.declaration.HaraProtocolBinding;

/** Bidirectional stream with graceful and exceptional termination. */
@HaraProtocolBinding(
    namespace = "std.protocol.istreamduplex",
    name = "IStreamDuplex",
    parents = {"IStream", "IStreamWrite", "IAbort"},
    availability = HaraAvailability.CAPABILITY_GATED,
    capability = "native-runtime-protocols")
public interface IStreamDuplex extends IStream, IStreamWrite, IAbort {}
