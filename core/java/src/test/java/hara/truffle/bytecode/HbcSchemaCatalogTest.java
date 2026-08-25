package hara.truffle.bytecode;

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

public class HbcSchemaCatalogTest {
  private static HbcSchemaLinks.SchemaCoordinate coordinate(String id, char digit) {
    return new HbcSchemaLinks.SchemaCoordinate(
        id, "sha256:" + String.valueOf(digit).repeat(64));
  }

  private static HbcSchemaCatalog.CatalogComponent component(
      List<HbcSchemaLinks.SchemaCoordinate> members,
      List<String> dependencies) {
    return new HbcSchemaCatalog.CatalogComponent(
        HbcSchemaCatalog.componentId(members), members, dependencies);
  }

  @Test
  public void componentIdentityMatchesThePortableCatalogEpoch() {
    HbcSchemaLinks.SchemaCoordinate identifier = coordinate("model/id", '1');
    assertEquals(
        "sha256:eb2433d563d47c84b3469d37f8786ee00ae0f7080b2505fc839d851615171c32",
        HbcSchemaCatalog.componentId(List.of(identifier)));
  }

  @Test
  public void linkedProgramIsReleasedWithDependencyFirstExactClosure() {
    HbcSchemaLinks.SchemaCoordinate identifier = coordinate("model/id", '1');
    HbcSchemaLinks.SchemaCoordinate profile = coordinate("model/profile", '2');
    HbcSchemaCatalog.CatalogComponent identifierComponent =
        component(List.of(identifier), List.of());
    HbcSchemaCatalog.CatalogComponent profileComponent =
        component(List.of(profile), List.of(identifierComponent.id()));
    HbcSchemaCatalog.AdmittedCatalog catalog =
        HbcSchemaCatalog.admitCatalog(
            List.of(
                new HbcSchemaCatalog.CatalogEntry(identifier, List.of()),
                new HbcSchemaCatalog.CatalogEntry(profile, List.of(identifier))),
            List.of(profileComponent, identifierComponent));

    byte[] artifact = HbcSchemaLinks.encode(arithmeticProgram(), List.of(profile));
    HbcSchemaCatalog.AdmittedLinkedProgram admitted =
        HbcSchemaCatalog.admitLinkedProgram(artifact, catalog);
    assertEquals(List.of(profile), admitted.linked().schemaLinks());
    assertEquals(List.of(identifier, profile), admitted.resolvedCoordinates());
  }

  @Test
  public void staleOrMissingExactLinksFailBeforeProgramRelease() {
    HbcSchemaLinks.SchemaCoordinate identifier = coordinate("model/id", '1');
    HbcSchemaCatalog.AdmittedCatalog catalog =
        HbcSchemaCatalog.admitCatalog(
            List.of(new HbcSchemaCatalog.CatalogEntry(identifier, List.of())),
            List.of(component(List.of(identifier), List.of())));
    HbcSchemaLinks.SchemaCoordinate stale = coordinate("model/id", '2');
    byte[] artifact = HbcSchemaLinks.encode(arithmeticProgram(), List.of(stale));
    HbcFormatException failure =
        assertThrows(
            HbcFormatException.class,
            () -> HbcSchemaCatalog.admitLinkedProgram(artifact, catalog));
    assertTrue(failure.getMessage().contains("is not admitted"));
  }

  @Test
  public void forgedComponentEvidenceIsRejectedAtomically() {
    HbcSchemaLinks.SchemaCoordinate identifier = coordinate("model/id", '1');
    HbcSchemaLinks.SchemaCoordinate profile = coordinate("model/profile", '2');
    HbcSchemaCatalog.CatalogComponent forged =
        component(List.of(identifier, profile), List.of());
    HbcFormatException failure =
        assertThrows(
            HbcFormatException.class,
            () ->
                HbcSchemaCatalog.admitCatalog(
                    List.of(
                        new HbcSchemaCatalog.CatalogEntry(identifier, List.of()),
                        new HbcSchemaCatalog.CatalogEntry(profile, List.of(identifier))),
                    List.of(forged)));
    assertEquals(
        "schema catalog component evidence does not match dependency graph",
        failure.getMessage());
  }

  @Test
  public void validSelfRecursionRemainsOneAdmittedComponent() {
    HbcSchemaLinks.SchemaCoordinate node = coordinate("tree/node", '3');
    HbcSchemaCatalog.AdmittedCatalog catalog =
        HbcSchemaCatalog.admitCatalog(
            List.of(new HbcSchemaCatalog.CatalogEntry(node, List.of(node))),
            List.of(component(List.of(node), List.of())));
    assertEquals(1, catalog.entries().size());
    assertEquals(1, catalog.componentOrder().size());
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
