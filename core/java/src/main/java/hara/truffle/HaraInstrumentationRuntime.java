package hara.truffle;

import com.oracle.truffle.api.source.SourceSection;
import hara.truffle.InstrumentationModel.EventKind;
import hara.truffle.InstrumentationModel.EventLocation;
import hara.truffle.InstrumentationModel.EventPhase;
import hara.truffle.InstrumentationModel.InstrumentDirective;
import hara.truffle.InstrumentationModel.TargetHandle;
import hara.truffle.bytecode.HbcProgram;
import java.util.Map;

/** Context-scoped instrumentation and suspended HBC execution state. */
final class HaraInstrumentationRuntime implements AutoCloseable {
  private final SessionKernel sessionKernel;
  private final String sessionId;
  private final ThreadLocal<Integer> interpreterRootDepth = ThreadLocal.withInitial(() -> 0);
  private volatile boolean ready;
  private HbcMachine.HbcContinuation hbcContinuation;

  HaraInstrumentationRuntime(SessionKernel sessionKernel, String sessionId) {
    this.sessionKernel = sessionKernel;
    this.sessionId = sessionId;
  }

  void markReady() {
    ready = true;
  }

  void publishInterpreterEvent(EventKind event, SourceSection source, Map<String, String> data) {
    if (!ready) return;
    TargetHandle target = target(InstrumentationModel.TargetKind.INTERPRETER);
    if (target == null || !sessionKernel.instrumentationHub().hasSubscribers(target, event)) return;
    EventLocation location = null;
    if (sessionKernel.instrumentationHub().hasSourceLocationSubscribers(target, event)
        && source != null
        && source.isAvailable()) {
      location =
          new EventLocation(
              source.getSource().getName(),
              java.util.List.of(),
              new InstrumentationModel.SourceSpan(
                  Math.max(0, source.getCharIndex()),
                  Math.max(0, source.getCharIndex() + source.getCharLength())),
              null,
              null);
    }
    sessionKernel
        .instrumentationHub()
        .publish(target, event, EventPhase.LIVE, location, data);
  }

  void publishHbcEvent(
      EventKind event,
      int instructionPointer,
      String function,
      String sourceId,
      Map<String, String> data) {
    publishHbcEvent(
        event,
        instructionPointer,
        function,
        sourceId,
        null,
        data,
        InstrumentationEventAccess.none());
  }

  void publishHbcEvent(
      EventKind event,
      int instructionPointer,
      String function,
      String sourceId,
      HbcProgram.Position position,
      Map<String, String> data,
      InstrumentationEventAccess access) {
    TargetHandle target = target(InstrumentationModel.TargetKind.HBC);
    if (target == null || !sessionKernel.instrumentationHub().hasSubscribers(target, event)) return;
    EventLocation location = null;
    if (sessionKernel.instrumentationHub().hasSourceLocationSubscribers(target, event)) {
      location =
          new EventLocation(
              sourceId,
              java.util.List.of(),
              position == null ? null : sourceSpan(position),
              function,
              instructionPointer);
    }
    sessionKernel
        .instrumentationHub()
        .publish(target, event, EventPhase.LIVE, location, data, access);
  }

  void publishWholeWasmProtocolCall(
      String targetName,
      int arity,
      HaraTargetRuntime.ResultMode resultMode,
      String status) {
    if (!ready || sessionKernel == null) return;
    TargetHandle target = target(InstrumentationModel.TargetKind.WHOLE_WASM);
    EventKind event = EventKind.PROTOCOL_CALL;
    if (target == null || !sessionKernel.instrumentationHub().hasSubscribers(target, event)) return;
    sessionKernel
        .instrumentationHub()
        .publish(
            target,
            event,
            EventPhase.LIVE,
            null,
            Map.of(
                "target", targetName,
                "arity", Integer.toString(arity),
                "result-mode", resultMode.name().toLowerCase(java.util.Locale.ROOT),
                "status", status));
  }

  private static InstrumentationModel.SourceSpan sourceSpan(HbcProgram.Position position) {
    int offset = boundedOffset(position.offset());
    return new InstrumentationModel.SourceSpan(offset, offset);
  }

  private static int boundedOffset(long offset) {
    if (offset <= 0) return 0;
    return offset >= Integer.MAX_VALUE ? Integer.MAX_VALUE : (int) offset;
  }

  boolean hbcInstrumentationEnabled(EventKind event) {
    if (!ready) return false;
    TargetHandle target = target(InstrumentationModel.TargetKind.HBC);
    return target != null && sessionKernel.instrumentationHub().hasSubscribers(target, event);
  }

  boolean hbcNativeExecutionAllowed() {
    if (!ready) return true;
    TargetHandle target = target(InstrumentationModel.TargetKind.HBC);
    return target == null || sessionKernel.instrumentationHub().hbcNativeExecutionAllowed(target);
  }

  InstrumentDirective pollHbcDirective() {
    if (!ready) return null;
    TargetHandle target = target(InstrumentationModel.TargetKind.HBC);
    return target == null ? null : sessionKernel.instrumentationHub().pollDirective(target);
  }

  synchronized HbcMachine.HbcContinuation hbcContinuation(HbcProgram program) {
    if (hbcContinuation == null) return null;
    if (hbcContinuation.program != program) {
      throw new HaraException("HBC execution is suspended for another program");
    }
    return hbcContinuation;
  }

  synchronized void retainHbcContinuation(HbcMachine.HbcContinuation continuation) {
    hbcContinuation = continuation;
  }

  synchronized void clearHbcContinuation(HbcMachine.HbcContinuation continuation) {
    if (hbcContinuation == continuation) hbcContinuation = null;
  }

  synchronized void clearHbcContinuation() {
    hbcContinuation = null;
  }

  boolean enterInterpreterRoot() {
    int depth = interpreterRootDepth.get();
    interpreterRootDepth.set(depth + 1);
    return depth == 0;
  }

  void exitInterpreterRoot() {
    int depth = interpreterRootDepth.get();
    if (depth <= 1) {
      interpreterRootDepth.remove();
    } else {
      interpreterRootDepth.set(depth - 1);
    }
  }

  void publishInterpreterTerminal(SourceSection source, String status) {
    publishInterpreterEvent(EventKind.EXECUTION_TERMINAL, source, Map.of("status", status));
  }

  void publishInterpreterSemanticBoundary(SourceSection source) {
    publishInterpreterEvent(EventKind.SEMANTIC_BOUNDARY, source, Map.of());
  }

  void publishInterpreterTopLevelFailure(SourceSection source, RuntimeException error) {
    publishInterpreterEvent(
        EventKind.EXCEPTION_RAISE, source, Map.of("type", error.getClass().getName()));
    publishInterpreterTerminal(source, "failure");
  }

  private TargetHandle target(InstrumentationModel.TargetKind kind) {
    if (sessionKernel == null
        || !sessionKernel.instrumentationActive()
        || sessionId == null
        || sessionId.isBlank()) {
      return null;
    }
    return sessionKernel.instrumentationTarget(sessionId, kind);
  }

  @Override
  public void close() {
    clearHbcContinuation();
    interpreterRootDepth.remove();
    ready = false;
  }
}
