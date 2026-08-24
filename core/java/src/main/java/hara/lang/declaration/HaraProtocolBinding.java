package hara.lang.declaration;

import java.lang.annotation.Documented;
import java.lang.annotation.ElementType;
import java.lang.annotation.Retention;
import java.lang.annotation.RetentionPolicy;
import java.lang.annotation.Target;

/** Declares the Hara protocol identity represented by a Java interface. */
@Documented
@Retention(RetentionPolicy.RUNTIME)
@Target(ElementType.TYPE)
public @interface HaraProtocolBinding {
  String namespace();

  String name();

  String[] parents() default {};

  HaraAvailability availability() default HaraAvailability.PORTABLE;

  String capability() default "";
}
