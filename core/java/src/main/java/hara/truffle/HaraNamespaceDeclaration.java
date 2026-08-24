package hara.truffle;

import hara.lang.data.Keyword;
import hara.lang.data.List;
import hara.lang.data.Symbol;
import hara.lang.protocol.IMapType;
import hara.lang.protocol.ILinearType;
import java.util.ArrayList;
import java.util.Iterator;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.Map;
import java.util.Set;

/** Fully validated, immutable interpretation of an ns declaration. */
final class HaraNamespaceDeclaration {
  private static final Set<String> FOUNDATION_LIBRARIES =
      Set.of("string", "bytes", "promise", "coroutine", "pretty");

  final Symbol name;
  final boolean blank;
  final Set<String> excludedFoundation;
  final boolean selectiveFoundation;
  final Set<String> exposedFoundation;
  final Set<String> excludedFoundationLibraries;
  final Map<String, String> foundationAliases;
  final String role;
  final String globalAlias;
  final Set<String> globalImports;
  final Object[] structuralClauses;

  private HaraNamespaceDeclaration(
      Symbol name,
      boolean blank,
      Set<String> excludedFoundation,
      boolean selectiveFoundation,
      Set<String> exposedFoundation,
      Set<String> excludedFoundationLibraries,
      Map<String, String> foundationAliases,
      String role,
      String globalAlias,
      Set<String> globalImports,
      Object[] structuralClauses) {
    this.name = name;
    this.blank = blank;
    this.excludedFoundation = Set.copyOf(excludedFoundation);
    this.selectiveFoundation = selectiveFoundation;
    this.exposedFoundation = Set.copyOf(exposedFoundation);
    this.excludedFoundationLibraries = Set.copyOf(excludedFoundationLibraries);
    this.foundationAliases = Map.copyOf(foundationAliases);
    this.role = role;
    this.globalAlias = globalAlias;
    this.globalImports = Set.copyOf(globalImports);
    this.structuralClauses = structuralClauses.clone();
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  static HaraNamespaceDeclaration parse(Symbol name, Object[] clauses) {
    if (name.getNamespace() != null) {
      throw new HaraException("ns name must be an unqualified symbol");
    }
    boolean configSeen = false;
    boolean blank = false;
    boolean overrideSeen = false;
    boolean onlySeen = false;
    String role = "standard";
    LinkedHashSet<String> excludedFoundation = new LinkedHashSet<>();
    LinkedHashSet<String> exposedFoundation = new LinkedHashSet<>();
    LinkedHashSet<String> excluded = new LinkedHashSet<>();
    LinkedHashMap<String, String> aliases = new LinkedHashMap<>();
    ArrayList<Object> structural = new ArrayList<>();
    String globalAlias = null;
    LinkedHashSet<String> globalImports = new LinkedHashSet<>();

    for (Object clauseValue : clauses) {
      if (!(clauseValue instanceof List<?> clause) || clause.count() == 0) {
        throw new HaraException("ns clauses must be non-empty lists");
      }
      if (!(clause.nth(0) instanceof Keyword keyword) || keyword.getNamespace() != null) {
        throw new HaraException("ns clause must start with an unqualified keyword");
      }
      String clauseName = keyword.getName();
      if ("config".equals(clauseName)) {
        if (configSeen) throw new HaraException("ns accepts only one :config clause");
        configSeen = true;
        if (clause.count() != 2 || !(clause.nth(1) instanceof IMapType<?, ?>)) {
          throw new HaraException(":config expects one map");
        }
        IMapType options = (IMapType) clause.nth(1);
        Iterator<?> iterator = options.iterator();
        while (iterator.hasNext()) {
          java.util.Map.Entry<?, ?> entry = (java.util.Map.Entry<?, ?>) iterator.next();
          if (!(entry.getKey() instanceof Keyword option) || option.getNamespace() != null) {
            throw new HaraException(":config keys must be unqualified keywords");
          }
          if (!Set.of(
                  "blank",
                  "rename",
                  "override",
                  "only",
                  "role",
                  "set-global-alias",
                  "set-global")
              .contains(option.getName())) {
            throw new HaraException("Unsupported :config option: :" + option.getName());
          }
        }
        Object blankValue = options.lookup(Keyword.create("blank"));
        if (blankValue != null) {
          if (!(blankValue instanceof Boolean)) {
            throw new HaraException(":config :blank expects a boolean");
          }
          blank = (Boolean) blankValue;
        }
        Object overrideValue = options.lookup(Keyword.create("override"));
        if (overrideValue != null) {
          overrideSeen = true;
          parseFoundationNames(overrideValue, "override", excludedFoundation);
        }
        Object onlyValue = options.lookup(Keyword.create("only"));
        if (onlyValue != null) {
          onlySeen = true;
          parseFoundationNames(onlyValue, "only", exposedFoundation);
        }
        Object renameValue = options.lookup(Keyword.create("rename"));
        if (renameValue != null) parseRename(renameValue, excluded, aliases);
        Object roleValue = options.lookup(Keyword.create("role"));
        if (roleValue != null) {
          if (!(roleValue instanceof Keyword roleKeyword)
              || roleKeyword.getNamespace() != null
              || !Set.of("default", "internal", "facade").contains(roleKeyword.getName())) {
            throw new HaraException(
                ":config :role expects :default, :internal, or :facade");
          }
          role = "default".equals(roleKeyword.getName()) ? "standard" : roleKeyword.getName();
        }
        Object globalAliasValue = options.lookup(Keyword.create("set-global-alias"));
        if (globalAliasValue != null) {
          if (!(globalAliasValue instanceof Symbol alias)
              || alias.getNamespace() != null) {
            throw new HaraException(
                ":config :set-global-alias expects an unqualified symbol");
          }
          if ("-".equals(alias.getName())) {
            throw new HaraException(":config :set-global-alias is reserved: -");
          }
          globalAlias = alias.getName();
        }
        Object globalImportsValue = options.lookup(Keyword.create("set-global"));
        if (globalImportsValue != null) {
          parseGlobalImports(globalImportsValue, globalImports);
        }
      } else if ("require".equals(clauseName)
          || "use".equals(clauseName)
          || "flavor".equals(clauseName)
          || "import".equals(clauseName)) {
        structural.add(clause);
      } else {
        throw new HaraException("Unsupported ns clause: :" + clauseName);
      }
    }
    for (String library : aliases.keySet()) {
      if (excluded.contains(library)) {
        throw new HaraException(
            "Foundation library cannot be both excluded and aliased: " + library);
      }
    }
    if (blank && overrideSeen) {
      throw new HaraException(":config :blank true cannot be combined with :override");
    }
    if (blank && onlySeen) {
      throw new HaraException(":config :blank true cannot be combined with :only");
    }
    if (overrideSeen && onlySeen) {
      throw new HaraException(":config :override cannot be combined with :only");
    }
    return new HaraNamespaceDeclaration(
        name,
        blank,
        excludedFoundation,
        onlySeen,
        exposedFoundation,
        excluded,
        aliases,
        role,
        globalAlias,
        globalImports,
        structural.toArray());
  }

  private static void parseFoundationNames(Object value, String option, Set<String> output) {
    if (!(value instanceof ILinearType<?> symbols) || !"[".equals(symbols.startString())) {
      throw new HaraException(
          ":config :" + option + " expects a vector of unqualified symbols");
    }
    for (Object item : symbols) {
      if (!(item instanceof Symbol symbol) || symbol.getNamespace() != null) {
        throw new HaraException(
            ":config :" + option + " expects a vector of unqualified symbols");
      }
      if (!output.add(symbol.getName())) {
        String label = "override".equals(option) ? "override" : "selection";
        throw new HaraException("Duplicate Foundation " + label + ": " + symbol.getName());
      }
    }
  }

  private static void parseGlobalImports(Object value, Set<String> output) {
    if (!(value instanceof ILinearType<?> symbols) || !"[".equals(symbols.startString())) {
      throw new HaraException(
          ":config :set-global expects a vector of qualified Vars");
    }
    for (Object item : symbols) {
      if (!(item instanceof Symbol symbol) || symbol.getNamespace() == null) {
        throw new HaraException(":config :set-global expects qualified Vars");
      }
      if (!output.add(symbol.display())) {
        throw new HaraException("Duplicate global import: " + symbol.display());
      }
    }
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  private static void parseRename(
      Object value, Set<String> excluded, Map<String, String> aliases) {
    if (Keyword.create("all").equals(value)) return;
    if (!(value instanceof IMapType<?, ?> options)) {
      throw new HaraException(":config :rename expects :all or an options map");
    }
    Iterator<?> iterator = options.iterator();
    while (iterator.hasNext()) {
      java.util.Map.Entry<?, ?> entry = (java.util.Map.Entry<?, ?>) iterator.next();
      if (!(entry.getKey() instanceof Keyword option) || option.getNamespace() != null) {
        throw new HaraException(":config :rename keys must be unqualified keywords");
      }
      if (!"exclude".equals(option.getName()) && !"alias".equals(option.getName())) {
        throw new HaraException(
            "Unsupported :config :rename option: :" + option.getName());
      }
    }
    Object excludeValue = ((IMapType) options).lookup(Keyword.create("exclude"));
    if (excludeValue != null) {
      if (!(excludeValue instanceof ILinearType<?> vector)
          || !"[".equals(vector.startString())) {
        throw new HaraException(":config :rename :exclude expects a vector");
      }
      for (Object item : vector) {
        String library = libraryName(item, ":config :rename :exclude");
        if (!excluded.add(library)) {
          throw new HaraException("Duplicate Foundation library exclusion: " + library);
        }
      }
    }
    Object aliasValue = ((IMapType) options).lookup(Keyword.create("alias"));
    if (aliasValue != null) {
      if (!(aliasValue instanceof IMapType<?, ?> aliasMap)) {
        throw new HaraException(":config :rename :alias expects a map");
      }
      LinkedHashSet<String> usedAliases = new LinkedHashSet<>();
      for (Object entryValue : aliasMap) {
        java.util.Map.Entry<?, ?> entry = (java.util.Map.Entry<?, ?>) entryValue;
        String library = libraryName(entry.getKey(), ":config :rename :alias");
        if (!(entry.getValue() instanceof Symbol alias) || alias.getNamespace() != null) {
          throw new HaraException("Foundation library aliases must be unqualified symbols");
        }
        if (!usedAliases.add(alias.getName())) {
          throw new HaraException("Duplicate Foundation library alias target: " + alias.getName());
        }
        if (aliases.put(library, alias.getName()) != null) {
          throw new HaraException("Duplicate Foundation library alias: " + library);
        }
      }
    }
  }

  private static String libraryName(Object value, String operation) {
    if (!(value instanceof Symbol symbol) || symbol.getNamespace() != null) {
      throw new HaraException(operation + " expects unqualified library symbols");
    }
    if (!FOUNDATION_LIBRARIES.contains(symbol.getName())) {
      throw new HaraException("Unknown Foundation library: " + symbol.getName());
    }
    return symbol.getName();
  }
}
