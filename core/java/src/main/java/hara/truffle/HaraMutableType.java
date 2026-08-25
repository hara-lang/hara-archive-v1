package hara.truffle;

import com.oracle.truffle.api.interop.ArityException;

/** A fixed-shape named type whose instances carry reference-identity mutable storage. */
public final class HaraMutableType extends HaraType {
  public HaraMutableType(String name, String[] fields) {
    super(name, fields);
  }

  HaraMutableType(
      String name,
      String[] fields,
      HalcSchema.NamedField[] declarationFields,
      Object schema) {
    super(name, fields, declarationFields, schema);
  }

  @Override
  public Object construct(Object[] values) throws ArityException {
    requireArity(values.length);
    return new HaraMutable(this, values);
  }
}
