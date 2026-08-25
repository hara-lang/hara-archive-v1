package hara.truffle.bytecode;

import java.io.ByteArrayOutputStream;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.CharacterCodingException;
import java.nio.charset.CodingErrorAction;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Comparator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * HBC1 exact identified-schema links layered over one unchanged canonical HBC0 program.
 *
 * <p>The first catalog artifact epoch is external-link-only. Runtime installation must resolve
 * every exact coordinate through an admitted catalog and must never substitute a tooling-oriented
 * fallback.
 */
public final class HbcSchemaLinks {
  private static final byte[] MAGIC = {'H', 'B', 'C', '1'};
  private static final int DIGEST_BYTES = 32;
  private static final String HASH_PREFIX = "sha256:";
  private static final Comparator<SchemaCoordinate> COORDINATE_ORDER =
      Comparator.comparing(SchemaCoordinate::id)
          .thenComparing(SchemaCoordinate::hash);

  private HbcSchemaLinks() {}

  /** One immutable exact schema coordinate. The id excludes the leading keyword colon. */
  public record SchemaCoordinate(String id, String hash) {
    public SchemaCoordinate {
      validateId(id);
      hashBytes(hash);
    }
  }

  /** One decoded canonical HBC0 program plus its exact external schema dependencies. */
  public record LinkedProgram(HbcProgram program, List<SchemaCoordinate> schemaLinks) {
    public LinkedProgram {
      if (program == null) throw malformed("linked bytecode program is required");
      if (schemaLinks == null) throw malformed("linked bytecode schema links are required");
      schemaLinks = List.copyOf(schemaLinks);
    }
  }

  public static byte[] encode(HbcProgram program, List<SchemaCoordinate> schemaLinks) {
    if (program == null) throw malformed("linked bytecode program is required");
    List<SchemaCoordinate> canonical = canonicalLinks(schemaLinks);
    Writer payload = new Writer();
    payload.bytes(HbcCodec.encode(program));
    payload.many(canonical, coordinate -> writeCoordinate(payload, coordinate));
    byte[] bytes = payload.toByteArray();
    Writer artifact = new Writer();
    artifact.raw(MAGIC);
    artifact.u32(bytes.length);
    artifact.raw(bytes);
    artifact.raw(sha256(bytes));
    return artifact.toByteArray();
  }

  public static LinkedProgram decode(byte[] artifact) {
    if (artifact == null) throw malformed("linked bytecode artifact is required");
    byte[] payload = decodeEnvelope(artifact);
    Reader input = new Reader(payload);
    HbcProgram program = HbcCodec.decode(input.bytes());
    List<SchemaCoordinate> schemaLinks = input.many(HbcSchemaLinks::readCoordinate);
    input.finish();
    List<SchemaCoordinate> canonical = canonicalLinks(schemaLinks);
    if (!canonical.equals(schemaLinks)) {
      throw malformed("linked bytecode artifact has non-canonical schema link order");
    }
    return new LinkedProgram(program, schemaLinks);
  }

  private static List<SchemaCoordinate> canonicalLinks(List<SchemaCoordinate> schemaLinks) {
    if (schemaLinks == null) throw malformed("linked bytecode schema links are required");
    ArrayList<SchemaCoordinate> values = new ArrayList<>(schemaLinks);
    values.sort(COORDINATE_ORDER);
    Map<String, String> identities = new LinkedHashMap<>();
    for (SchemaCoordinate coordinate : values) {
      if (coordinate == null) throw malformed("linked bytecode schema coordinate is required");
      String identity = coordinate.id();
      String existing = identities.put(identity, coordinate.hash());
      if (existing != null) {
        if (existing.equals(coordinate.hash())) {
          throw malformed("linked bytecode artifact contains duplicate schema coordinate");
        }
        throw malformed("linked bytecode artifact contains conflicting schema identity");
      }
    }
    return List.copyOf(values);
  }

  private static void validateId(String id) {
    if (id == null) throw malformed("linked bytecode schema id is required");
    int separator = id.indexOf('/');
    if (separator <= 0
        || separator != id.lastIndexOf('/')
        || separator == id.length() - 1
        || id.startsWith(":")
        || id.chars().anyMatch(Character::isWhitespace)) {
      throw malformed("linked bytecode schema id must be a qualified keyword name");
    }
  }

  private static byte[] hashBytes(String hash) {
    if (hash == null || !hash.startsWith(HASH_PREFIX)) {
      throw malformed("linked bytecode schema hash must use sha256");
    }
    String digest = hash.substring(HASH_PREFIX.length());
    if (digest.length() != DIGEST_BYTES * 2) {
      throw malformed("linked bytecode schema hash must be canonical lowercase hex");
    }
    byte[] output = new byte[DIGEST_BYTES];
    for (int index = 0; index < output.length; index++) {
      int offset = index * 2;
      char high = digest.charAt(offset);
      char low = digest.charAt(offset + 1);
      if (!lowerHex(high) || !lowerHex(low)) {
        throw malformed("linked bytecode schema hash must be canonical lowercase hex");
      }
      output[index] = (byte) ((Character.digit(high, 16) << 4) | Character.digit(low, 16));
    }
    return output;
  }

