#!/usr/bin/env python3
"""Pack GloVe vectors into the blob the module embeds.

The module runs with no network, no filesystem and no allocator, so knowing that
two different words are the same answer has to be compiled in. This writes the
most frequent N words as FNV-1a hashes, the same hash the tokeniser computes, so
lookup is a binary search over a sorted u32 array, followed by one L2-normalised
int8 row per word, which makes a cosine an integer dot product.

Vectors: GloVe (Pennington, Socher and Manning, 2014), released under the Open
Data Commons Public Domain Dedication and License v1.0. Taken from the
sentence-transformers mirror, which stores them as a safetensors tensor whose
rows follow the vocabulary file in descending frequency order:

  base=https://huggingface.co/sentence-transformers/average_word_embeddings_glove.6B.300d/resolve/main/0_WordEmbeddings
  curl -sL "$base/whitespacetokenizer_config.json" -o vocab.json
  curl -sL -r 128-36000127 "$base/model.safetensors" -o rows.f32   # first 30000 rows
  python3 tools/pack_vectors.py vocab.json rows.f32 module/src/vectors.bin 20000
"""
import json
import struct
import sys

DIM = 300


def fnv1a(word: str) -> int:
    h = 0x811C9DC5
    for b in word.encode("utf-8"):
        if 0x41 <= b <= 0x5A:
            b += 32
        h ^= b
        h = (h * 0x01000193) & 0xFFFFFFFF
    return h


def main() -> None:
    if len(sys.argv) < 4:
        raise SystemExit(__doc__)
    vocab_path, rows_path, out_path = sys.argv[1], sys.argv[2], sys.argv[3]
    want = int(sys.argv[4]) if len(sys.argv) > 4 else 20000

    vocab = json.load(open(vocab_path))["vocab"]
    raw = open(rows_path, "rb").read()
    available = len(raw) // (DIM * 4)

    rows = {}
    skipped = 0
    for index in range(min(available, len(vocab))):
        word = vocab[index]
        # the module only tokenises runs of letters and digits, so a vector for
        # "," or "n't" could never be looked up
        if not word.isascii() or not word.isalnum():
            skipped += 1
            continue
        key = fnv1a(word.lower())
        if key in rows:  # keep the more frequent word on a collision
            continue
        values = struct.unpack_from("<%df" % DIM, raw, index * DIM * 4)
        norm = sum(v * v for v in values) ** 0.5
        if norm == 0.0:
            continue
        rows[key] = bytes(
            (max(-127, min(127, round(v / norm * 127))) & 0xFF) for v in values
        )
        if len(rows) == want:
            break

    ordered = sorted(rows)
    with open(out_path, "wb") as fh:
        fh.write(b"VLV1")
        fh.write(struct.pack("<II", len(ordered), DIM))
        for key in ordered:
            fh.write(struct.pack("<I", key))
        for key in ordered:
            fh.write(rows[key])
    print("%d words, %dd, skipped %d, %d bytes"
          % (len(ordered), DIM, skipped, 12 + 4 * len(ordered) + DIM * len(ordered)))


if __name__ == "__main__":
    main()
