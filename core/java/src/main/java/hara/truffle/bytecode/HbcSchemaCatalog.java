package hara.truffle.bytecode;

import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.TreeMap;
import java.util.TreeSet;

/** Admits exact std.typed catalog evidence before an HBC1 program becomes usable. */
public final class HbcSchemaCatalog {
  private static final String COMPONENT_EPOCH = ":std.typed.catalog/component-v2";
  private static final String HASH_PREFIX = "sha256:";
  private static final int DIGEST_HEX_LENGTH = 64;
  private static final Comparator<HbcSchemaLinks.SchemaCoordinate> COORDINATE_ORDER =
      Comparator.comparing(HbcSchemaLinks.SchemaCoordinate::id)
          .thenComparing(HbcSchemaLinks.SchemaCoordinate::hash);

  private HbcSchemaCatalog() {}

  private record Identity(String id) {}

  /** One exact catalog entry and its exact direct dependencies. */
  public record CatalogEntry(
      HbcSchemaLinks.SchemaCoordinate coordinate,
      List<HbcSchemaLinks.SchemaCoordinate> dependencies) {
    public CatalogEntry {
      coordinate = requireCoordinate(coordinate);
      dependencies = canonicalCoordinates(dependencies, "catalog entry dependencies");
    }
  }

  /** One deterministic strongly connected component from std.typed.catalog. */
  public record CatalogComponent(
      String id,
      List<HbcSchemaLinks.SchemaCoordinate> members,
      List<String> dependencies) {
    public CatalogComponent {
      validateHash(id, "schema catalog component id");
      members = canonicalCoordinates(members, "schema catalog component members");
      if (members.isEmpty()) {
        throw malformed("schema catalog component requires at least one member");
      }
      dependencies = canonicalComponentDependencies(dependencies);
    }
  }

  /** A catalog whose identities, edges, components, and order were verified atomically. */
  public record AdmittedCatalog(
      Map<HbcSchemaLinks.SchemaCoordinate, CatalogEntry> entries,
      Map<String, CatalogComponent> components,
      List<String> componentOrder) {
    public AdmittedCatalog {
      entries = Map.copyOf(entries);
      components = Map.copyOf(components);
      componentOrder = List.copyOf(componentOrder);
    }
  }

  /** One linked program released with its dependency-first exact closure. */
  public record AdmittedLinkedProgram(
      HbcSchemaLinks.LinkedProgram linked,
      List<HbcSchemaLinks.SchemaCoordinate> resolvedCoordinates) {
    public AdmittedLinkedProgram {
      resolvedCoordinates = List.copyOf(resolvedCoordinates);
    }
  }

  /** Reproduces sha256(pr-str [:std.typed.catalog/component-v2 members]). */
  public static String componentId(List<HbcSchemaLinks.SchemaCoordinate> rawMembers) {
    List<HbcSchemaLinks.SchemaCoordinate> members =
        canonicalCoordinates(rawMembers, "schema catalog component members");
    if (members.isEmpty()) {
      throw malformed("schema catalog component requires at least one member");
    }
    StringBuilder input = new StringBuilder("[").append(COMPONENT_EPOCH).append(" [");
    for (int index = 0; index < members.size(); index++) {
      if (index > 0) input.append(' ');
      HbcSchemaLinks.SchemaCoordinate coordinate = members.get(index);
      input
          .append("[:schema :")
          .append(coordinate.id())
          .append(" \"")
          .append(coordinate.hash())
          .append("\"]");
    }
    input.append("]]" );
    return displayHash(sha256(input.toString().getBytes(StandardCharsets.UTF_8)));
  }

