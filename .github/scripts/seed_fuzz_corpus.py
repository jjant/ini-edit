"""Add reproducible seeds without overwriting or deleting learned corpus files."""

import argparse
import hashlib
from pathlib import Path

TARGETS = ("round_trip", "editor", "editor_reparse", "editor_stateful", "generated_edits")
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
    for flags in range(8):
        for spacing in range(3):
            for operations in (4, 32, 128):
                data = bytearray((flags, spacing, flags % 3))
                data.extend(random_bytes(flags * 7 + spacing, operations * 4))
                add("editor_stateful", data)
    for seed in range(192):
        data = random_bytes(seed, 1536)
        data[0] = seed % 8
        data[3] = seed // 8 % 3
        data[4] = seed // 24 % 4
        add("generated_edits", data)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus-root", type=Path, default=REPO / "fuzz" / "corpus")
    args = parser.parse_args()
    seed_corpus(args.corpus_root)
    for target in TARGETS:
        count = sum(path.is_file() for path in (args.corpus_root / target).iterdir())
        print(f"{target}: {count} corpus files")
