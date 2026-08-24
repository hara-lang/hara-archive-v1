package hara.truffle;

import hara.kernel.base.Parser;
import hara.lang.base.G;
import hara.lang.data.Keyword;
import hara.lang.protocol.IMapType;
import java.io.IOException;
import java.io.PrintStream;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.nio.charset.StandardCharsets;
import java.util.regex.Pattern;

/** External-signer publisher identity client for the Truffle CLI. */
final class HaraIdentityTool {
  private static final Pattern PUBLIC_KEY = Pattern.compile("^[0-9a-f]{64}$");

  private HaraIdentityTool() {}

  static int run(String[] arguments, PrintStream output, PrintStream error) {
    if (arguments.length == 0 || "--help".equals(arguments[0]) || "-h".equals(arguments[0])) {
      usage(output);
      return 0;
    }
    try {
      return switch (arguments[0]) {
        case "login" -> {
          output.println(endpoint() + "/login/github");
          yield 0;
        }
        case "enroll" -> enroll(arguments, output);
        case "status" -> request("GET", "/v1/status", null, output);
        case "namespace" -> request("GET", "/v1/namespaces", null, output);
        case "key" -> key(arguments, output);
        default -> {
          error.println("unknown id command: " + arguments[0]);
          yield 2;
        }
      };
    } catch (HaraException | IOException | InterruptedException exception) {
      if (exception instanceof InterruptedException) Thread.currentThread().interrupt();
      error.println(exception.getMessage());
      return exception instanceof HaraException ? 1 : 2;
    }
  }

  private static int enroll(String[] arguments, PrintStream output)
      throws IOException, InterruptedException {
    String owner = option(arguments, "--owner");
    if (owner == null) throw new HaraException("id enroll requires --owner");
    String tap = option(arguments, "--tap");
    if (tap == null || "official".equals(tap)) tap = "hara";
    String publicKey = System.getenv("HARA_SIGNER_PUBLIC_KEY");
    if (publicKey == null || !PUBLIC_KEY.matcher(publicKey).matches())
      throw new HaraException("id enroll requires lowercase 32-byte HARA_SIGNER_PUBLIC_KEY");
    String challenge = option(arguments, "--challenge");
    if (challenge == null)
      challenge =
          response(
                  "GET",
                  "/v1/enrollments/challenge?owner=" + java.net.URLEncoder.encode(owner, StandardCharsets.UTF_8),
                  null)
              .strip();
    if (challenge.isEmpty()) throw new HaraException("identity service returned an empty challenge");
    String canonical = canonicalEnrollment(tap, owner, publicKey, challenge);
    SignerResponse signed = sign(canonical);
    if (contains(arguments, "--dry-run")) {
      output.print(canonical);
      output.println("key-id=" + signed.keyId() + " signature=" + signed.signature());
      return 0;
    }
    String envelope =
        "{:enrollment/request "
            + G.display(canonical)
            + " :enrollment/key-id "
            + G.display(signed.keyId())
            + " :enrollment/signature "
            + G.display(signed.signature())
            + "}\n";
    return request("POST", "/v1/enrollments", envelope, output);
  }

  private static int key(String[] arguments, PrintStream output)
      throws IOException, InterruptedException {
    if (arguments.length < 2) throw new HaraException("usage: hara id key <list|rotate|revoke KEY_ID>");
    return switch (arguments[1]) {
      case "list" -> request("GET", "/v1/keys", null, output);
      case "rotate" -> request("POST", "/v1/keys/rotate", "{}\n", output);
      case "revoke" -> {
        if (arguments.length < 3) throw new HaraException("id key revoke requires KEY_ID");
        yield request(
            "POST",
            "/v1/keys/" + arguments[2] + "/revocations",
            "{:revocation/reason :publisher-request}\n",
            output);
      }
      default -> throw new HaraException("usage: hara id key <list|rotate|revoke KEY_ID>");
    };
  }

  static String canonicalEnrollment(String tap, String owner, String publicKey, String challenge) {
    return "{:enrollment/format \"0.0.0-alpha\" :enrollment/tap "
        + G.display(tap)
        + " :enrollment/provider :github :enrollment/owner "
        + G.display(owner)
        + " :enrollment/public-key "
        + G.display(publicKey)
        + " :enrollment/challenge "
        + G.display(challenge)
        + "}\n";
  }

  @SuppressWarnings("rawtypes")
  private static SignerResponse sign(String canonical) throws IOException, InterruptedException {
    String signer = System.getenv("HARA_SIGNER");
    if (signer == null) throw new HaraException("HARA_SIGNER must name an external signer command");
    Process process = new ProcessBuilder(signer).start();
    process.getOutputStream().write(canonical.getBytes(StandardCharsets.UTF_8));
    process.getOutputStream().close();
    byte[] response = process.getInputStream().readAllBytes();
    byte[] errors = process.getErrorStream().readAllBytes();
    int status = process.waitFor();
    if (status != 0)
      throw new HaraException(
          "external signer failed: " + new String(errors, StandardCharsets.UTF_8).strip());
    Object value =
        Parser.LispReader.readString(new String(response, StandardCharsets.UTF_8), null);
    if (!(value instanceof IMapType map))
      throw new HaraException("signer response must be an EDN map");
    return new SignerResponse(string(map, "key/id"), string(map, "signature"));
  }

  private static int request(String method, String path, String body, PrintStream output)
      throws IOException, InterruptedException {
    output.print(response(method, path, body));
    return 0;
  }

  private static String response(String method, String path, String body)
      throws IOException, InterruptedException {
    HttpRequest.Builder request =
        HttpRequest.newBuilder(URI.create(endpoint() + path))
            .header("accept", "application/edn");
    if (body == null) request.method(method, HttpRequest.BodyPublishers.noBody());
    else
      request
          .header("content-type", "application/edn")
          .method(method, HttpRequest.BodyPublishers.ofString(body, StandardCharsets.UTF_8));
    HttpResponse<String> response =
        HttpClient.newHttpClient()
            .send(request.build(), HttpResponse.BodyHandlers.ofString(StandardCharsets.UTF_8));
    if (response.statusCode() < 200 || response.statusCode() >= 300)
      throw new HaraException("identity request failed: HTTP " + response.statusCode() + " " + response.body());
    return response.body();
  }

  @SuppressWarnings("rawtypes")
  private static String string(IMapType map, String key) {
    Object value = map.lookup(Keyword.create(key));
    if (!(value instanceof String text))
      throw new HaraException("signer response requires :" + key);
    return text;
  }

  private static String option(String[] arguments, String name) {
    for (int index = 0; index < arguments.length; index++) {
      if (name.equals(arguments[index])) {
        if (index + 1 >= arguments.length) throw new HaraException(name + " requires a value");
        return arguments[index + 1];
      }
    }
    return null;
  }

  private static boolean contains(String[] arguments, String value) {
    return java.util.Arrays.asList(arguments).contains(value);
  }

  private static String endpoint() {
    String value = System.getenv("HARA_ID_ENDPOINT");
    return (value == null ? "https://id.hara-lang.org" : value).replaceAll("/+$", "");
  }

  private static void usage(PrintStream output) {
    output.println("hara id login");
    output.println("hara id enroll --owner OWNER [--tap hara] [--dry-run]");
    output.println("hara id status");
    output.println("hara id namespace");
    output.println("hara id key <list|rotate|revoke KEY_ID>");
  }

  private record SignerResponse(String keyId, String signature) {}
}
