package hara.truffle.bytecode;

import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import java.util.Arrays;
import java.util.List;
import org.junit.Test;

public class HbcCodegenPlanTest {
  @Test
  public void acceptsTheValueAndCallSubsetWithStableStackHeights() {
    HbcProgram.Function callee =
        function(
            "callee",
            1,
            1,
            1,
            List.of(
                new HbcProgram.Instruction(HbcProgram.Opcode.LOAD_LOCAL, 0, 0, 0),
                HbcProgram.Instruction.of(HbcProgram.Opcode.RETURN)));
    HbcProgram.Function entry =
        function(
            null,
            0,
            0,
            1,
            List.of(
                new HbcProgram.Instruction(HbcProgram.Opcode.CONSTANT, 0, 0, 0),
                new HbcProgram.Instruction(HbcProgram.Opcode.CALL_STATIC, 1, 1, 0),
                HbcProgram.Instruction.of(HbcProgram.Opcode.RETURN)));

    HbcCodegenPlan plan =
        HbcCodegenPlan.analyze(
            new HbcProgram(List.of(42L), List.of(), List.of(entry, callee), 0));

    assertTrue(plan.entryEligible());
    assertTrue(plan.entry().reason() == null);
  }

  @Test
  public void acceptsReducibleConditionalControlAndRoutesHandlersToThePortableMachine() {
    HbcProgram.Function conditional =
        function(
            null,
            0,
            0,
            1,
            List.of(
                HbcProgram.Instruction.of(HbcProgram.Opcode.TRUE),
                new HbcProgram.Instruction(HbcProgram.Opcode.JUMP_IF_FALSE, 4, 0, 0),
                new HbcProgram.Instruction(HbcProgram.Opcode.CONSTANT, 0, 0, 0),
                new HbcProgram.Instruction(HbcProgram.Opcode.JUMP, 5, 0, 0),
                new HbcProgram.Instruction(HbcProgram.Opcode.CONSTANT, 1, 0, 0),
                HbcProgram.Instruction.of(HbcProgram.Opcode.RETURN)));
    HbcProgram.Function handled =
        new HbcProgram.Function(
            null,
            false,
            0,
            false,
            0,
            1,
            1,
            List.of(
                new HbcProgram.Instruction(HbcProgram.Opcode.CONSTANT, 0, 0, 0),
                HbcProgram.Instruction.of(HbcProgram.Opcode.THROW),
                new HbcProgram.Instruction(HbcProgram.Opcode.LOAD_LOCAL, 0, 0, 0),
                HbcProgram.Instruction.of(HbcProgram.Opcode.RETURN)),
            Arrays.asList(null, null, null, null),
            List.of(
                new HbcProgram.TryEntry(
                    0,
                    2,
                    0,
                    List.of(new HbcProgram.CatchEntry("Exception", 0, 2)),
                    null,
                    null,
                    null)));

    assertTrue(
        HbcCodegenPlan.analyze(new HbcProgram(List.of(1L, 2L), List.of(), List.of(conditional), 0))
            .entryEligible());
    assertFalse(
        HbcCodegenPlan.analyze(new HbcProgram(List.of("boom"), List.of(), List.of(handled), 0))
            .entryEligible());
  }

  @Test
  public void acceptsStructuredBackwardLoopControl() {
    HbcProgram.Function loop =
        function(
            null,
            0,
            1,
            2,
            List.of(
                new HbcProgram.Instruction(HbcProgram.Opcode.CONSTANT, 0, 0, 0),
                HbcProgram.Instruction.of(HbcProgram.Opcode.STORE_LOCAL),
                new HbcProgram.Instruction(HbcProgram.Opcode.LOAD_LOCAL, 0, 0, 0),
                new HbcProgram.Instruction(HbcProgram.Opcode.CONSTANT, 1, 0, 0),
                new HbcProgram.Instruction(
                    HbcProgram.Opcode.PRIMITIVE, HbcProgram.Primitive.LESS.id(), 2, 0),
                new HbcProgram.Instruction(HbcProgram.Opcode.JUMP_IF_FALSE, 11, 0, 0),
                new HbcProgram.Instruction(HbcProgram.Opcode.LOAD_LOCAL, 0, 0, 0),
                new HbcProgram.Instruction(HbcProgram.Opcode.CONSTANT, 2, 0, 0),
                new HbcProgram.Instruction(
                    HbcProgram.Opcode.PRIMITIVE, HbcProgram.Primitive.ADD.id(), 2, 0),
                HbcProgram.Instruction.of(HbcProgram.Opcode.STORE_LOCAL),
                new HbcProgram.Instruction(HbcProgram.Opcode.JUMP, 2, 0, 0),
                new HbcProgram.Instruction(HbcProgram.Opcode.LOAD_LOCAL, 0, 0, 0),
                HbcProgram.Instruction.of(HbcProgram.Opcode.RETURN)));

    assertTrue(
        HbcCodegenPlan.analyze(new HbcProgram(List.of(0L, 10L, 1L), List.of(), List.of(loop), 0))
            .entryEligible());
  }

  private static HbcProgram.Function function(
      String name, int arity, int localCount, int maxStack, List<HbcProgram.Instruction> code) {
    return new HbcProgram.Function(
        name,
        false,
        arity,
        false,
        0,
        localCount,
        maxStack,
        code,
        code.stream().map(ignored -> (HbcProgram.Position) null).toList(),
        List.of());
  }
}
