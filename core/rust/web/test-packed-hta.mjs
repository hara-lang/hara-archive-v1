import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import {
  existsSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = path.dirname(fileURLToPath(import.meta.url));
const temporary = mkdtempSync(path.join(tmpdir(), "hara-hta-pack-"));
const cache = path.join(temporary, "npm-cache");
const environment = { ...process.env, npm_config_cache: cache };

try {
  const packed = JSON.parse(
    execFileSync(
      "npm",
      [
        "pack",
        "--json",
        "--workspace",
        "@hara-lang/hta",
        "--pack-destination",
        temporary,
      ],
      { cwd: root, encoding: "utf8", env: environment },
    ),
  )[0];
  const packedPaths = new Set(packed.files.map(({ path: file }) => file));
  for (const required of [
    "LICENSE",
    "index.js",
    "provider-browser.mjs",
    "provider-common.mjs",
    "provider-node.mjs",
    "shared-worker.js",
    "worker.mjs",
  ]) {
    assert.ok(packedPaths.has(required), `packed HTA package is missing ${required}`);
  }

  writeFileSync(
    path.join(temporary, "package.json"),
    `${JSON.stringify({ private: true, type: "module" }, null, 2)}\n`,
  );
  execFileSync(
    "npm",
    [
      "install",
      "--ignore-scripts",
      "--package-lock=false",
      path.join(temporary, packed.filename),
    ],
    { cwd: temporary, stdio: "inherit", env: environment },
  );

  const packageRoot = path.join(
    temporary,
    "node_modules",
    "@hara-lang",
    "hta",
  );
  const hta = await import(pathToFileURL(path.join(packageRoot, "index.js")));
  const browser = await import(
    pathToFileURL(path.join(packageRoot, "provider-browser.mjs"))
  );
  const node = await import(
    pathToFileURL(path.join(packageRoot, "provider-node.mjs"))
  );

  assert.equal(typeof hta.encodeHta, "function");
  assert.equal(typeof hta.HtaContext, "function");
  assert.equal(typeof browser.createBrowserProvider, "function");
  assert.equal(typeof node.serveNodeProvider, "function");

  const manifest = JSON.parse(
    readFileSync(path.join(packageRoot, "package.json"), "utf8"),
  );
  for (const target of Object.values(manifest.exports)) {
    assert.ok(
      existsSync(path.join(packageRoot, target)),
      `export target is missing: ${target}`,
    );
  }
  for (const worker of ["shared-worker.js", "worker.mjs"]) {
    execFileSync(process.execPath, ["--check", path.join(packageRoot, worker)]);
  }

  console.log(
    `verified packed ${packed.name}@${packed.version} (${packed.integrity})`,
  );
} finally {
  rmSync(temporary, { recursive: true, force: true });
}
