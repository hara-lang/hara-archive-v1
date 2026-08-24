package hara.truffle;

import hara.kernel.base.Parser;
import hara.lang.data.Keyword;
import hara.lang.protocol.ILinearType;
import hara.lang.protocol.IMapType;
import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

/**
 * Data-defined command router backed by the normative Hara CLI EDN manifest.
 *
 * <p>The manifest contains stable handler IDs only. Executable dispatch remains
 * a closed switch in the Truffle CLI.
 */
final class HaraCliRouter {
  private static final String RESOURCE = "02-platform/000001-cli/draft/hara-cli.edn";
  private static final HaraCliRouter INSTANCE = load();

  record Route(
      String id,
      List<String> path,
      List<List<String>> aliases,
      String handler,
      String execution,
      String tier,
      String summary) {}

  record Resolution(Route route, List<String> arguments, boolean alias) {}

  private record Match(List<String> path, Route route, boolean alias) {}

  private final List<Route> routes;

  private HaraCliRouter(List<Route> routes) {
    this.routes = List.copyOf(routes);
    verify();
  }

  static HaraCliRouter instance() {
    return INSTANCE;
  }

  List<Route> publicRoutes() {
    return routes.stream().filter(route -> "public".equals(route.tier())).toList();
  }

  Resolution resolve(String[] arguments) {
    List<String> argv = List.of(arguments);
    if (argv.isEmpty()) return resolveId("hara.cli.route/repl", List.of(), false);
    if (argv.equals(List.of("run")))
      return resolveId("hara.cli.route/project-run", List.of(), true);
    ArrayList<Match> matches = new ArrayList<>();
    for (Route route : routes) {
      if (startsWith(argv, route.path())) matches.add(new Match(route.path(), route, false));
      for (List<String> alias : route.aliases()) {
        if (startsWith(argv, alias)) matches.add(new Match(alias, route, true));
      }
    }
    return matches.stream()
        .sorted(
            Comparator.comparingInt((Match match) -> match.path().size())
                .reversed()
                .thenComparing(Match::alias))
        .findFirst()
        .map(
            match -> {
              List<String> remaining =
                  new ArrayList<>(argv.subList(match.path().size(), argv.size()));
              if (!remaining.isEmpty() && "--".equals(remaining.get(0))) remaining.remove(0);
              return new Resolution(match.route(), List.copyOf(remaining), match.alias());
            })
        .orElse(null);
  }

  static int outcomeExit(String outcome) {
    return switch (outcome) {
      case "hara.cli.outcome/success" -> 0;
      case "hara.cli.outcome/failed" -> 1;
      case "hara.cli.outcome/interrupted" -> 130;
      case "hara.cli.outcome/usage-error",
          "hara.cli.outcome/read-error",
          "hara.cli.outcome/resolution-error",
          "hara.cli.outcome/unavailable",
          "hara.cli.outcome/internal-error" -> 2;
      default -> throw new IllegalArgumentException("Unknown CLI outcome: " + outcome);
    };
  }

  String[] normalize(String[] arguments) {
    if (arguments.length > 0
        && java.util.Set.of(
                "help",
                "compile-halc",
                "foundation-halc-benchmark",
                "conformance",
                "benchmark",
                "fabric",
                "service")
            .contains(arguments[0])) return arguments;
    if (List.of(arguments).equals(List.of("standalone"))) return arguments;
    Resolution resolution = resolve(arguments);
    if (resolution == null) return arguments;
    String command =
        switch (resolution.route().handler()) {
          case "hara.cli.handler/eval" -> "eval";
          case "hara.cli.handler/run-file" -> "run";
          case "hara.cli.handler/stdin" -> "stdin";
          case "hara.cli.handler/repl" -> "repl";
          case "hara.cli.handler/server" -> "server";
          case "hara.cli.handler/remote" -> "remote";
          case "hara.cli.handler/project-new" -> "new";
          case "hara.cli.handler/project-check" -> "check";
          case "hara.cli.handler/project-run" -> "run";
          case "hara.cli.handler/project-test" -> "test";
          case "hara.cli.handler/project-add" -> "add";
          case "hara.cli.handler/project-remove" -> "remove";
          case "hara.cli.handler/project-sync" -> "sync";
          case "hara.cli.handler/project-update" -> "update";
          case "hara.cli.handler/package" -> "package";
          case "hara.cli.handler/spec" -> "spec";
          case "hara.cli.handler/extension" -> "extension";
          case "hara.cli.handler/identity" -> "id";
          case "hara.cli.handler/asset" -> "asset";
          case "hara.cli.handler/tap" -> "tap";
          default -> null;
        };
    if (command == null) return arguments;
    ArrayList<String> normalized = new ArrayList<>();
    normalized.add(command);
    if (!"hara.cli.route/package-extension".equals(resolution.route().id())
        && java.util.Set.of(
            "hara.cli.handler/package",
            "hara.cli.handler/spec",
            "hara.cli.handler/extension",
            "hara.cli.handler/identity",
            "hara.cli.handler/asset",
            "hara.cli.handler/tap")
        .contains(resolution.route().handler()))
      normalized.addAll(resolution.route().path().subList(1, resolution.route().path().size()));
    normalized.addAll(resolution.arguments());
    return normalized.toArray(new String[0]);
  }