  private static boolean lowerHex(char value) {
    return (value >= '0' && value <= '9') || (value >= 'a' && value <= 'f');
  }

  private static String displayHash(byte[] digest) {
    StringBuilder output = new StringBuilder(HASH_PREFIX);
    for (byte value : digest) output.append(String.format("%02x", Byte.toUnsignedInt(value)));
    return output.toString();
  }

  private static void writeCoordinate(Writer output, SchemaCoordinate coordinate) {
    output.string(coordinate.id());
    output.raw(hashBytes(coordinate.hash()));
  }

  private static SchemaCoordinate readCoordinate(Reader input) {
    String id = input.string();
    String hash = displayHash(input.raw(DIGEST_BYTES));
    return new SchemaCoordinate(id, hash);
  }

  private static byte[] decodeEnvelope(byte[] artifact) {
    if (artifact.length < MAGIC.length + Integer.BYTES + DIGEST_BYTES) {
      throw malformed("linked bytecode artifact is truncated");
    }
    if (!Arrays.equals(MAGIC, Arrays.copyOf(artifact, MAGIC.length))) {
      throw malformed("linked bytecode artifact has invalid magic");
    }
    long payloadLength = Integer.toUnsignedLong(ByteBuffer.wrap(artifact, 4, 4).getInt());
    long payloadEnd = 8L + payloadLength;
    if (payloadEnd > Integer.MAX_VALUE || payloadEnd + DIGEST_BYTES != artifact.length) {
      throw malformed("linked bytecode artifact length mismatch");
    }
    byte[] payload = Arrays.copyOfRange(artifact, 8, (int) payloadEnd);
    byte[] expected = Arrays.copyOfRange(artifact, (int) payloadEnd, artifact.length);
    if (!MessageDigest.isEqual(sha256(payload), expected)) {
      throw malformed("linked bytecode artifact checksum mismatch");
    }
    return payload;
  }

  private static byte[] sha256(byte[] bytes) {
    try {
      return MessageDigest.getInstance("SHA-256").digest(bytes);
    } catch (NoSuchAlgorithmException impossible) {
      throw new AssertionError(impossible);
    }
  }

  private static HbcFormatException malformed(String message) {
    return new HbcFormatException(message);
  }

  @FunctionalInterface
  private interface ReaderFunction<T> {
    T read(Reader reader);
  }

  @FunctionalInterface
  private interface WriterConsumer<T> {
    void write(T value);
  }

  private static final class Reader {
    private final ByteBuffer input;

    Reader(byte[] bytes) {
      input = ByteBuffer.wrap(bytes).order(ByteOrder.BIG_ENDIAN);
    }

    long u32() {
      require(4);
      return Integer.toUnsignedLong(input.getInt());
    }

    byte[] raw(int size) {
      require(size);
      byte[] result = new byte[size];
      input.get(result);
      return result;
    }

    byte[] bytes() {
      long size = u32();
      if (size > Integer.MAX_VALUE) throw malformed("linked bytecode artifact length overflow");
      return raw((int) size);
    }

    String string() {
      try {
        return StandardCharsets.UTF_8
            .newDecoder()
            .onMalformedInput(CodingErrorAction.REPORT)
            .onUnmappableCharacter(CodingErrorAction.REPORT)
            .decode(ByteBuffer.wrap(bytes()))
            .toString();
      } catch (CharacterCodingException error) {
        throw malformed("linked bytecode artifact contains invalid UTF-8");
      }
    }

    <T> List<T> many(ReaderFunction<T> function) {
      long size = u32();
      if (size > Integer.MAX_VALUE) throw malformed("linked bytecode artifact length overflow");
      ArrayList<T> values = new ArrayList<>(Math.min((int) size, 4096));
      for (int index = 0; index < size; index++) values.add(function.read(this));
      return values;
    }

    void finish() {
      if (input.hasRemaining()) {
        throw malformed("linked bytecode artifact has trailing payload bytes");
      }
    }

    private void require(int size) {
      if (size < 0 || input.remaining() < size) {
        throw malformed("linked bytecode artifact is truncated");
      }
    }
  }

  private static final class Writer {
    private final ByteArrayOutputStream output = new ByteArrayOutputStream();

    void u32(long value) {
      if (value < 0 || value > 0xffffffffL) {
        throw malformed("linked bytecode field does not fit u32");
      }
      for (int shift = 24; shift >= 0; shift -= 8) output.write((int) (value >>> shift));
    }

    void raw(byte[] value) {
      output.writeBytes(value);
    }

    void bytes(byte[] value) {
      u32(value.length);
      raw(value);
    }

    void string(String value) {
      bytes(value.getBytes(StandardCharsets.UTF_8));
    }

    <T> void many(List<T> values, WriterConsumer<T> consumer) {
      u32(values.size());
      for (T value : values) consumer.write(value);
    }

    byte[] toByteArray() {
      return output.toByteArray();
    }
  }
}
