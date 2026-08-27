export interface StartOptions {
  /** Override the default adjacent hara_wasm_bg.wasm URL. */
  wasmUrl?: RequestInfo | URL | ArrayBuffer | WebAssembly.Module | Uint8Array;
  /** Host resources registered before the first require. */
  resources?: Map<string, string> | Record<string, string>;
  /** Exact lock used to install semantic packages before the runtime is returned. */
  lock?: string;
  /** Semantic package names, coordinates, or namespaces selected from the lock. */
  targets?: string[];
  /** Fetch, capability, host, and browser-worker policy for package installation. */
  packageOptions?: LockedPackageOptions;
}

export interface HaraRuntime {
  eval(source: string): string;
  require(namespace: string): string;
  registerResource(namespace: string, source: string): void;
  installDirectWasmImport(logical: string, bytes: Uint8Array): void;
  installMemoryWasmBinding(
    manifest: string,
    interfaceSource: string,
    bindingsSource: string,
    bytes: Uint8Array
  ): void;
  unregisterResource(namespace: string): void;
  evalInNamespace(namespace: string, source: string): string;
  currentNamespace(): string;
  compileBytecode(source: string): Uint8Array;
  evalBytecode(artifact: Uint8Array): string;
  evalBytecodeBundle(artifact: Uint8Array): void;
  installPackages(lockSource: string, options?: LockedPackageOptions): Promise<string[]>;
  compileWholeWasm(source: string): Promise<WholeWasmModule>;
  compileWholeWasmProduct(source: string): WholeWasmProduct;
  loadWholeWasm(
    product: WholeWasmProduct | Uint8Array | ArrayBuffer
  ): Promise<WholeWasmModule>;
  installHostHandler(handler: Function): void;
  dispose(): Promise<void>;
  readonly raw: unknown;
}

export interface WholeWasmModule {
  call(...arguments: Array<number | bigint>): bigint;
  callFunction(functionId: number, ...arguments: Array<number | bigint>): bigint;
  readonly manifest: Readonly<Record<string, unknown>> | null;
  readonly module: WebAssembly.Module;
  readonly instance: WebAssembly.Instance;
}

export interface WholeWasmProduct {
  readonly artifact: Uint8Array;
  readonly manifest: Readonly<Record<string, unknown>>;
}

export interface LockedPackageOptions {
  fetch?: typeof globalThis.fetch;
  origin?: string;
  targets?: string[];
  capabilities?: string[];
  hostCalls?: Record<string, Function | Record<string, Function>>;
  workerFactory?: (url: string, options: WorkerOptions) => Worker;
  createObjectURL?: (blob: Blob) => string;
  revokeObjectURL?: (url: string) => void;
  Blob?: typeof Blob;
}

export function loadLockedPackageResources(
  lockSource: string,
  request?: typeof globalThis.fetch
): Promise<Record<string, string>>;

export function installLockedPackages(
  runtime: Pick<HaraRuntime, "registerResource"> & Partial<Pick<HaraRuntime, "evalBytecodeBundle">>,
  lockSource: string,
  options?: LockedPackageOptions
): Promise<string[]>;

export function installPackageProvider(
  runtime: HaraRuntime,
  lockSource: string,
  options?: LockedPackageOptions
): { readonly active: ReadonlySet<string>; readonly handler: Function };

export function disposeBrowserPackageProviders(runtime: HaraRuntime): Promise<void>;

export function start(options?: StartOptions): Promise<HaraRuntime>;
export const ready: Promise<HaraRuntime>;
export default start;
