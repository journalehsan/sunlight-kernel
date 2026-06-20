#!/usr/bin/env python3

import sys
import os
import re
from collections import defaultdict


def normalize_definition(defn: str) -> str:
    # remove surrounding quotes
    defn = defn.strip()

    if defn.startswith('"') and defn.endswith('"'):
        defn = defn[1:-1]

    # unescape doubled quotes
    defn = defn.replace('""', '"')

    # collapse whitespace and newlines
    defn = re.sub(r'\s+', ' ', defn)

    return defn.strip()


def parse_records(path):
    records = []

    with open(path, "r", encoding="utf-8") as f:
        buffer = ""

        for line in f:
            line = line.rstrip("\n")

            if not buffer:
                buffer = line
            else:
                buffer += "\n" + line

            # count quotes to detect end of definition
            if buffer.count('"') % 2 == 0:
                records.append(buffer)
                buffer = ""

        if buffer:
            records.append(buffer)

    return records


def parse_entry(record):
    parts = record.split("\t", 2)

    if len(parts) < 3:
        return None

    word = parts[0].strip().lower()
    wordtype = parts[1].strip()
    definition = parts[2]

    definition = normalize_definition(definition)

    return word, wordtype, definition


def write_chunks(entries, outdir):
    os.makedirs(outdir, exist_ok=True)

    buckets = defaultdict(list)

    for word, wordtype, definition in entries:
        first = word[0].lower()

        if 'a' <= first <= 'z':
            key = first
        else:
            key = "other"

        buckets[key].append((word, wordtype, definition))

    for key, items in buckets.items():
        items.sort(key=lambda x: x[0])

        path = os.path.join(outdir, f"dictionary-{key}.tsv")

        with open(path, "w", encoding="utf-8") as f:
            for word, wordtype, definition in items:
                f.write(f"{word}\t{wordtype}\t{definition}\n")


def main():
    if len(sys.argv) != 3:
        print("usage: convert_dictionary.py input.tsv output_dir")
        sys.exit(1)

    input_file = sys.argv[1]
    outdir = sys.argv[2]

    raw_records = parse_records(input_file)

    entries = []

    for r in raw_records:
        entry = parse_entry(r)
        if entry:
            entries.append(entry)

    write_chunks(entries, outdir)

    print(f"processed {len(entries)} entries")


if __name__ == "__main__":
    main()
