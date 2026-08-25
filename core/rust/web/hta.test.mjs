import test from "node:test";
import assert from "node:assert/strict";

import { decodeHta, encodeHta, HtaArray, HtaAtom, HtaCharacter, HtaKeyword, HtaNamespace, HtaObject, HtaPointer, HtaRegex, HtaSymbol, HtaVar } from "./packages/hta/index.js";

test("HTA transports Vars without confusing them with their function values", () => {
  const decoded = decodeHta(encodeHta(new HtaVar(new HtaSymbol("demo/rank"))));
  assert.equal(decoded.constructor.name, "HtaVar");
  assert.equal(decoded.symbol.name, "demo/rank");
  assert.equal(String(decoded), "#'demo/rank");
  assert.throws(() => encodeHta(new HtaVar(new HtaSymbol("rank"))), /qualified symbol/);
});

test("HTA transports the canonical scalar and descriptor tags", () => {
  const values = [
    new HtaCharacter("雪"),
    new HtaRegex("^[a-z]+$"),
    new HtaNamespace("demo.core"),
    new HtaPointer(new HtaKeyword("kernel"), new Map([[new HtaKeyword("id"), "ROOT"]]))
  ];
  for (const value of values) {
    const decoded = decodeHta(encodeHta(value));
    assert.equal(decoded.constructor, value.constructor);
    assert.deepEqual(decoded, value);
  }
});

test("HTA rejects the legacy unqualified Var wire tag", () => {
  assert.throws(() => decodeHta(Uint8Array.from([72, 84, 65, 48, 14, 7, 0, 0, 0, 4, 114, 97, 110, 107, 0])), /legacy var tag/);
});

test("HTA transports Atom snapshots", () => {
  const atom = new HtaAtom(new Map([[new HtaKeyword("x"), 10]]));
  const decoded = decodeHta(encodeHta(atom));
  assert.equal(decoded.constructor.name, "HtaAtom");
  assert.equal(String(decoded), "#atom <{:x 10}>");
});

test("HTA transports mutable collection snapshots", () => {
  const array = decodeHta(encodeHta(new HtaArray([1, 2, 3])));
  const object = decodeHta(encodeHta(new HtaObject([["score", 10]])));
  assert.equal(String(array), "(array 1 2 3)");
  assert.equal(String(object), '(object "score" 10)');
});
