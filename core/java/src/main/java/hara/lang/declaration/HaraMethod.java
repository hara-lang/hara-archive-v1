package hara.lang.declaration;

import java.lang.annotation.Documented;
import java.lang.annotation.ElementType;
import java.lang.annotation.Retention;
import java.lang.annotation.RetentionPolicy;
import java.lang.annotation.Target;

/** Declares the Hara method name represented by a Java protocol method. */
@Documented
@Retention(RetentionPolicy.CLASS)
@Target(ElementType.METHOD)
public @interface HaraMethod {
  int UNSPECIFIED_ARITY = Integer.MIN_VALUE;

  String value();

  /**
   * Hara call arity, including the protocol receiver. An unspecified value is derived from the
   * Java method signature by the declaration processor.
   */
  int arity() default UNSPECIFIED_ARITY;

  /** Marks a method whose Hara call accepts a variable number of arguments. */
  boolean variadic() default false;
}
