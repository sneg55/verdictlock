// keccak-256, the hash `registerWasm` wants. Dependency-free so the number in
// the README can be reproduced without installing anything.
const RC = [
  0x00000001n, 0x00008082n, 0x800000000000808an, 0x8000000080008000n,
  0x000000000000808bn, 0x0000000080000001n, 0x8000000080008081n, 0x8000000000008009n,
  0x000000000000008an, 0x0000000000000088n, 0x0000000080008009n, 0x000000008000000an,
  0x000000008000808bn, 0x800000000000008bn, 0x8000000000008089n, 0x8000000000008003n,
  0x8000000000008002n, 0x8000000000000080n, 0x000000000000800an, 0x800000008000000an,
  0x8000000080008081n, 0x8000000000008080n, 0x0000000080000001n, 0x8000000080008008n,
];
const ROT = [
  [0, 36, 3, 41, 18], [1, 44, 10, 45, 2], [62, 6, 43, 15, 61],
  [28, 55, 25, 21, 56], [27, 20, 39, 8, 14],
];
const MASK = (1n << 64n) - 1n;
const rotl = (x, n) => n === 0 ? x : ((x << BigInt(n)) | (x >> BigInt(64 - n))) & MASK;

function permute(a) {
  for (let round = 0; round < 24; round += 1) {
    const c = [0n, 0n, 0n, 0n, 0n];
    for (let x = 0; x < 5; x += 1) c[x] = a[x][0] ^ a[x][1] ^ a[x][2] ^ a[x][3] ^ a[x][4];
    for (let x = 0; x < 5; x += 1) {
      const d = c[(x + 4) % 5] ^ rotl(c[(x + 1) % 5], 1);
      for (let y = 0; y < 5; y += 1) a[x][y] ^= d;
    }
    const b = [[], [], [], [], []];
    for (let x = 0; x < 5; x += 1) {
      for (let y = 0; y < 5; y += 1) b[y][(2 * x + 3 * y) % 5] = rotl(a[x][y], ROT[x][y]);
    }
    for (let x = 0; x < 5; x += 1) {
      for (let y = 0; y < 5; y += 1) a[x][y] = b[x][y] ^ (~b[(x + 1) % 5][y] & b[(x + 2) % 5][y]) & MASK;
    }
    a[0][0] ^= RC[round];
  }
}

export function keccak256(bytes) {
  const rate = 136;
  const state = [[0n, 0n, 0n, 0n, 0n], [0n, 0n, 0n, 0n, 0n], [0n, 0n, 0n, 0n, 0n],
    [0n, 0n, 0n, 0n, 0n], [0n, 0n, 0n, 0n, 0n]];
  const padded = new Uint8Array(Math.ceil((bytes.length + 1) / rate) * rate);
  padded.set(bytes);
  padded[bytes.length] = 0x01;
  padded[padded.length - 1] |= 0x80;
  for (let offset = 0; offset < padded.length; offset += rate) {
    for (let i = 0; i < rate / 8; i += 1) {
      let lane = 0n;
      for (let k = 7; k >= 0; k -= 1) lane = (lane << 8n) | BigInt(padded[offset + i * 8 + k]);
      state[i % 5][Math.floor(i / 5)] ^= lane;
    }
    permute(state);
  }
  let out = "";
  for (let i = 0; i < 4; i += 1) {
    let lane = state[i % 5][Math.floor(i / 5)];
    for (let k = 0; k < 8; k += 1) {
      out += (lane & 0xffn).toString(16).padStart(2, "0");
      lane >>= 8n;
    }
  }
  return `0x${out}`;
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const { readFile } = await import("node:fs/promises");
  const empty = keccak256(new Uint8Array(0));
  const expected = "0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470";
  if (empty !== expected) throw new Error(`keccak self-test failed: ${empty}`);
  for (const path of process.argv.slice(2)) {
    const bytes = new Uint8Array(await readFile(path));
    console.log(`${keccak256(bytes)}  ${bytes.length} bytes  ${path}`);
  }
}
