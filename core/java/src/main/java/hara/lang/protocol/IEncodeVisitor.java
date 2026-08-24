package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(namespace = "std.protocol.iencodevisitor", name = "IEncodeVisitor")
public interface IEncodeVisitor {
  @HaraMethod(value = "visit-nil", arity = 1) Object visitNil();
  @HaraMethod(value = "visit-boolean", arity = 2) Object visitBoolean(Object value);
  @HaraMethod(value = "visit-number", arity = 2) Object visitNumber(Object value);
  @HaraMethod(value = "visit-character", arity = 2) Object visitCharacter(Object value);
  @HaraMethod(value = "visit-string", arity = 2) Object visitString(Object value);
  @HaraMethod(value = "visit-keyword", arity = 2) Object visitKeyword(Object value);
  @HaraMethod(value = "visit-symbol", arity = 2) Object visitSymbol(Object value);
  @HaraMethod(value = "visit-seq", arity = 2) Object visitSeq(Object value);
  @HaraMethod(value = "visit-vector", arity = 2) Object visitVector(Object value);
  @HaraMethod(value = "visit-map", arity = 2) Object visitMap(Object value);
  @HaraMethod(value = "visit-set", arity = 2) Object visitSet(Object value);
  @HaraMethod(value = "visit-tagged", arity = 3) Object visitTagged(Object tag, Object value);
  @HaraMethod(value = "visit-unknown", arity = 2) Object visitUnknown(Object value);
}
