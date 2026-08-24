package hara.truffle;

import hara.truffle.InstrumentationModel.EventProjection;
import hara.truffle.InstrumentationModel.PortableProjection;
import hara.truffle.InstrumentationModel.ProjectionLimits;
import hara.truffle.InstrumentationModel.ProjectionRequest;
import hara.truffle.bytecode.HbcProgram;
import java.util.ArrayList;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/** Bounded, copy-only inspection of one live HBC machine boundary. */
final class HbcInstrumentationAccess implements InstrumentationEventAccess {
  private final HbcProgram program;
  private final int functionIndex;
  private final HbcProgram.Function function;
  private final int instructionPointer;
  private final Object[] locals;
  private final List<Object> stack;
  private final List<HbcMachine.CallFrame> calls;
  private final String status;
  private final Object result;
  private final String error;

  private HbcInstrumentationAccess(
      HbcProgram program,
      int functionIndex,
      HbcProgram.Function function,
      int instructionPointer,
      Object[] locals,
      List<Object> stack,
      List<HbcMachine.CallFrame> calls,
      String status,
      Object result,
      String error) {
    this.program = program;
    this.functionIndex = functionIndex;
    this.function = function;
    this.instructionPointer = instructionPointer;
    this.locals = locals == null ? new Object[0] : locals.clone();
    this.stack =
        stack == null
            ? List.of()
            : Collections.unmodifiableList(new ArrayList<>(stack));
    this.calls = calls == null ? List.of() : List.copyOf(calls);
    this.status = status == null ? "running" : status;
    this.result = result;
    this.error = error;
  }

  static HbcInstrumentationAccess live(
      HbcProgram program,
      int functionIndex,
      HbcProgram.Function function,
      int instructionPointer,
      Object[] locals,
      List<Object> stack,
      List<HbcMachine.CallFrame> calls) {
    return new HbcInstrumentationAccess(
        program,
        functionIndex,
        function,
        instructionPointer,
        locals,
        stack,
        calls,
        "running",
        null,
        null);
  }

  static HbcInstrumentationAccess terminal(
      HbcProgram program,
      int functionIndex,
      HbcProgram.Function function,
      int instructionPointer,
      Object[] locals,
      List<Object> stack,
      List<HbcMachine.CallFrame> calls,
      String status,
      Object result,
      String error) {
    return new HbcInstrumentationAccess(
        program,
        functionIndex,
        function,
        instructionPointer,
        locals,
        stack,
        calls,
        status,
        result,
        error);
  }

  @Override
  public EventProjection project(ProjectionRequest request) {
    if (request == null) return EventProjection.none();
    return new EventProjection(
        request.currentFrame() == null ? null : currentFrame(request.currentFrame()),
        request.frames() == null ? null : frames(request.frames()),
        request.locals() == null ? null : locals(request.locals()),
        request.stack() == null ? null : stack(request.stack()),
        request.valuePreview() == null ? null : valuePreview(request.valuePreview()),
        request.machineSnapshot() == null ? null : snapshot(request.machineSnapshot()));
  }

  private PortableProjection currentFrame(ProjectionLimits limits) {
    Map<String, String> fields = baseFields();
    fields.put("stack-base", "0");
    addValues(fields, "local", java.util.Arrays.asList(locals), limits);
    return projection("hbc/current-frame", fields);
  }

  private PortableProjection frames(ProjectionLimits limits) {
    Map<String, String> fields = new LinkedHashMap<>();
    fields.put("count", Integer.toString(calls.size() + 1));
    int retainedCalls = Math.min(calls.size(), limits.maxItems());
    int start = calls.size() - retainedCalls;
    List<HbcMachine.CallFrame> newestFirst = new ArrayList<>(calls);
    for (int index = 0; index < retainedCalls; index++) {
      HbcMachine.CallFrame call = newestFirst.get(calls.size() - 1 - index);
      fields.put("frame/" + index + "/function", Integer.toString(call.functionIndex()));
      fields.put("frame/" + index + "/call-ip", Integer.toString(call.returnIp() - 1));
      if (call.function().name() != null) {
        fields.put("frame/" + index + "/name", call.function().name());
      }
    }
    fields.put("omitted", Integer.toString(start));
    return projection("hbc/frames", fields);
  }

  private PortableProjection locals(ProjectionLimits limits) {
    Map<String, String> fields = new LinkedHashMap<>();
    addValues(fields, "local", java.util.Arrays.asList(locals), limits);
    fields.put("count", Integer.toString(locals.length));
    fields.put("omitted", Integer.toString(Math.max(0, locals.length - limits.maxItems())));
    return projection("hbc/locals", fields);
  }

  private PortableProjection stack(ProjectionLimits limits) {
    Map<String, String> fields = new LinkedHashMap<>();
    int start = Math.max(0, stack.size() - limits.maxItems());
    addValues(fields, "stack", stack.subList(start, stack.size()), limits, start);
    fields.put("count", Integer.toString(stack.size()));
    fields.put("omitted", Integer.toString(start));
    return projection("hbc/stack", fields);
  }

  private PortableProjection valuePreview(ProjectionLimits limits) {
    if (stack.isEmpty()) return null;
    Object value = stack.get(stack.size() - 1);
    Map<String, String> fields = new LinkedHashMap<>();
    fields.put("display", display(value, limits.maxBytes()));
    return projection("hbc/value-preview", fields);
  }

  private PortableProjection snapshot(ProjectionLimits limits) {
    Map<String, String> fields = new LinkedHashMap<>();
    fields.put("program/entry", Integer.toString(program.entry()));
    fields.put("program/functions", Integer.toString(program.functions().size()));
    fields.put("program/constants", Integer.toString(program.constants().size()));
    fields.put("function", Integer.toString(functionIndex));
    fields.put("ip", Integer.toString(instructionPointer));
    fields.put("calls", Integer.toString(calls.size()));
    fields.put("stack/depth", Integer.toString(stack.size()));
    fields.put("locals/count", Integer.toString(locals.length));
    int retained = Math.min(stack.size(), limits.maxItems());
    int start = stack.size() - retained;
    addValues(fields, "stack", stack.subList(start, stack.size()), limits, start);
    return projection("hbc/snapshot", fields);
  }

  private Map<String, String> baseFields() {
    Map<String, String> fields = new LinkedHashMap<>();
    fields.put("function", Integer.toString(functionIndex));
    if (function.name() != null) fields.put("function/name", function.name());
    fields.put("ip", Integer.toString(instructionPointer));
    return fields;
  }

  private static void addValues(
      Map<String, String> fields,
      String prefix,
      List<Object> values,
      ProjectionLimits limits) {
    addValues(fields, prefix, values, limits, 0);
  }

  private static void addValues(
      Map<String, String> fields,
      String prefix,
      List<Object> values,
      ProjectionLimits limits,
      int offset) {
    int retained = Math.min(values.size(), limits.maxItems());
    for (int index = 0; index < retained; index++) {
      fields.put(prefix + "/" + (offset + index), display(values.get(index), limits.maxBytes()));
    }
  }

  private static PortableProjection projection(String kind, Map<String, String> fields) {
    return new PortableProjection(kind, fields);
  }

  private static String display(Object value, int maxBytes) {
    return bounded(value == null ? "nil" : String.valueOf(value), maxBytes);
  }

  private static String bounded(String value, int maxBytes) {
    if (value.length() <= maxBytes) return value;
    return value.substring(0, Math.max(0, maxBytes - 1)) + "…";
  }
}
