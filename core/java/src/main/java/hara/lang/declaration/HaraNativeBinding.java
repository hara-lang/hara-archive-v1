package hara.lang.declaration;

import java.lang.annotation.Documented;
import java.lang.annotation.ElementType;
import java.lang.annotation.Retention;
import java.lang.annotation.RetentionPolicy;
import java.lang.annotation.Target;

/** Binds a Java provider to one runtime-owned native type declaration. */
@Documented
@Retention(RetentionPolicy.CLASS)
@Target(ElementType.TYPE)
public @interface HaraNativeBinding {
  /** Declaration owner namespace, for example {@code std.native}. */
  String namespace();

  String name();

  HaraAvailability availability() default HaraAvailability.PORTABLE;

  String capability() default "";
}