  private Resolution resolveId(String id, List<String> arguments, boolean alias) {
    return routes.stream()
        .filter(route -> id.equals(route.id()))
        .findFirst()
        .map(route -> new Resolution(route, arguments, alias))
        .orElse(null);
  }

  private void verify() {
    Map<List<String>, String> paths = new HashMap<>();
    for (Route route : routes) {
      if (!route.id().contains("/"))
        throw new IllegalStateException("CLI route has invalid stable ID: " + route.id());
      ArrayList<List<String>> all = new ArrayList<>(route.aliases());
      all.add(route.path());
      for (List<String> path : all) {
        String existing = paths.put(path, route.id());
        if (existing != null)
          throw new IllegalStateException(
              "Ambiguous CLI path "
                  + String.join(" ", path)
                  + ": "
                  + existing
                  + " and "
                  + route.id());
      }
    }
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  private static HaraCliRouter load() {
    try (InputStream input =
        HaraCliRouter.class.getClassLoader().getResourceAsStream(RESOURCE)) {
      if (input == null) throw new IllegalStateException("Missing embedded CLI manifest: " + RESOURCE);
      Object document =
          Parser.LispReader.readString(
              new String(input.readAllBytes(), StandardCharsets.UTF_8), null);
      if (!(document instanceof IMapType map))
        throw new IllegalStateException("CLI manifest must be an EDN map");
      Object value = map.lookup(Keyword.create("cli/routes"));
      if (!(value instanceof ILinearType routes))
        throw new IllegalStateException("CLI manifest requires :cli/routes");
      ArrayList<Route> parsed = new ArrayList<>();
      for (Object route : routes) parsed.add(parseRoute(route));
      return new HaraCliRouter(parsed);
    } catch (IOException exception) {
      throw new IllegalStateException("Cannot read embedded CLI manifest", exception);
    }
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  private static Route parseRoute(Object value) {
    if (!(value instanceof IMapType map))
      throw new IllegalStateException("CLI route must be a map");
    ArrayList<List<String>> aliases = new ArrayList<>();
    Object aliasValue = map.lookup(Keyword.create("route/aliases"));
    if (!(aliasValue instanceof ILinearType aliasValues))
      throw new IllegalStateException("CLI route requires :route/aliases");
    for (Object alias : aliasValues) aliases.add(strings(alias, "route/aliases"));
    return new Route(
        keyword(map, "route/id"),
        strings(map.lookup(Keyword.create("route/path")), "route/path"),
        List.copyOf(aliases),
        keyword(map, "route/handler"),
        keyword(map, "route/execution"),
        keyword(map, "route/tier"),
        string(map, "route/summary"));
  }

  @SuppressWarnings("rawtypes")
  private static String keyword(IMapType map, String key) {
    Object value = map.lookup(Keyword.create(key));
    if (!(value instanceof Keyword keyword))
      throw new IllegalStateException("CLI route requires keyword :" + key);
    return keyword.display().substring(1);
  }

  @SuppressWarnings("rawtypes")
  private static String string(IMapType map, String key) {
    Object value = map.lookup(Keyword.create(key));
    if (!(value instanceof String text))
      throw new IllegalStateException("CLI route requires string :" + key);
    return text;
  }

  private static List<String> strings(Object value, String key) {
    if (!(value instanceof ILinearType<?> values))
      throw new IllegalStateException("CLI route :" + key + " must be a vector");
    ArrayList<String> output = new ArrayList<>();
    for (Object item : values) {
      if (!(item instanceof String text))
        throw new IllegalStateException("CLI route :" + key + " must contain strings");
      output.add(text);
    }
    return List.copyOf(output);
  }

  private static boolean startsWith(List<String> values, List<String> prefix) {
    return values.size() >= prefix.size()
        && values.subList(0, prefix.size()).equals(prefix);
  }
}
