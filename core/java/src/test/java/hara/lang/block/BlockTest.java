package hara.lang.block;

import hara.lang.data.HaraCharacter;
import hara.lang.data.Vector;
import org.junit.Test;

import static org.junit.Assert.*;

public class BlockTest {

  @Test
  public void testIsComment() {
    assertTrue(Block.isComment("; a comment"));
    assertTrue(Block.isComment(";; another comment"));
    assertFalse(Block.isComment("not a comment"));
    assertFalse(Block.isComment(""));
    assertFalse(Block.isComment(null));
  }

  @Test
  public void testVoidTag() {
    assertEquals("eof", Block.voidTag(null));
    assertEquals("linebreak", Block.voidTag('\n'));
    assertEquals("linebreak", Block.voidTag('\r'));
    assertEquals("linebreak", Block.voidTag('\f'));
    assertEquals("linespace", Block.voidTag(' '));
    assertEquals("linespace", Block.voidTag('\t'));
    assertNull(Block.voidTag('a'));
    assertNull(Block.voidTag('1'));
  }

  @Test
  public void testTokenTag() {
    assertEquals("string", Block.tokenTag("hello"));
    assertEquals("number", Block.tokenTag(123));
    assertEquals("number", Block.tokenTag(123.45));
    assertEquals("boolean", Block.tokenTag(true));
    assertEquals("character", Block.tokenTag('c'));
    assertEquals("character", Block.tokenTag(HaraCharacter.of('c')));
    assertEquals("object", Block.tokenTag(new Object()));
    assertEquals("object", Block.tokenTag(null)); // null is not a specific token type
  }

  @Test
  public void testVoidBlockCreation() {
    Block.Void voidBlock = new Block.Void("linespace", ' ', 1, 0);

    assertEquals("void", voidBlock.type());
    assertEquals("linespace", voidBlock.tag());
    assertEquals(" ", voidBlock.string());
    assertEquals(1, voidBlock.length());
    assertEquals(1, voidBlock.width());
    assertEquals(0, voidBlock.height());
    assertTrue(voidBlock.verify());
  }

  @Test
  public void testCommentBlockCreation() {
    String commentText = "; this is a comment";
    Block.Comment commentBlock = new Block.Comment(commentText);

    assertEquals("comment", commentBlock.type());
    assertEquals("comment", commentBlock.tag());
    assertEquals(commentText, commentBlock.string());
    assertEquals(commentText.length(), commentBlock.length());
    assertEquals(commentText.length(), commentBlock.width());
    assertEquals(0, commentBlock.height());
    assertTrue(commentBlock.verify());
  }

  @Test
  public void testTokenBlockCreation() {
    Block.Token tokenBlock = new Block.Token("string", "\"hello\"", "hello", "\"hello\"", 7, 0);

    assertEquals("token", tokenBlock.type());
    assertEquals("string", tokenBlock.tag());
    assertEquals("\"hello\"", tokenBlock.string());
    assertEquals("hello", tokenBlock.value());
    assertEquals(7, tokenBlock.width());
    assertTrue(tokenBlock.verify());
  }

  @Test
  public void testModifierBlockCreation() {
    Block.Modifier modifierBlock = new Block.Modifier("uneval", "#_", (acc, input) -> acc);

    assertEquals("modifier", modifierBlock.type());
    assertEquals("uneval", modifierBlock.tag());
    assertEquals("#_", modifierBlock.string());
    assertNotNull(modifierBlock.command);
  }

  @Test
  public void testContainerImmutability() {
    Vector<Block.IBlock> children =
        Vector.Standard.from(null, new Block.Token("number", "1", 1, "1", 1, 0));
    Block.Container container =
        new Block.Container("vector", children, new Block.Container.Props("[", "]"));

    try {
      ((Vector.Mutable<Block.IBlock>) container.children())
          .pushLast(new Block.Token("number", "2", 2, "2", 1, 0));
      fail("Expected ClassCastException");
    } catch (ClassCastException e) {
      // Test passed
    }
  }

  @Test
  public void testContainerSingleLineWidth() {
    Vector<Block.IBlock> children =
        Vector.Standard.from(
            null,
            new Block.Token("number", "1", 1, "1", 1, 0),
            new Block.Void("linespace", ' ', 1, 0),
            new Block.Token("number", "2", 2, "2", 1, 0));
    Block.Container container =
        new Block.Container("vector", children, new Block.Container.Props("[", "]"));
    assertEquals(5, container.width()); // [1 2] -> 1 + 1 + 1 + 1 + 1
  }

