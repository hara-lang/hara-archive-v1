package hara.truffle.bytecode;

import static org.junit.Assert.assertArrayEquals;
import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import hara.truffle.bytecode.HbcProgram.Function;
import hara.truffle.bytecode.HbcProgram.Instruction;
import hara.truffle.bytecode.HbcProgram.Opcode;
import hara.truffle.bytecode.HbcProgram.Primitive;
import java.util.Arrays;
import java.util.List;
import org.junit.Test;

public class HbcSchemaLinksTest {
  private static HbcSchemaLinks.SchemaCoordinate coordinate(String id, char digit) {
    return new HbcSchemaLinks.SchemaCoordinate(
        id, "sha256:" + String.valueOf(digit).repeat(64));
  }

  @Test
  public void exactSchemaLinksRoundTripCanonically() {
    HbcProgram program = arithmeticProgram();
    HbcSchemaLinks.SchemaCoordinate identifier = coordinate("model/id", '1');
    HbcSchemaLinks.SchemaCoordinate account = coordinate("model/account", '2');
    byte[] encoded = HbcSchemaLinks.encode(program, List.of(identifier, account));
    assertArrayEquals(new byte[] {'H', 'B', 'C', '1'}, Arrays.copyOf(encoded, 4));

    HbcSchemaLinks.LinkedProgram decoded = HbcSchemaLinks.decode(encoded);
    assertEquals(List.of(account, identifier), decoded.schemaLinks());
    assertArrayEquals(HbcCodec.encode(program), HbcCodec.encode(decoded.program()));
    assertArrayEquals(encoded, HbcSchemaLinks.encode(decoded.program(), decoded.schemaLinks()));
  }

  @Test
  public void duplicateAndConflictingSchemaLinksAreRejected() {
    HbcProgram program = arithmeticProgram();
    HbcSchemaLinks.SchemaCoordinate first = coordinate("model/id", '1');
    HbcFormatException duplicate =
        assertThrows(
            HbcFormatException.class,
            () -> HbcSchemaLinks.encode(program, List.of(first, first)));
    assertEquals(
        "linked bytecode artifact contains duplicate schema coordinate", duplicate.getMessage());

    HbcSchemaLinks.SchemaCoordinate conflicting = coordinate("model/id", '2');
    HbcFormatException conflict =
        assertThrows(
            HbcFormatException.class,
            () -> HbcSchemaLinks.encode(program, List.of(first, conflicting)));
    assertEquals(
        "linked bytecode artifact contains conflicting schema identity", conflict.getMessage());
  }

  @Test
  public void malformedCoordinatesAreRejectedBeforeEncoding() {
    HbcFormatException identifier =
        assertThrows(
            HbcFormatException.class,
            () -> coordinate("unqualified", '1'));
    assertTrue(identifier.getMessage().contains("qualified keyword name"));

    HbcFormatException hash =
        assertThrows(
            HbcFormatException.class,
            () -> new HbcSchemaLinks.SchemaCoordinate("model/id", "sha256:BAD"));
    assertTrue(hash.getMessage().contains("canonical lowercase hex"));
  }

  @Test
  public void corruptionIsRejectedBeforeNestedProgramDecode() {
    byte[] encoded =
        HbcSchemaLinks.encode(
            arithmeticProgram(), List.of(coordinate("model/id", '1')));
    encoded[12] ^= 1;
    HbcFormatException failure =
        assertThrows(HbcFormatException.class, () -> HbcSchemaLinks.decode(encoded));
    assertEquals("linked bytecode artifact checksum mismatch", failure.getMessage());
  }

  @Test
  public void hbc0IsNotSilentlyTreatedAsLinked() {
    byte[] encoded = HbcCodec.encode(arithmeticProgram());
    HbcFormatException failure =
        assertThrows(HbcFormatException.class, () -> HbcSchemaLinks.decode(encoded));
    assertEquals("linked bytecode artifact has invalid magic", failure.getMessage());
  }

  private static HbcProgram arithmeticProgram() {
    Function entry =
        new Function(
            null,
            false,
            0,
            false,
            0,
            0,
            2,
            List.of(
                new Instruction(Opcode.CONSTANT, 0, 0, 0),
                new Instruction(Opcode.CONSTANT, 1, 0, 0),
                new Instruction(Opcode.PRIMITIVE, Primitive.ADD.id(), 2, 0),
                Instruction.of(Opcode.RETURN)),
            Arrays.asList(null, null, null, null),
            List.of());
    return new HbcProgram(List.of(19L, 23L), List.of(), List.of(entry), 0);
  }
}
