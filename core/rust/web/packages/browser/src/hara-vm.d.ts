export interface StartOptions {
  wasmUrl?: RequestInfo | URL | ArrayBuffer | WebAssembly.Module | Uint8Array;
  resources?: Map<string, string> | Record<string, string>;
  lock?: string;
  targets?: string[];
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
  evalInNamespace(namespace: string, source: string): string;
  currentNamespace(): string;
  compileBytecode(source: string): Uint8Array;
  evalBytecode(artifact: Uint8Array): string;
  evalBytecodeBundle(artifact: Uint8Array): void;
  installPackages(lockSource: string, options?: LockedPackageOptions): Promise<string[]>;
  dispose(): Promise<void>;
  readonly raw: unknown;
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

export function start(options?: StartOptions): Promise<HaraRuntime>;
export const ready: Promise<HaraRuntime>;
export default start;