  @Test
  public void testContainerMultiLineWidth() {
    Vector<Block.IBlock> children =
        Vector.Standard.from(
            null,
            new Block.Token("number", "1", 1, "1", 1, 0),
            new Block.Void("linebreak", '\n', 0, 1),
            new Block.Token("number", "2", 2, "2", 1, 0));
    Block.Container container =
        new Block.Container("vector", children, new Block.Container.Props("[", "]"));
    assertEquals(2, container.width()); // 2] -> 1 + 1
  }

  @Test
  public void testContainerHeight() {
    Vector<Block.IBlock> children =
        Vector.Standard.from(
            null,
            new Block.Token("number", "1", 1, "1", 1, 0),
            new Block.Void("linebreak", '\n', 0, 1),
            new Block.Token("number", "2", 2, "2", 1, 0),
            new Block.Void("linebreak", '\n', 0, 1));
    Block.Container container =
        new Block.Container("vector", children, new Block.Container.Props("[", "]"));
    assertEquals(2, container.height());
  }

  @Test
  public void testModifierMethods() {
    Block.Modifier modifier = new Block.Modifier("uneval", "#_", (acc, input) -> input);

    assertEquals(2, modifier.width());
    assertEquals(0, modifier.height());
    assertEquals(0, modifier.prefixed());
    assertEquals(0, modifier.suffixed());
    assertTrue(modifier.verify());
    assertEquals("#_", modifier.toString());
    assertEquals("#_", modifier.display());
    assertEquals(hara.lang.protocol.Constant.ObjType.CLASS, modifier.getObjType());
    assertEquals("BLOCK", modifier.getObjName());
    assertNull(modifier.meta());
    assertSame(modifier, modifier.withMeta(null));
    assertEquals(0, modifier.hashCalc(null));

    Object result = modifier.modify(null, "test");
    assertEquals("test", result);
  }

  @Test
  public void testContainerMethods() {
    Vector<Block.IBlock> children =
        Vector.Standard.from(null, new Block.Token("number", "1", 1, "1", 1, 0));
    Block.Container container =
        new Block.Container("vector", children, new Block.Container.Props("[", "]"));

    assertEquals("collection", container.type());
    assertEquals("vector", container.tag());
    assertEquals(1, container.prefixed());
    assertEquals(1, container.suffixed());
    assertTrue(container.verify());
    assertNull(container.value());
    assertEquals("[1]", container.string());
    assertEquals(3, container.length());
    assertEquals("[1]", container.toString());
    assertEquals("[1]", container.display());
    assertEquals(hara.lang.protocol.Constant.ObjType.CLASS, container.getObjType());
    assertEquals("BLOCK", container.getObjName());
    assertNull(container.meta());
    assertSame(container, container.withMeta(null));
    assertEquals(0, container.hashCalc(null));
  }

  @Test
  public void testContainerReplaceChildren() {
    Vector<Block.IBlock> children = Vector.Standard.from(null);
    Block.Container container =
        new Block.Container("vector", children, new Block.Container.Props("[", "]"));

    Vector<Block.IBlock> newChildren =
        Vector.Standard.from(null, new Block.Token("number", "1", 1, "1", 1, 0));
    Block.IContainer newContainer = container.replaceChildren(newChildren);

    assertEquals(1, newContainer.children().count());
    assertEquals(children, container.children());
  }

  @Test
  public void testContainerValueString() {
    Vector<Block.IBlock> children =
        Vector.Standard.from(
            null,
            new Block.Token("number", "1", 1, "1", 1, 0),
            new Block.Void("linespace", ' ', 1, 0),
            new Block.Token("number", "2", 2, "2", 1, 0));
    Block.Container container =
        new Block.Container("vector", children, new Block.Container.Props("[", "]"));

    assertEquals("[1 2]", container.valueString());
  }

  @Test
  public void testCompare() {
    Block.Token t1 = new Block.Token("a", "a", "a", "a", 1, 0);
    Block.Token t2 = new Block.Token("b", "b", "b", "b", 1, 0);
    Block.Token t3 = new Block.Token("a", "a", "a", "a", 1, 0);

    assertTrue(t1.compareTo(t2) < 0);
    assertTrue(t2.compareTo(t1) > 0);
    assertEquals(0, t1.compareTo(t3));
  }
}
