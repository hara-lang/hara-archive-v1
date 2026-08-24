package hara.lang.declaration;

import java.lang.annotation.Documented;
import java.lang.annotation.ElementType;
import java.lang.annotation.Repeatable;
import java.lang.annotation.Retention;
import java.lang.annotation.RetentionPolicy;
import java.lang.annotation.Target;

/** Binds a Java catalog owner to one runtime-owned native type declaration. */
@Documented
@Retention(RetentionPolicy.RUNTIME)
@Target(ElementType.TYPE)
@Repeatable(HaraNativeBindings.class)
public @interface HaraNativeBinding {
  String namespace();

  String name();

  String[] methods() default {};

  HaraAvailability availability() default HaraAvailability.PORTABLE;

  String capability() default "";
}