  /** Validates a complete catalog manifest without partially accepting entries. */
  public static AdmittedCatalog admitCatalog(
      List<CatalogEntry> rawEntries, List<CatalogComponent> rawComponents) {
    if (rawEntries == null) throw malformed("schema catalog entries are required");
    if (rawComponents == null) throw malformed("schema catalog components are required");

    Map<HbcSchemaLinks.SchemaCoordinate, CatalogEntry> entries =
        new TreeMap<>(COORDINATE_ORDER);
    Map<Identity, String> identities =
        new TreeMap<>(Comparator.comparing(Identity::id));
    for (CatalogEntry rawEntry : rawEntries) {
      if (rawEntry == null) throw malformed("schema catalog entry is required");
      CatalogEntry entry = new CatalogEntry(rawEntry.coordinate(), rawEntry.dependencies());
      Identity identity = new Identity(entry.coordinate().id());
      String previousHash = identities.put(identity, entry.coordinate().hash());
      if (previousHash != null) {
        if (previousHash.equals(entry.coordinate().hash())) {
          throw malformed("schema catalog contains duplicate exact entry");
        }
        throw malformed("schema catalog contains conflicting immutable identity");
      }
      if (entries.put(entry.coordinate(), entry) != null) {
        throw malformed("schema catalog contains duplicate exact entry");
      }
    }

    for (CatalogEntry entry : entries.values()) {
      for (HbcSchemaLinks.SchemaCoordinate dependency : entry.dependencies()) {
        if (!entries.containsKey(dependency)) {
          throw malformed(
              "schema catalog dependency is not admitted: " + displayCoordinate(dependency));
        }
      }
    }

    Map<HbcSchemaLinks.SchemaCoordinate, Set<HbcSchemaLinks.SchemaCoordinate>> graph =
        entryGraph(entries);
    List<List<HbcSchemaLinks.SchemaCoordinate>> computedComponents =
        HbcSchemaCatalogGraph.stronglyConnectedComponents(graph, COORDINATE_ORDER);

    Map<String, CatalogComponent> components = new TreeMap<>();
    Map<HbcSchemaLinks.SchemaCoordinate, String> owners =
        new TreeMap<>(COORDINATE_ORDER);
    List<List<HbcSchemaLinks.SchemaCoordinate>> declaredComponents = new ArrayList<>();
    for (CatalogComponent rawComponent : rawComponents) {
      if (rawComponent == null) throw malformed("schema catalog component is required");
      CatalogComponent component =
          new CatalogComponent(
              rawComponent.id(), rawComponent.members(), rawComponent.dependencies());
      String expectedId = componentId(component.members());
      if (!component.id().equals(expectedId)) {
        throw malformed("schema catalog component id mismatch: expected " + expectedId);
      }
      if (components.put(component.id(), component) != null) {
        throw malformed("schema catalog contains duplicate component id");
      }
      for (HbcSchemaLinks.SchemaCoordinate member : component.members()) {
        if (!entries.containsKey(member)) {
          throw malformed(
              "schema catalog component member is not admitted: " + displayCoordinate(member));
        }
        if (owners.put(member, component.id()) != null) {
          throw malformed(
              "schema catalog entry belongs to multiple components: "
                  + displayCoordinate(member));
        }
      }
      declaredComponents.add(component.members());
    }

    if (owners.size() != entries.size()) {
      HbcSchemaLinks.SchemaCoordinate missing =
          entries.keySet().stream()
              .filter(value -> !owners.containsKey(value))
              .findFirst()
              .orElseThrow();
      throw malformed(
          "schema catalog entry has no component evidence: " + displayCoordinate(missing));
    }

    declaredComponents.sort(
        (left, right) ->
            HbcSchemaCatalogGraph.compareCoordinateLists(
                left, right, COORDINATE_ORDER));
    if (!declaredComponents.equals(computedComponents)) {
      throw malformed("schema catalog component evidence does not match dependency graph");
    }

    for (CatalogComponent component : components.values()) {
      List<String> expected = expectedComponentDependencies(component, entries, owners);
      if (!component.dependencies().equals(expected)) {
        throw malformed(
            "schema catalog component dependencies mismatch for " + component.id());
      }
    }

    Map<String, Set<String>> componentGraph = new TreeMap<>();
    for (CatalogComponent component : components.values()) {
      componentGraph.put(component.id(), new TreeSet<>(component.dependencies()));
    }
    return new AdmittedCatalog(
        entries,
        components,
        HbcSchemaCatalogGraph.dependencyFirstOrder(componentGraph));
  }

  /** Decodes HBC1 only after every exact link and transitive dependency is admitted. */
  public static AdmittedLinkedProgram admitLinkedProgram(
      byte[] artifact, AdmittedCatalog catalog) {
    if (catalog == null) throw malformed("admitted schema catalog is required");
    HbcSchemaLinks.LinkedProgram linked = HbcSchemaLinks.decode(artifact);
    Set<HbcSchemaLinks.SchemaCoordinate> reachable =
        new TreeSet<>(COORDINATE_ORDER);
    ArrayList<HbcSchemaLinks.SchemaCoordinate> pending =
        new ArrayList<>(linked.schemaLinks());
    while (!pending.isEmpty()) {
      HbcSchemaLinks.SchemaCoordinate coordinate = pending.remove(pending.size() - 1);
      CatalogEntry entry = catalog.entries().get(coordinate);
      if (entry == null) {
        throw malformed(
            "linked bytecode schema coordinate is not admitted: "
                + displayCoordinate(coordinate));
      }
      if (reachable.add(coordinate)) pending.addAll(entry.dependencies());
    }

    List<HbcSchemaLinks.SchemaCoordinate> resolved = new ArrayList<>();
    for (String componentId : catalog.componentOrder()) {
      CatalogComponent component = catalog.components().get(componentId);
      if (component == null) throw new AssertionError("admitted component order is invalid");
      for (HbcSchemaLinks.SchemaCoordinate member : component.members()) {
        if (reachable.contains(member)) resolved.add(member);
      }
    }
    return new AdmittedLinkedProgram(linked, resolved);
  }

