import test from "node:test";
import assert from "node:assert/strict";

import { decodeHta, encodeHta, HtaArray, HtaAtom, HtaKeyword, HtaObject, HtaSymbol, HtaVar } from "./packages/hta/index.js";

test("HTA transports Vars without confusing them with their function values", () => {
  const decoded = decodeHta(encodeHta(new HtaVar(new HtaSymbol("rank"))));
  assert.equal(decoded.constructor.name, "HtaVar");
  assert.equal(decoded.symbol.name, "rank");
  assert.equal(String(decoded), "#'rank");
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
