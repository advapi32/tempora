"""Turn packages.jsonl into paste-ready chunks for a chat model.

Usage: python -I build_chunks.py <catalog dir> [rows per chunk]

Reads  <catalog>/packages.jsonl, and PROMPT.md next to this script
Writes <catalog>/prelabeled.tsv      packages labelled by rule, with the reason
       <catalog>/groups.tsv          every package ID -> the representative that gets labelled
       <catalog>/chunks/chunk-NNN.txt  prompt + rows, paste one per message
       <catalog>/sample-hand-label.tsv  random representatives to label by hand (optional)
       <catalog>/stats.txt
"""

import hashlib
import json
import random
import re
import sys
from collections import defaultdict
from pathlib import Path

RULE_OTHER_NAME = re.compile(
    r"\b(runtime|redistributable|redist|sdk|drivers?|codec pack)\b", re.I)
# Typefaces only: a font name alone also matches title designers and font managers.
RULE_FONT_NAME = re.compile(r"\b(fonts?|nerd font)\b", re.I)
RULE_TYPEFACE = re.compile(r"\b(typefaces?|font famil(y|ies)|monospaced?)\b", re.I)
RULE_OTHER_ID = re.compile(
    r"^(Microsoft\.(VCRedist|DotNet|WindowsSDK|WindowsAppRuntime|UI\.Xaml|VCLibs)|"
    r"Oracle\.JavaRuntime|.*\.(JRE|JDK)(\.|$))", re.I)
STORE_ID = re.compile(r"^[0-9a-z]{12}$")
NOISE = re.compile(r"\(.*?\)|\b(x64|x86|arm64|64-bit|32-bit|portable|beta|preview|nightly|"
                   r"insiders?|canary|dev|esr|lts|stable)\b|\bv?\d+(\.\d+)*\b", re.I)


def clean(text, limit):
    text = re.sub(r"\s+", " ", (text or "").replace("\t", " ")).strip()
    return text if len(text) <= limit else text[: limit - 1].rstrip() + "…"


def group_key(package):
    name = NOISE.sub(" ", package["name"] or package["id"]).lower()
    name = re.sub(r"[^a-z0-9]+", " ", name).strip()
    publisher = re.sub(r"[^a-z0-9]+", " ", (package["publisher"] or "").lower()).strip()
    return (publisher, name) if name else ("", package["id"].lower())


def main(catalog, per_chunk):
    catalog = Path(catalog)
    packages = [json.loads(line) for line in open(catalog / "packages.jsonl", encoding="utf-8")]
    prompt = (Path(__file__).parent / "PROMPT.md").read_text(encoding="utf-8").strip()

    prelabeled, rest = [], []
    for package in packages:
        name = package["name"] or ""
        if RULE_OTHER_ID.match(package["id"]):
            prelabeled.append((package["id"], "Other", "runtime/SDK package ID"))
        elif RULE_OTHER_NAME.search(name):
            prelabeled.append((package["id"], "Other", "runtime/SDK/driver name"))
        elif RULE_FONT_NAME.search(name) and RULE_TYPEFACE.search(
                f"{name} {package.get('description') or ''}"):
            prelabeled.append((package["id"], "Other", "typeface"))
        else:
            rest.append(package)

    groups = defaultdict(list)
    for package in rest:
        groups[group_key(package)].append(package)
    representatives = []
    with open(catalog / "groups.tsv", "w", encoding="utf-8") as out:
        out.write("id\trepresentative\n")
        for members in groups.values():
            # The representative is the member with the richest text.
            rep = max(members, key=lambda p: (len(p["description"] or ""), len(p["tags"]), -len(p["id"])))
            representatives.append(rep)
            for member in members:
                out.write(f"{member['id']}\t{rep['id']}\n")

    # Stable but shuffled order, so one chunk isn't all one publisher.
    representatives.sort(key=lambda p: hashlib.sha1(p["id"].encode()).hexdigest())

    with open(catalog / "prelabeled.tsv", "w", encoding="utf-8") as out:
        out.write("id\tcategory\treason\n")
        for row in prelabeled:
            out.write("\t".join(row) + "\n")

    chunk_dir = catalog / "chunks"
    chunk_dir.mkdir(exist_ok=True)
    for old in chunk_dir.glob("chunk-*.txt"):
        old.unlink()
    rows = []
    for package in representatives:
        tags = [t for t in package["tags"] if not STORE_ID.match(t.lower())][:6]
        rows.append("\t".join([
            package["id"], clean(package["name"], 80), clean(package["publisher"], 60),
            clean(package["description"], 200), clean(", ".join(tags), 100),
        ]))
    header = "id\tname\tpublisher\tdescription\ttags"
    chunks = [rows[i:i + per_chunk] for i in range(0, len(rows), per_chunk)]
    for number, chunk in enumerate(chunks, 1):
        body = "\n".join([header, *chunk])
        (chunk_dir / f"chunk-{number:03}.txt").write_text(
            f"{prompt}\n\n## Packages ({len(chunk)} rows)\n\n```tsv\n{body}\n```\n", encoding="utf-8")

    sample = random.Random(27).sample(representatives, min(150, len(representatives)))
    with open(catalog / "sample-hand-label.tsv", "w", encoding="utf-8") as out:
        out.write("id\tname\tdescription\tcategory\n")
        for package in sample:
            out.write(f"{package['id']}\t{clean(package['name'], 80)}\t"
                      f"{clean(package['description'], 120)}\t\n")

    keys = defaultdict(int)
    for package in packages:
        for kind, values in package["keys"].items():
            keys[kind] += len(values)
    match_keys = sum(keys[k] for k in ("ProductCode", "UpgradeCode", "PackageFamilyName",
                                       "RelativeFilePath", "PortableCommandAlias"))
    # Hashed binary estimate: 8-byte key hash + 4-byte package index per key,
    # plus per package 1 category byte and a ~24-byte display name.
    estimate = match_keys * 12 + len(packages) * 25
    no_text = sum(1 for p in representatives if not p["description"] and not p["tags"])
    stats = [
        f"packages:                 {len(packages)}",
        f"labelled by rule (Other): {len(prelabeled)}",
        f"left for the model:       {len(rest)} packages in {len(representatives)} groups",
        f"groups with no description or tags: {no_text}",
        f"chunks:                   {len(chunks)} of up to {per_chunk} rows",
        "",
        "matching keys across all versions:",
        *[f"  {kind:22} {count}" for kind, count in sorted(keys.items())],
        "",
        f"shipped catalog estimate (hashed binary): {estimate / 1024 / 1024:.2f} MB",
    ]
    (catalog / "stats.txt").write_text("\n".join(stats) + "\n", encoding="utf-8")
    print("\n".join(stats))


if __name__ == "__main__":
    main(sys.argv[1], int(sys.argv[2]) if len(sys.argv) > 2 else 150)
