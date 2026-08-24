package hara.lang.declaration;

import java.lang.annotation.Documented;
import java.lang.annotation.ElementType;
import java.lang.annotation.Repeatable;
import java.lang.annotation.Retention;
import java.lang.annotation.RetentionPolicy;
import java.lang.annotation.Target;

/** Declares one built-in implementation of a Hara protocol method. */
@Documented
@Retention(RetentionPolicy.RUNTIME)
@Target(ElementType.METHOD)
@Repeatable(HaraProtocolExtensions.class)
public @interface HaraProtocolExtension {
  /** Java interface carrying the annotated Hara protocol declaration. */
  Class<?> protocol();

  /** Hara method name, including hyphens where applicable. */
  String method();

  /** Java receiver class for {@link HaraProtocolTarget#JAVA_CLASS}. */
  Class<?> receiver() default Void.class;

  /** Receiver category used when resolving this implementation. */
  HaraProtocolTarget target() default HaraProtocolTarget.JAVA_CLASS;

  /** Marks a fixed runtime implementation eligible for specialized dispatch. */
  boolean intrinsic() default false;
}
