"""Stream the winget-pkgs archive and write one JSON record per package.

Usage: python -I extract.py <winget-pkgs.tar.gz> <out packages.jsonl>

The archive is read as a stream; nothing is extracted to disk. Only a handful of fields are
parsed, line by line, so no YAML library is needed. The forms it understands are the ones
test_extract.py checks: plain and quoted scalars, block scalars, block lists and flow lists.

Prints the winget-pkgs commit the archive was made from; record it in README.md.
"""

import json
import re
import sys
import tarfile
from collections import defaultdict

# Scalar fields wanted from locale manifests (top level only).
LOCALE_FIELDS = {"PackageIdentifier", "PackageVersion", "PackageLocale", "Publisher",
                 "PackageName", "ShortDescription", "Moniker", "ManifestType"}
# Scalar keys collected anywhere in installer manifests, across every version.
KEY_SCALARS = {"ProductCode", "UpgradeCode", "PackageFamilyName", "RelativeFilePath",
               "PortableCommandAlias", "DisplayName", "Publisher", "InstallerType"}
# List keys collected anywhere in installer manifests.
KEY_LISTS = {"FileExtensions", "Protocols", "Commands"}

LINE = re.compile(r"^(\s*)(?:-\s+)?([A-Za-z]+):\s*(.*?)\s*$")
ITEM = re.compile(r"^(\s*)-\s+(.*?)\s*$")


def strip_comment(value):
    """The value without a trailing `# comment`. A `#` inside quotes is text; a quote only opens
    a quoted scalar at the start of the value or of a flow list item, so O'Reilly is plain."""
    quote = None
    for index, character in enumerate(value):
        if quote:
            quote = None if character == quote else quote
        elif character in "'\"" and value[:index].rstrip()[-1:] in ("", "[", ","):
            quote = character
        elif character == "#" and (index == 0 or value[index - 1] in " \t"):
            return value[:index].rstrip()
    return value


def unquote(value):
    value = strip_comment(value)
    if len(value) >= 2 and value[0] == value[-1] and value[0] in "'\"":
        return value[1:-1]
    return value


def flow_items(value):
    """The items of a flow list such as `[pwsh, 'pull-watch']`, or None for any other value."""
    value = strip_comment(value)
    if not (value.startswith("[") and value.endswith("]")):
        return None
    items, current, quote = [], "", None
    for character in value[1:-1]:
        if quote:
            quote = None if character == quote else quote
        elif character in "'\"" and not current.strip():
            quote = character
        elif character == ",":
            items.append(current)
            current = ""
            continue
        current += character
    items.append(current)
    return [unquote(item.strip()) for item in items if item.strip()]


def parse(text):
    """Return (top-level scalars, top-level lists, nested scalars, nested lists)."""
    top, top_lists = {}, defaultdict(list)
    nested, nested_lists = defaultdict(list), defaultdict(list)
    lines = text.splitlines()
    i = 0
    while i < len(lines):
        line = lines[i]
        i += 1
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        m = LINE.match(line)
        if not m:
            continue
        indent, key, value = len(m.group(1)), m.group(2), m.group(3)
        if value in ("|", "|-", ">", ">-", "|+", ">+"):
            block = []
            while i < len(lines) and (not lines[i].strip() or
                                      len(lines[i]) - len(lines[i].lstrip()) > indent):
                block.append(lines[i].strip())
                i += 1
            value = " ".join(part for part in block if part)
        elif (items := flow_items(value)) is not None or value == "":
            if items is None:
                items = []
                while i < len(lines):
                    item = ITEM.match(lines[i])
                    if not item or len(item.group(1)) < indent or LINE.match(lines[i]):
                        break
                    items.append(unquote(item.group(2)))
                    i += 1
            if items:
                (top_lists if indent == 0 else nested_lists)[key].extend(items)
                if indent == 0:
                    nested_lists[key].extend(items)
            continue
        value = unquote(value)
        if indent == 0 and not line.lstrip().startswith("-"):
            top[key] = value
        nested[key].append(value)
    return top, top_lists, nested, nested_lists


def version_key(version):
    parts = re.split(r"[^0-9A-Za-z]+", version or "")
    return tuple((int(p), "") if p.isdigit() else (-1, p.lower()) for p in parts if p)


def main(archive, out_path):
    packages = defaultdict(lambda: {
        "locale": None, "locale_version": (), "en": None, "en_version": (),
        "keys": defaultdict(set),
    })
    files = 0
    commit = None
    with tarfile.open(archive, "r|gz") as tar:
        for member in tar:
            commit = commit or tar.pax_headers.get("comment")
            if not member.isfile() or not member.name.endswith(".yaml"):
                continue
            if "/manifests/" not in member.name:
                continue
            handle = tar.extractfile(member)
            if handle is None:
                continue
            text = handle.read().decode("utf-8-sig", "replace")
            files += 1
            top, top_lists, nested, nested_lists = parse(text)
            package_id = top.get("PackageIdentifier")
            kind = top.get("ManifestType", "")
            if not package_id:
                continue
            record = packages[package_id]
            version = version_key(top.get("PackageVersion"))
            if kind in ("defaultLocale", "locale", "singleton"):
                info = {k: v for k, v in top.items() if k in LOCALE_FIELDS}
                info["Tags"] = top_lists.get("Tags", [])
                is_en = top.get("PackageLocale", "").lower().startswith("en")
                if kind in ("defaultLocale", "singleton") and version >= record["locale_version"]:
                    record["locale"], record["locale_version"] = info, version
                if is_en and version >= record["en_version"]:
                    record["en"], record["en_version"] = info, version
            if kind in ("installer", "singleton"):
                for key in KEY_SCALARS:
                    record["keys"][key].update(v for v in nested.get(key, []) if v)
                for key in KEY_LISTS:
                    record["keys"][key].update(v for v in nested_lists.get(key, []) if v)
            if files % 100000 == 0:
                print(f"{files} manifests, {len(packages)} packages", file=sys.stderr)

    with open(out_path, "w", encoding="utf-8") as out:
        for package_id in sorted(packages, key=str.lower):
            record = packages[package_id]
            info = dict(record["locale"] or {})
            # Prefer English text when the default locale isn't English.
            if record["en"] and not info.get("PackageLocale", "").lower().startswith("en"):
                info.update({k: v for k, v in record["en"].items() if v})
            out.write(json.dumps({
                "id": package_id,
                "name": info.get("PackageName", ""),
                "publisher": info.get("Publisher", ""),
                "moniker": info.get("Moniker", ""),
                "description": info.get("ShortDescription", ""),
                "tags": info.get("Tags", []),
                "keys": {k: sorted(v) for k, v in record["keys"].items() if v},
            }, ensure_ascii=False) + "\n")
    print(f"done: {files} manifests, {len(packages)} packages", file=sys.stderr)
    print(f"winget-pkgs commit: {commit or 'unknown'}", file=sys.stderr)


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
