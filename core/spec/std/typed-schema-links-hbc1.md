# HBC1 exact schema-link artifact epoch

Status: first implementation tranche for hara-lang/hara#902.

## Purpose

HBC1 binds one canonical HBC0 program to an immutable vector of identified
`std.typed` schema coordinates. It does not place identity inside structural
`SchemaType` / `HalcSchema.Type` nodes and it never resolves a registry alias
or tooling-oriented latest entry.

The first epoch is **external-link-only**. An HBC1 consumer must admit and
verify a catalog containing every exact coordinate before installing or
executing the nested program. Identified-entry embedding is not inferred from
the link format; a later embedded mode requires a separate explicit versioned
contract.

## Coordinate

A coordinate is the binary projection of:

```hara
[:schema :qualified/id "sha256:..."]
```

The binary record stores:

1. qualified keyword name without the leading colon, as canonical UTF-8;
2. the 32 raw bytes represented by the lowercase `sha256:` hash.

The id must contain exactly one namespace separator and non-empty namespace
and name components. Hash input must be exactly 64 lowercase hexadecimal
characters after `sha256:`.

Coordinates are ordered by id, then hash. Duplicate exact coordinates are
invalid. Reusing one id with a different hash is an immutable identity
conflict.

## Envelope

All integers use unsigned big-endian encoding. Lengths are u32.

```text
magic                       4 bytes = HBC1
payload-length              u32
payload
  nested-hbc0-length        u32
  nested-hbc0               exact canonical HBC0 artifact bytes
  schema-link-count         u32
  schema-links              coordinate[schema-link-count]
payload-sha256              32 bytes
```

The outer digest authenticates the nested HBC0 bytes and every coordinate.
The nested program retains its own HBC0 digest. Decode authenticates the outer
envelope before decoding the nested program.

## Canonicality

- HBC0 encoding and decoding remain byte-compatible and unchanged.
- HBC1 encoding sorts coordinates before writing.
- HBC1 decoding rejects a non-canonical coordinate order.
- Invalid UTF-8, malformed IDs or hashes, duplicate/conflicting identities,
  corruption, length mismatch, truncation, and trailing bytes fail before
  catalog resolution or execution.
- Re-encoding a decoded HBC1 value produces identical bytes.

## Admission and execution boundary

The codec returns `{program, schema-links}` as separate values. Ordinary HBC0
execution APIs must not ignore HBC1 links. A later #903 admission layer must:

1. resolve every exact coordinate in an admitted `std.typed.catalog`;
2. verify the catalog hash epoch, direct dependencies, transitive closure, and
   recursive component evidence from #901;
3. reject missing, stale, conflicting, or unsupported entries atomically;
4. install or execute only after complete verification.

There is no fallback from an exact coordinate to an unpinned latest version.

## Compatibility

Anonymous structural schemas and all existing HBC0 schema tags, including the
set and property-aware forms delivered by #836, remain unchanged. HBC1 adds an
outer exact-link envelope and does not define a second structural schema AST.
