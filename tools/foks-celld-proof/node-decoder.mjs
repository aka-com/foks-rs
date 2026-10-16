// Pure shared-Rust decoder. The deployment supplies the immutable Wasm module;
// neither replay callers nor HTTP requests may choose code or a length override.
export const MAX_EXACT_NODE_BYTES = 65_536;
const fail = () => { throw new Error("proof node derivation rejected"); };

export function nodeDecoder(module) {
  const forbidden = () => { throw new Error("pure node decoder called an import"); };
  const instance = new WebAssembly.Instance(module, {foks:{sql:forbidden,domain:forbidden}});
  const exports = instance.exports;
  for (const name of ["small_box_input_alloc","small_box_ciphertext_length","output_pointer","output_length"]) {
    if (typeof exports[name] !== "function") fail();
  }
  if (!(exports.memory instanceof WebAssembly.Memory) || exports.memory.buffer.byteLength > 64*1024*1024) fail();
  let busy = false;
  const range = (pointer,length) => {
    const memory = exports.memory.buffer;
    if (!Number.isInteger(pointer) || !Number.isInteger(length) || pointer <= 0 || length <= 0 ||
        memory.byteLength > 64*1024*1024 || pointer > memory.byteLength-length) fail();
    return new Uint8Array(memory,pointer,length);
  };
  return Object.freeze(function derive(exact) {
    if (busy || !(exact instanceof Uint8Array) || exact.byteLength === 0 || exact.byteLength > MAX_EXACT_NODE_BYTES) fail();
    busy = true;
    try {
      const pointer = exports.small_box_input_alloc(exact.byteLength);
      range(pointer,exact.byteLength).set(exact);
      const status = exports.small_box_ciphertext_length();
      const length = exports.output_length();
      if (status !== 0 || !Number.isInteger(length) || length < 1 || length > 19) fail();
      // Decode synchronously from a fresh memory view; no view escapes or spans
      // another Wasm allocation/import. The resulting string owns its bytes.
      const value = new TextDecoder("ascii",{fatal:true}).decode(range(exports.output_pointer(),length));
      if (!/^(?:0|[1-9][0-9]*)$/.test(value) || BigInt(value) > 9223372036854775807n) fail();
      return value;
    } finally {busy = false;}
  });
}
