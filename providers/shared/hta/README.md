# Rich HTA filesystem provider contract

The filesystem provider packages use one portable boundary:

- `provider/provider.wasm` is the only package artifact that implements the
  public Hara façade.
- The Wasm façade exports `describe`, `open`, `descriptor`, `stat`, `read`,
  `write`, `entries-page`, `mkdir`, `delete`, `copy`, `move`, and `close`.
- The façade may call only the provider's declared
  `filesystem.<provider>/describe`, `open`, `request`, `cancel`, and `close`
  host methods.
- Credentials, transport clients, request signing, connection pools, and
  cancellation controllers stay in the trusted host adapter. They are never
  serialized into descriptors or embedded in the archive.
- Host responses are bounded and normalized to the provider-neutral
  `IFilesystem` shape. Provider-specific details belong under `extensions`.

`project.edn` is the package source declaration and `extension.edn` is the
route manifest. Both declare `:provider :wasm`, `:abi :hta.v1`, `:root
"provider"`, and `:module "provider.wasm"`; a consumer installs the archive
without compiling source or discovering host code dynamically.

The JVM providers remain explicit compatibility routes. Their transport
adapters are not used as a fallback for the Wasm route.

`src/hara/hta/provider/common.hal` is embedded by the raw rich-HTA runtime
under `hara.hta.provider.common`. Provider source files only choose the trusted
host service identity and route; protocol definitions, descriptor handling,
normalization, host dispatch, and lifecycle behavior live in that shared
namespace. This keeps each provider interface declaration small while keeping
the Wasm artifacts self-contained. Its path-matched Hara tests live under
`test/hara/hta/provider/common_test.hal`.
