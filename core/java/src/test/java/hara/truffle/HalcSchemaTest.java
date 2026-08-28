package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import java.util.List;
import java.util.Map;
import hara.lang.data.Keyword;
import org.junit.Test;

public class HalcSchemaTest {
  @Test
  public void normalizesNestedNamedFunctionSchemas() {
    Object schema =
        HaraLanguage.readAll("[:fn [#'demo/Customer & :int] [:maybe :str]]", "schema.hal")[0];
    assertEquals(
        new HalcSchema.FunctionType(
            List.of(
                new HalcSchema.Function(
                    List.of(new HalcSchema.Reference("demo/Customer")),
                    new HalcSchema.Primitive("int"),
                    new HalcSchema.Union(
                        List.of(
                            new HalcSchema.Primitive("str"),
                            new HalcSchema.Primitive("nil")))))),
        HalcSchema.normalize(schema));
  }

  @Test
  public void rejectsMalformedKnownSchemaForms() {
    for (String source :
        List.of("[:map [:name]]", "[:fn [:str & :int :bool] :str]", "[:maybe]")) {
      Object schema = HaraLanguage.readAll(source, "schema.hal")[0];
      assertTrue(
          assertThrows(HaraException.class, () -> HalcSchema.normalize(schema))
              .getMessage()
              .contains("schema"));
    }
  }

  @Test
  public void normalizesIntegerAsTheLongOrBigIntegerUnion() {
    HalcSchema.Type expected =
        new HalcSchema.Union(
            List.of(new HalcSchema.Primitive("long"), new HalcSchema.Primitive("bigint")));
    assertEquals(expected, HalcSchema.normalize(HaraLanguage.readAll(":integer", "schema.hal")[0]));
    assertEquals(expected, HalcSchema.normalize(HaraLanguage.readAll("[:integer]", "schema.hal")[0]));
  }

  @Test
  public void normalizesNamedDeclarationFieldsIntoOneMutableStructSchema() {
    Object fieldForm = HaraLanguage.readAll("[position :int]", "schema.hal")[0];
    HalcSchema.NamedField field = HalcSchema.normalizeNamedField(fieldForm);
    assertEquals("position", field.name());
    assertEquals(new HalcSchema.Primitive("int"), field.type());

    HalcSchema.Type normalized =
        HalcSchema.normalize(
            HalcSchema.namedTypeSchema(
                "demo/Cursor", true, new HalcSchema.NamedField[] {field}));
    assertEquals(
        new HalcSchema.StructType(
            "demo/Cursor",
            true,
            List.of(new HalcSchema.Field(Keyword.create("position"), null,
                new HalcSchema.Primitive("int")))),
        normalized);
  }

  @Test
  public void infersBodyResultsSeparatelyFromDeclaredContracts() {
    Object[] forms =
        HaraLanguage.readAll(
            "(ns demo)\n"
                + "(def Unary [:fn [:int] :number])\n"
                + "(defn ^{:schema #'demo/Unary} choose [value] "
                + "  (let [next (+ value 1)] (if true next 0)))\n"
                + "(defn labels [] {:name \"Ada\" :active true})\n"
                + "(defn select ([value] value) ([left right] right))",
            "typed.hal");
    Map<String, HalcSchema.Type> declarations =
        Map.of("demo/choose", new HalcSchema.Reference("demo/Unary"));
    Map<String, HalcSchema.Type> definitions =
        Map.of(
            "demo/Unary",
            HalcSchema.normalize(HaraLanguage.readAll("[:fn [:int] :number]", "typed.hal")[0]));

    Map<String, HalcSchema.Type> inferred =
        HalcSchema.inferFunctionTypes("demo", forms, declarations, definitions);
    HalcSchema.Function choose =
        ((HalcSchema.FunctionType) inferred.get("demo/choose")).arities().get(0);
    assertEquals(List.of(new HalcSchema.Primitive("int")), choose.fixed());
    assertEquals(new HalcSchema.Primitive("long"), choose.output());
    HalcSchema.Function labels =
        ((HalcSchema.FunctionType) inferred.get("demo/labels")).arities().get(0);
    assertTrue(labels.output() instanceof HalcSchema.MapType);
    assertEquals(
        2,
        ((HalcSchema.FunctionType) inferred.get("demo/select")).arities().size());
  }
}