  private static HbcSchemaLinks.SchemaCoordinate requireCoordinate(
      HbcSchemaLinks.SchemaCoordinate coordinate) {
    if (coordinate == null) throw malformed("schema catalog coordinate is required");
    return new HbcSchemaLinks.SchemaCoordinate(
        coordinate.id(), coordinate.hash());
  }

  private static List<HbcSchemaLinks.SchemaCoordinate> canonicalCoordinates(
      List<HbcSchemaLinks.SchemaCoordinate> values, String label) {
    if (values == null) throw malformed(label + " are required");
    ArrayList<HbcSchemaLinks.SchemaCoordinate> output = new ArrayList<>();
    Map<Identity, String> identities =
        new TreeMap<>(Comparator.comparing(Identity::id));
    for (HbcSchemaLinks.SchemaCoordinate raw : values) {
      HbcSchemaLinks.SchemaCoordinate coordinate = requireCoordinate(raw);
      Identity identity = new Identity(coordinate.id());
      String previousHash = identities.put(identity, coordinate.hash());
      if (previousHash != null) {
        if (previousHash.equals(coordinate.hash())) {
          throw malformed(label + " contain a duplicate coordinate");
        }
        throw malformed(label + " contain conflicting immutable identities");
      }
      output.add(coordinate);
    }
    output.sort(COORDINATE_ORDER);
    return List.copyOf(output);
  }

  private static List<String> canonicalComponentDependencies(List<String> values) {
    if (values == null) {
      throw malformed("schema catalog component dependencies are required");
    }
    ArrayList<String> output = new ArrayList<>(values);
    output.sort(String::compareTo);
    for (String value : output) {
      validateHash(value, "schema catalog component dependency");
    }
    for (int index = 1; index < output.size(); index++) {
      if (output.get(index - 1).equals(output.get(index))) {
        throw malformed("schema catalog component dependencies contain a duplicate");
      }
    }
    return List.copyOf(output);
  }

  private static void validateHash(String value, String label) {
    if (value == null || !value.startsWith(HASH_PREFIX)) {
      throw malformed(label + " must use sha256");
    }
    String digest = value.substring(HASH_PREFIX.length());
    if (digest.length() != DIGEST_HEX_LENGTH
        || digest.chars().anyMatch(character -> !lowerHex((char) character))) {
      throw malformed(label + " must be canonical lowercase hex");
    }
  }

  private static boolean lowerHex(char value) {
    return (value >= '0' && value <= '9') || (value >= 'a' && value <= 'f');
  }

  private static Map<HbcSchemaLinks.SchemaCoordinate, Set<HbcSchemaLinks.SchemaCoordinate>>
      entryGraph(Map<HbcSchemaLinks.SchemaCoordinate, CatalogEntry> entries) {
    Map<HbcSchemaLinks.SchemaCoordinate, Set<HbcSchemaLinks.SchemaCoordinate>> graph =
        new TreeMap<>(COORDINATE_ORDER);
    for (CatalogEntry entry : entries.values()) {
      Set<HbcSchemaLinks.SchemaCoordinate> dependencies =
          new TreeSet<>(COORDINATE_ORDER);
      dependencies.addAll(entry.dependencies());
      graph.put(entry.coordinate(), dependencies);
    }
    return graph;
  }

  private static List<String> expectedComponentDependencies(
      CatalogComponent component,
      Map<HbcSchemaLinks.SchemaCoordinate, CatalogEntry> entries,
      Map<HbcSchemaLinks.SchemaCoordinate, String> owners) {
    Set<String> output = new TreeSet<>();
    for (HbcSchemaLinks.SchemaCoordinate member : component.members()) {
      for (HbcSchemaLinks.SchemaCoordinate dependency : entries.get(member).dependencies()) {
        String owner = owners.get(dependency);
        if (!component.id().equals(owner)) output.add(owner);
      }
    }
    return List.copyOf(output);
  }

  private static String displayCoordinate(HbcSchemaLinks.SchemaCoordinate coordinate) {
    return "[:schema :"
        + coordinate.id()
        + " \""
        + coordinate.hash()
        + "\"]";
  }

  private static byte[] sha256(byte[] bytes) {
    try {
      return MessageDigest.getInstance("SHA-256").digest(bytes);
    } catch (NoSuchAlgorithmException impossible) {
      throw new AssertionError(impossible);
    }
  }

  private static String displayHash(byte[] digest) {
    StringBuilder output = new StringBuilder(HASH_PREFIX);
    for (byte value : digest) {
      output.append(String.format("%02x", Byte.toUnsignedInt(value)));
    }
    return output.toString();
  }

  private static HbcFormatException malformed(String message) {
    return new HbcFormatException(message);
  }
}
