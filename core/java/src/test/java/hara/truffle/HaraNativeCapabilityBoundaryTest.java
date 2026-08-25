package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import hara.lang.base.Ex;
import hara.lang.data.Keyword;
import hara.lang.protocol.IMapType;
import org.junit.Test;

public class HaraNativeCapabilityBoundaryTest {
  @Test
  public void policyIsExplicitAndDenialsCarryNativeData() {
    HaraNativeCapabilityBoundary boundary =
        new HaraNativeCapabilityBoundary(true, false, true, false, false, false);

    assertTrue(boundary.granted("kernel"));
    assertTrue(boundary.granted("file"));
    assertFalse(boundary.granted("network"));
    assertFalse(boundary.granted("host-call"));

    Ex.Info error =
        assertThrows(
            Ex.Info.class,
            () -> boundary.require("Socket", "connect", "network"));
    assertEquals(
        "std.native.Socket/connect requires capability :network", error.getMessage());
    assertEquals(
        Keyword.create("native", "capability-denied"),
        ((IMapType) error.data).lookup(Keyword.create("ex", "code")));
    assertEquals("write", HaraNativeCapabilityBoundary.method("os/process-write"));
  }
}
