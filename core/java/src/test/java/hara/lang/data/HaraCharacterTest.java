package hara.lang.data;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertThrows;

import org.junit.Test;

public class HaraCharacterTest {
  @Test
  public void representsUnicodeScalarsIncludingSupplementaryCodePoints() {
    HaraCharacter character = HaraCharacter.of(0x1F600);

    assertEquals(0x1F600, character.codePoint());
    assertEquals("😀", character.text());
    assertEquals("\\😀", character.display());
    assertEquals(character, HaraCharacter.of(0x1F600));
    assertEquals(0, character.compareTo(HaraCharacter.of(0x1F600)));
  }

  @Test
  public void rejectsNonScalarCodePoints() {
    assertThrows(IllegalArgumentException.class, () -> HaraCharacter.of(-1));
    assertThrows(IllegalArgumentException.class, () -> HaraCharacter.of(0x110000));
    assertThrows(IllegalArgumentException.class, () -> HaraCharacter.of(0xD800));
  }
}
