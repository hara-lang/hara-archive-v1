package hara.truffle;

import static org.junit.Assert.assertArrayEquals;
import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import hara.lang.data.Keyword;
import hara.lang.protocol.IMapType;
import java.nio.charset.StandardCharsets;
import java.util.HexFormat;
import org.graalvm.polyglot.Context;
import org.junit.Test;

public class NativeCryptoTest {
  private static final HexFormat HEX = HexFormat.of();

  @Test
  public void digestAndHmacMatchStandardVectors() {
    assertEquals(
        "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a"
            + "2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f",
        NativeCrypto.sha512(null, new Object[] {"abc".getBytes(StandardCharsets.UTF_8)}));
    assertEquals(
        "f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8",
        NativeCrypto.hmacSha256(
            null,
            new Object[] {
              "key".getBytes(StandardCharsets.UTF_8),
              "The quick brown fox jumps over the lazy dog".getBytes(StandardCharsets.UTF_8)
            }));
  }

  @Test
  public void ed25519MatchesTheRfc8032EmptyMessageVector() {
    byte[] seed = HEX.parseHex("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60");
    byte[] publicKey = HEX.parseHex("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a");
    byte[] expectedSignature =
        HEX.parseHex(
            "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e06522490155"
                + "5fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b");

    assertArrayEquals(publicKey, (byte[]) NativeCrypto.ed25519Public(null, new Object[] {seed}));
    byte[] signature =
        (byte[]) NativeCrypto.ed25519Sign(null, new Object[] {seed, new byte[0]});
    assertArrayEquals(expectedSignature, signature);
    assertEquals(
        true,
        NativeCrypto.ed25519Verify(null, new Object[] {publicKey, new byte[0], signature}));
  }

  @Test
  public void x25519AndP256ProduceSymmetricSharedSecrets() {
    IMapType aliceX = (IMapType) NativeCrypto.x25519Keypair(null, new Object[0]);
    IMapType bobX = (IMapType) NativeCrypto.x25519Keypair(null, new Object[0]);
    assertSharedSecret("x25519", aliceX, bobX);

    IMapType aliceP = (IMapType) NativeCrypto.p256Keypair(null, new Object[0]);
    IMapType bobP = (IMapType) NativeCrypto.p256Keypair(null, new Object[0]);
    assertSharedSecret("p256", aliceP, bobP);
  }

  @Test
  public void nativeObjectExposesSignVerifyRandomAndStableErrors() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(
          "[128 true true true]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(let [message (bytes 1 2 3)"
                      + " ed (Crypto/ed25519-keypair)"
                      + " ed-signature (Crypto/ed25519-sign (:private ed) message)"
                      + " p (Crypto/p256-keypair)"
                      + " p-signature (Crypto/p256-sign (:private p) message)]"
                      + " [(count (Crypto/random-bytes 128))"
                      + "  (Crypto/secure-equal? message message)"
                      + "  (Crypto/ed25519-verify (:public ed) message ed-signature)"
                      + "  (Crypto/p256-verify (:public p) message p-signature)])")
              .toString());
      assertThrows(
          RuntimeException.class,
          () -> context.eval(HaraLanguage.ID, "(Crypto/ed25519-public (bytes 1))"));
    }
  }

  private static void assertSharedSecret(
      String curve, IMapType alice, IMapType bob) {
    byte[] alicePrivate = (byte[]) alice.lookup(Keyword.create("private"));
    byte[] alicePublic = (byte[]) alice.lookup(Keyword.create("public"));
    byte[] bobPrivate = (byte[]) bob.lookup(Keyword.create("private"));
    byte[] bobPublic = (byte[]) bob.lookup(Keyword.create("public"));
    byte[] left;
    byte[] right;
    if (curve.equals("x25519")) {
      left = (byte[]) NativeCrypto.x25519Shared(null, new Object[] {alicePrivate, bobPublic});
      right = (byte[]) NativeCrypto.x25519Shared(null, new Object[] {bobPrivate, alicePublic});
    } else {
      left = (byte[]) NativeCrypto.p256Shared(null, new Object[] {alicePrivate, bobPublic});
      right = (byte[]) NativeCrypto.p256Shared(null, new Object[] {bobPrivate, alicePublic});
    }
    assertEquals(32, left.length);
    assertArrayEquals(left, right);
    assertTrue((Boolean) NativeCrypto.secureEqual(null, new Object[] {left, right}));
    right[0] ^= 1;
    assertFalse((Boolean) NativeCrypto.secureEqual(null, new Object[] {left, right}));
  }
}
