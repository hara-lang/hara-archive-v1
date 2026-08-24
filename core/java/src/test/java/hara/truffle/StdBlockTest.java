package hara.truffle;

import static org.junit.Assert.assertEquals;

import org.graalvm.polyglot.Context;
import org.junit.Test;

public class StdBlockTest {
  @Test
  public void providerPreservesSourceValuesAndPersistentEdits() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(
          "[\"[1 #_2 3]\" [1 3] \"[1 #_3 3]\" \"(if ready [1 2] [3 4])\"]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns std-lib-block-truffle-probe "
                      + "(:require [std.block :as block] "
                      + "          [std.block.navigate :as navigate] "
                      + "          [std.lib.zip :as zip])) "
                      + "(let [original (block/parse-first \"[1 #_2 3]\") "
                      + "      location (zip/step-right "
                      + "                (zip/step-right "
                      + "                  (zip/step-right "
                      + "                    (zip/step-inside "
                      + "                      (navigate/navigator original))))) "
                      + "      edited (zip/root-element "
                      + "               (zip/replace-right location (block/block 3)))] "
                      + "  (pr-str [(block/string original) "
                      + "           (block/value original) "
                      + "           (block/string edited) "
                      + "           (block/string "
                      + "             (block/layout '(if ready [1 2] [3 4]) "
                      + "                           {:width 10}))]))")
              .asString());
    }
  }
}
