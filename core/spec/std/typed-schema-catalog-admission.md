# Exact schema catalog admission for HBC1

Status: second implementation tranche for hara-lang/hara#902.

## Purpose

HBC1 carries exact identified-schema links but does not carry a mutable lookup
policy. A linked artifact becomes usable only after a complete catalog manifest
has been admitted atomically.

Admission verifies the same identity and graph evidence produced by
`std.typed.catalog`:

- exact `[:schema id hash]` coordinates;
- exact direct dependencies;
- deterministic strongly connected components from #901;
- deterministic component dependencies and dependency-first order;
- no unpinned latest-version lookup.

## Entry projection

One admitted entry is portable data equivalent to:

```hara
{:schema/coordinate
 [:schema :model/profile "sha256:..."]
 :schema/dependencies
 [[:schema :model/id "sha256:..."]]}
```

Every dependency must name another exact entry in the same admitted manifest.
Duplicate coordinates are invalid. Reusing one id with a different hash is an
immutable identity conflict.

## Component projection

One admitted component is portable data equivalent to:

```hara
{:component/id "sha256:..."
 :component/members
 [[:schema :model/profile "sha256:..."]]
 :component/dependencies ["sha256:..."]}
```

The component identity is exactly the existing #901 operation:

```hara
(str "sha256:"
     (Crypto/sha256
      (str/encode-utf8
       (pr-str
  [:std.typed.catalog/component-v2 members]))))
```

Members and component dependencies are canonically ordered. Native admission
recomputes strongly connected components from exact entry edges and rejects
forged, incomplete, overlapping, or missing component evidence.

## Atomic admission

Admission succeeds only when all of these checks pass:

1. every coordinate and component digest is canonical lowercase SHA-256;
2. every schema id has one immutable hash;
3. every direct dependency exists exactly;
4. every entry belongs to exactly one component;
5. declared components equal the graph's strongly connected components;
6. every component id matches the portable #901 hash algorithm;
7. declared component dependencies equal dependencies derived from entry edges;
8. the condensed component graph is acyclic and has a deterministic
   dependency-first order.

No partial catalog is returned after a failure.

## HBC1 release boundary

An HBC1 decoder authenticates and parses the artifact first. The admission layer
then resolves each linked coordinate against the admitted catalog and computes
its complete dependency closure in component order.

Only the resulting admitted linked-program value may cross the exact catalog
boundary. Missing or stale coordinates fail before the nested program is
released to an embedding caller. There is no fallback to an alias or latest
version.

## Compatibility

- Existing HBC0 artifacts and anonymous structural schemas remain unchanged.
- HBC1 remains external-link-only in this epoch.
- Catalog identity remains on entry envelopes, not structural schema nodes.
- Registry publication and package admission remain under #903.
- A later #902 tranche will bind this admission contract to the shared
  HAL/Rust/Truffle catalog corpus and runtime/package loading surfaces.
