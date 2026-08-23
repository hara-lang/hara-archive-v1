export interface StartOptions {
  wasmUrl?: RequestInfo | URL | ArrayBuffer | WebAssembly.Module | Uint8Array;
  resources?: Map<string, string> | Record<string, string>;
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
  dispose(): void;
  readonly raw: unknown;
}

export function start(options?: StartOptions): Promise<HaraRuntime>;
export const ready: Promise<HaraRuntime>;
export default start;
