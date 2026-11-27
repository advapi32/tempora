"""Check and merge the model's answers.

Usage: python -I merge.py <catalog dir>

Reads  <catalog>/chunks/chunk-NNN.txt      what was asked
       <catalog>/labels/chunk-NNN.tsv      the model's answer for that chunk (paste the code block)
       <catalog>/groups.tsv, prelabeled.tsv
       <catalog>/sample-hand-label.tsv     optional: your own labels in the last column
       overrides.tsv next to this script   hand corrections, applied last
Writes <catalog>/labels.tsv                every package ID -> category, with its source
Prints missing chunks, chunks to redo, and accuracy against your hand labels.
"""

import re
import sys
from collections import Counter, defaultdict
from pathlib import Path

CATEGORIES = {"Games", "Browsers", "Communication", "Media", "Productivity", "Development",
              "Creative", "Utilities", "Other", "Unknown"}


def asked_ids(chunk_file):
    text = chunk_file.read_text(encoding="utf-8")
    block = text.rsplit("```tsv", 1)[1].split("```", 1)[0]
    rows = [line for line in block.strip().splitlines()[1:] if line.strip()]
    return [row.split("\t", 1)[0] for row in rows]


def read_answer(answer_file):
    labels = {}
    for line in answer_file.read_text(encoding="utf-8-sig").splitlines():
        line = line.strip().strip("`")
        if not line or line.lower() in ("tsv", "id\tcategory"):
            continue
        parts = re.split(r"\t+|\s{2,}", line)
        if len(parts) >= 2:
            labels[parts[0].strip()] = parts[-1].strip().capitalize()
    return labels


def main(catalog):
    catalog = Path(catalog)
    labels, problems, missing = {}, [], []
    for chunk in sorted((catalog / "chunks").glob("chunk-*.txt")):
        answer = catalog / "labels" / (chunk.stem + ".tsv")
        if not answer.exists():
            missing.append(chunk.stem)
            continue
        ids = asked_ids(chunk)
        got = read_answer(answer)
        absent = [i for i in ids if i not in got]
        invalid = {i: got[i] for i in ids if i in got and got[i] not in CATEGORIES}
        extra = [i for i in got if i not in set(ids)]
        if absent or invalid or extra:
            problems.append(f"{chunk.stem}: {len(absent)} missing, {len(invalid)} invalid, "
                            f"{len(extra)} unexpected ids"
                            + (f" e.g. invalid {list(invalid.items())[:3]}" if invalid else ""))
        labels.update({i: got[i] for i in ids if got.get(i) in CATEGORIES})

    out_rows = []
    for line in (catalog / "prelabeled.tsv").read_text(encoding="utf-8").splitlines()[1:]:
        package, category, _ = line.split("\t")
        out_rows.append((package, category, "rule"))
    for line in (catalog / "groups.tsv").read_text(encoding="utf-8").splitlines()[1:]:
        package, rep = line.split("\t")
        if rep in labels:
            out_rows.append((package, labels[rep], "model" if package == rep else "group"))
    # Hand corrections win over rules and the model, for a package and for its whole group.
    overrides = {}
    override_file = Path(__file__).parent / "overrides.tsv"
    if override_file.exists():
        for line in override_file.read_text(encoding="utf-8").splitlines()[1:]:
            fields = line.split("\t")
            if len(fields) >= 2 and fields[0].strip():
                if fields[1] not in CATEGORIES:
                    problems.append(f"overrides.tsv: invalid category {fields[1]!r} for {fields[0]}")
                    continue
                overrides[fields[0]] = fields[1]
    representative = dict(line.split("\t") for line in
                          (catalog / "groups.tsv").read_text(encoding="utf-8").splitlines()[1:])
    known = {row[0] for row in out_rows}
    for package in overrides:
        if package not in known:
            problems.append(f"overrides.tsv: unknown package id {package}")
    out_rows = [
        (package, overrides[key], "override") if (key := next(
            (k for k in (package, representative.get(package)) if k in overrides), None))
        else (package, category, source)
        for package, category, source in out_rows
    ]

    with open(catalog / "labels.tsv", "w", encoding="utf-8") as out:
        out.write("id\tcategory\tsource\n")
        for row in sorted(out_rows, key=lambda r: r[0].lower()):
            out.write("\t".join(row) + "\n")

    total = len(list((catalog / "chunks").glob("chunk-*.txt")))
    print(f"answered chunks: {total - len(missing)} of {total}")
    if missing:
        print(f"not answered yet: {', '.join(missing[:10])}{' ...' if len(missing) > 10 else ''}")
    for problem in problems:
        print("REDO", problem)
    counts = Counter(category for _, category, _ in out_rows)
    print("categories:", ", ".join(f"{c} {n}" for c, n in counts.most_common()))

    hand = catalog / "sample-hand-label.tsv"
    pairs = []
    for line in hand.read_text(encoding="utf-8").splitlines()[1:]:
        fields = line.split("\t")
        if len(fields) >= 4 and fields[3].strip() and fields[0] in labels:
            pairs.append((fields[3].strip().capitalize(), labels[fields[0]]))
    if pairs:
        right = sum(1 for want, got in pairs if want == got)
        unknown = sum(1 for _, got in pairs if got == "Unknown")
        print(f"\nhand-label check: {right}/{len(pairs)} agree ({right / len(pairs):.0%}), "
              f"{unknown} answered Unknown")
        confusion = defaultdict(Counter)
        for want, got in pairs:
            if want != got:
                confusion[want][got] += 1
        for want, wrong in sorted(confusion.items()):
            print(f"  you said {want}: model said " + ", ".join(f"{g} ×{n}" for g, n in wrong.items()))


if __name__ == "__main__":
    main(sys.argv[1])
