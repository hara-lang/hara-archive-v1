/** Parses the JSON manifests returned by the embedded browser runtime. */
export function parseJson(source) {
  if (typeof source !== "string") throw new TypeError("json/parse expects text");
  try {
    return JSON.parse(source);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    throw new Error(`json/parse: ${message}`);
  }
}
