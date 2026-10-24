"""Add reproducible seeds without overwriting or deleting learned corpus files."""

import argparse
import hashlib
from pathlib import Path

TARGETS = (
    "round_trip",
    "editor",
    "editor_reparse",
    "editor_stateful",
    "generated_edits",
    "inputs",
)
# tests/support/editor_inputs.rs PIECES: the hostile strings of the normal and extended tests.
PIECES = (
    *("", " ", "\t", "\n", "\r", "\r\n", "=", ":"),
    *("[", "]", ";", "#", "\\", "\ufeff", "a", "b"),
)
REPO = Path(__file__).resolve().parents[2]


def random_bytes(seed, length):
    state = seed + 1
    result = bytearray()
    while len(result) < length:
        state = (state * 6364136223846793005 + 1) % (1 << 64)
        result.extend(state.to_bytes(8, "little")[4:])
    return result[:length]


def seed_corpus(root):
    for target in TARGETS:
        (root / target).mkdir(parents=True, exist_ok=True)

    def add(target, data):
        path = root / target / ("seed-" + hashlib.sha256(data).hexdigest())
        # Existing learned inputs, including same-content seeds, are retained.
        if not path.exists():
            path.write_bytes(data)

    for fixture in sorted((REPO / "tests" / "fixtures").iterdir()):
        if fixture.is_file():
            add("round_trip", fixture.read_bytes())
    for source in ("", "\ufeff", "[s]\r\nk=one \\\r\n tail ; note\r\n", "bad key\n", "[]\r[missing"):
        add("round_trip", source.encode())
    # tests/fuzz_regressions.rs: bit 3 of the first byte is mixed line endings.
    for flags in range(16):
        for spacing in range(3):
            for operations in (4, 32, 128):
                data = bytearray((flags, spacing, flags % 3))
                data.extend(random_bytes(flags * 7 + spacing, operations * 4))
                add("editor_stateful", data)
    operations = 9  # tests/support/generated_edits.rs OPERATIONS
    for seed in range(operations * 24):
        data = random_bytes(seed, 1536)
        data[0] = seed % operations
        data[3] = seed // operations % 3
        data[4] = seed // (operations * 3) % 4
        add("generated_edits", data)
    # tests/support/editor_inputs.rs check: a selector byte (parser options in bits 0 and 1,
    # spacing in the rest, modulo 3) and a document byte, then the text and, for Exact spacing,
    # the whitespace before and after the separator, separated by 0xFF bytes. Every piece goes
    # into every document. The selector steps with both, so that each document and each piece
    # gets all 12 selectors: every option set with every spacing.
    documents = 14  # tests/support/editor_inputs.rs DOCUMENTS
    for document in range(documents):
        for index, piece in enumerate(PIECES):
            selector = (document + index) % 12
            data = bytearray((selector, document)) + piece.encode()
            if selector >> 2 == 2:
                data += b"\xff\t\xff  "
            add("inputs", data)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus-root", type=Path, default=REPO / "fuzz" / "corpus")
    args = parser.parse_args()
    seed_corpus(args.corpus_root)
    for target in TARGETS:
        count = sum(path.is_file() for path in (args.corpus_root / target).iterdir())
        print(f"{target}: {count} corpus files")
