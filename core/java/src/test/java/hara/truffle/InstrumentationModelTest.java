package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import hara.truffle.InstrumentationModel.Capability;
import hara.truffle.InstrumentationModel.EventDelivery;
import hara.truffle.InstrumentationModel.EventEnvelope;
import hara.truffle.InstrumentationModel.EventKind;
import hara.truffle.InstrumentationModel.EventPhase;
import hara.truffle.InstrumentationModel.InstrumentFilter;
import hara.truffle.InstrumentationModel.InstrumentMode;
import hara.truffle.InstrumentationModel.InstrumentRegistration;
import hara.truffle.InstrumentationModel.ProjectionLimits;
import hara.truffle.InstrumentationModel.ProjectionRequest;
import hara.truffle.InstrumentationModel.RuntimeBackend;
import hara.truffle.InstrumentationModel.TargetDescriptor;
import hara.truffle.InstrumentationModel.TargetKind;
import java.util.Map;
import java.util.Set;
import org.junit.Test;

public class InstrumentationModelTest {
  @Test
  public void portableProtocolAndSchemaMatchRustContracts() {
    assertEquals("hara.instrumentation/0-alpha", InstrumentationModel.PROTOCOL);
    assertEquals(
        "hara.instrumentation.event/0-alpha", InstrumentationModel.EVENT_SCHEMA);
  }

  @Test
  public void registrationsRequireEventProjectionAndModeCapabilities() {
    assertThrows(
        IllegalArgumentException.class,
        () ->
            registration(
                InstrumentMode.PASSIVE,
                Set.of(),
                Set.of(EventKind.EXECUTION_TERMINAL),
                ProjectionRequest.none()));
    assertThrows(
        IllegalArgumentException.class,
        () ->
            registration(
                InstrumentMode.PASSIVE,
                Set.of(Capability.CONTROL_PAUSE),
                Set.of(),
                ProjectionRequest.none()));
    assertThrows(
        IllegalArgumentException.class,
        () ->
            registration(
                InstrumentMode.PASSIVE,
                Set.of(),
                Set.of(),
                new ProjectionRequest(
                    false,
                    null,
                    null,
                    ProjectionLimits.defaults(),
                    null,
                    null,
                    null)));

    InstrumentRegistration bounded =
        registration(
            InstrumentMode.PASSIVE,
            Set.of(Capability.INSPECT_LOCALS),
            Set.of(),
            new ProjectionRequest(
                false,
                null,
                null,
                ProjectionLimits.defaults(),
                null,
                null,
                null));
    assertTrue(
        bounded
            .projection()
            .requiredCapabilities()
            .contains(Capability.INSPECT_LOCALS));
  }

  @Test
  public void queueAndProjectionLimitsAreHardBounded() {
    assertThrows(IllegalArgumentException.class, () -> EventDelivery.queue(0));
    assertThrows(
        IllegalArgumentException.class,
        () -> EventDelivery.queue(InstrumentationModel.MAX_QUEUE_CAPACITY + 1));
    assertThrows(
        IllegalArgumentException.class, () -> new ProjectionLimits(0, 1, 1));
    assertThrows(
        IllegalArgumentException.class,
        () ->
            new ProjectionLimits(
                1, InstrumentationModel.MAX_PROJECTION_DEPTH + 1, 1));
  }

  @Test
  public void targetAndEventSemanticsRemainBackendSpecific() {
    assertThrows(
        IllegalArgumentException.class,
        () ->
            new TargetDescriptor(
                "target",
                "session",
                TargetKind.INTERPRETER,
                new RuntimeBackend("java"),
                Set.of(Capability.TRANSFORM_HALC)));
    assertThrows(
        IllegalArgumentException.class,
        () ->
            new EventEnvelope(
                InstrumentationModel.EVENT_SCHEMA,
                InstrumentationModel.PROTOCOL,
                "instrument",
                new RuntimeBackend("java"),
                "session",
                "target",
                TargetKind.INTERPRETER,
                0,
                1,
                EventPhase.LIVE,
                EventKind.INSTRUCTION_EXECUTE,
                null,
                Map.of()));
  }

  @Test
  public void wholeWasmSupportsOnlyProtocolBoundaryEvents() {
    assertTrue(EventKind.PROTOCOL_CALL.supports(TargetKind.WHOLE_WASM));
    assertTrue(EventKind.EXECUTION_TERMINAL.supports(TargetKind.WHOLE_WASM));
    assertTrue(!EventKind.PROTOCOL_CALL.supports(TargetKind.HBC));
  }

  private static InstrumentRegistration registration(
      InstrumentMode mode,
      Set<Capability> capabilities,
      Set<EventKind> events,
      ProjectionRequest projection) {
    return new InstrumentRegistration(
        "instrument",
        "session",
        mode,
        capabilities,
        events,
        InstrumentFilter.all(),
        projection,
        EventDelivery.queue(16));
  }
}
