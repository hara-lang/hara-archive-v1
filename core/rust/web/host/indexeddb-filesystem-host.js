import {
  FilesystemProviderError,
  createIndexedDbFilesystemFactory
} from "./indexeddb-filesystem-provider.js";

/**
 * Kernel-context and mount-id adapter for the browser IndexedDB provider.
 *
 * Provider construction remains an embedding concern. The adapter stores only
 * opened capabilities and never exposes the IDBDatabase or factory to Hara.
 */
export function createIndexedDbFilesystemHost({ factory, ...factoryOptions } = {}) {
  const providerFactory = factory ?? createIndexedDbFilesystemFactory(factoryOptions);
  if (providerFactory?.kind !== "indexeddb" || typeof providerFactory.open !== "function") {
    throw new TypeError("an IndexedDB filesystem factory is required");
  }
  const contexts = new WeakMap();

  function mountsFor(context, create = false) {
    if (!context || (typeof context !== "object" && typeof context !== "function")) {
      throw new TypeError("filesystem kernel context must be an object");
    }
    let mounts = contexts.get(context);
    if (!mounts && create) {
      mounts = new Map();
      contexts.set(context, mounts);
    }
    return mounts;
  }

  function validateMountId(mountId) {
    if (!Number.isSafeInteger(mountId) || mountId <= 0) {
      throw new TypeError("filesystem mount id must be a positive safe integer");
    }
    return mountId;
  }

  function requireMount(context, mountId) {
    validateMountId(mountId);
    const mount = mountsFor(context)?.get(mountId);
    if (!mount) {
      throw new FilesystemProviderError(
        "provider-closed",
        "filesystem mount is not attached",
        { providerCode: "mount-not-attached" }
      );
    }
    return mount;
  }

  return Object.freeze({
    kind: "indexeddb",

    async register(context, mountId, descriptor = {}) {
      validateMountId(mountId);
      const mounts = mountsFor(context, true);
      if (mounts.has(mountId)) {
        throw new TypeError(`filesystem mount already exists: ${mountId}`);
      }
      const provider = descriptor.provider ?? descriptor.kind ?? "indexeddb";
      if (provider !== "indexeddb") {
        throw new TypeError(`filesystem provider is not indexeddb: ${provider}`);
      }
      const configuration = {
        database: descriptor.database,
        namespace: descriptor.namespace ?? descriptor.key,
        version: descriptor.version,
        quotaBytes: descriptor.quotaBytes,
        maxFileBytes: descriptor.maxFileBytes
      };
      for (const key of Object.keys(configuration)) {
        if (configuration[key] === undefined) delete configuration[key];
      }
      const filesystem = await providerFactory.open(configuration);
      mounts.set(mountId, filesystem);
      return filesystem.descriptor();
    },

    descriptor(context, mountId) {
      return requireMount(context, mountId).descriptor();
    },

    capabilities(context, mountId) {
      return requireMount(context, mountId).capabilities();
    },

    async invoke(context, mountId, method, args = [], callContext = null) {
      const filesystem = requireMount(context, mountId);
      switch (method) {
        case "read":
          return filesystem.read(callContext, args[0]);
        case "write":
          return filesystem.write(callContext, args[0], args[1], args[2], args[3]);
        case "exists":
        case "exists?":
          try {
            await filesystem.stat(callContext, args[0]);
            return true;
          } catch (error) {
            if (error?.code === "file/not-found") return false;
            throw error;
          }
        case "stat":
          return filesystem.stat(callContext, args[0]);
        case "entries-page":
          return filesystem.entriesPage(callContext, args[0], args[1]);
        case "entries":
          return collectEntries(filesystem, callContext, args[0], args[1]);
        case "list":
          return (await collectEntries(filesystem, callContext, args[0], args[1]))
            .map((entry) => entry.path);
        case "walk":
          return (await collectWalk(filesystem, callContext, args[0]))
            .map((entry) => entry.path);
        case "mkdir":
          return filesystem.mkdir(callContext, args[0], args[1], args[2]);
        case "delete":
          return filesystem.delete(callContext, args[0], args[1], args[2]);
        case "copy":
          return filesystem.copy(callContext, args[0], args[1], args[2], args[3]);
        case "move":
          return filesystem.move(callContext, args[0], args[1], args[2], args[3]);
        default:
          throw new FilesystemProviderError(
            "unsupported",
            `filesystem method is unsupported: ${method}`,
            { operation: String(method), providerCode: "method-unsupported" }
          );
      }
    },

    async close(context, mountId) {
      validateMountId(mountId);
      const mounts = mountsFor(context);
      const filesystem = mounts?.get(mountId);
      if (!filesystem) {
        throw new FilesystemProviderError(
          "provider-closed",
          "filesystem mount is not attached",
          { operation: "close", providerCode: "mount-not-attached" }
        );
      }
      mounts.delete(mountId);
      await filesystem.close();
      if (mounts.size === 0) contexts.delete(context);
      return true;
    },

    async closeContext(context) {
      const mounts = mountsFor(context);
      if (!mounts) return 0;
      contexts.delete(context);
      const filesystems = [...mounts.values()];
      mounts.clear();
      await Promise.allSettled(filesystems.map((filesystem) => filesystem.close()));
      return filesystems.length;
    }
  });
}

async function collectEntries(filesystem, context, path, options = {}) {
  const pageLimit = options?.pageLimit ?? 256;
  let token = null;
  const entries = [];
  do {
    const page = await filesystem.entriesPage(context, path, { token, limit: pageLimit });
    entries.push(...page.entries);
    token = page.nextToken;
  } while (token != null);
  entries.sort(compareEntries);
  return entries;
}

async function collectWalk(filesystem, context, path, ancestors = new Set()) {
  const entry = await filesystem.stat(context, path);
  if (entry.type !== "directory") return [entry];
  const id = entry.extensions?.["file/id"] ?? entry.path;
  if (ancestors.has(id)) {
    throw new FilesystemProviderError(
      "io",
      "filesystem traversal cycle detected",
      { operation: "walk", path: entry.path, providerCode: "cycle-detected" }
    );
  }
  const nextAncestors = new Set(ancestors);
  nextAncestors.add(id);
  const output = [];
  for (const child of await collectEntries(filesystem, context, entry.path)) {
    if (child.type === "directory") {
      output.push(...await collectWalk(filesystem, context, child.path, nextAncestors));
    } else {
      output.push(child);
    }
  }
  output.sort(compareEntries);
  return output;
}

function compareEntries(left, right) {
  return left.path < right.path ? -1 : left.path > right.path ? 1 : 0;
}
