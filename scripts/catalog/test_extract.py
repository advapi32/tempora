"""Checks the manifest forms extract.py understands.

Usage: python -I -B scripts/catalog/test_extract.py
"""

import pathlib
import sys
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from extract import parse  # noqa: E402

INSTALLER = """\
# yaml-language-server: $schema=https://aka.ms/winget-manifest.installer.1.9.0.schema.json
PackageIdentifier: Example.Tool
PackageVersion: 1.2.0
InstallerType: zip
Commands: [tool, 'tool-cli', "tool two"]  # a comment
FileExtensions:
- txt
- 'md'
Protocols: []
Installers:
- Architecture: x64
  ProductCode: '{8EB86E9F-9120-43CF-9D1B-2810CD854BCD}_is1'
  NestedInstallerFiles:
  - RelativeFilePath: bin\\tool.exe
    PortableCommandAlias: tool
  AppsAndFeaturesEntries:
  - DisplayName: "Example Tool"
    Publisher: Example Ltd. # trailing comment
  Commands:
  - tool-admin
ManifestType: installer
"""

LOCALE = """\
PackageIdentifier: Example.Tool
PackageLocale: en-US
Publisher: Example
PackageName: Example Tool
ShortDescription: |-
  Does a thing,
  and another.
Tags: [cli, "utility"]
ManifestType: defaultLocale
"""

COMMENTS = """\
PackageName: "C # Studio" # the name has a hash
Publisher: O'Reilly # an apostrophe is not a quote
Moniker: c#
Tags: ["C # toolkit", 'sdk'] # a comment after the list
"""


class Parse(unittest.TestCase):
    def test_top_level_scalars(self):
        top, _, _, _ = parse(INSTALLER)
        self.assertEqual(top["PackageIdentifier"], "Example.Tool")
        self.assertEqual(top["ManifestType"], "installer")

    def test_flow_lists_with_quotes_and_comments(self):
        _, top_lists, _, nested_lists = parse(INSTALLER)
        self.assertEqual(top_lists["Commands"], ["tool", "tool-cli", "tool two"])
        self.assertNotIn("Protocols", top_lists)
        self.assertIn("tool-admin", nested_lists["Commands"])

    def test_block_lists(self):
        _, top_lists, _, _ = parse(INSTALLER)
        self.assertEqual(top_lists["FileExtensions"], ["txt", "md"])

    def test_nested_scalars_inside_list_items(self):
        _, _, nested, _ = parse(INSTALLER)
        self.assertEqual(nested["ProductCode"], ["{8EB86E9F-9120-43CF-9D1B-2810CD854BCD}_is1"])
        self.assertEqual(nested["RelativeFilePath"], ["bin\\tool.exe"])
        self.assertEqual(nested["PortableCommandAlias"], ["tool"])
        self.assertEqual(nested["DisplayName"], ["Example Tool"])
        self.assertEqual(nested["Publisher"], ["Example Ltd."])

    def test_block_scalars_and_tags(self):
        top, top_lists, _, _ = parse(LOCALE)
        self.assertEqual(top["ShortDescription"], "Does a thing, and another.")
        self.assertEqual(top_lists["Tags"], ["cli", "utility"])
        self.assertEqual(top["PackageName"], "Example Tool")

    def test_a_hash_inside_quotes_is_text(self):
        top, top_lists, _, _ = parse(COMMENTS)
        self.assertEqual(top["PackageName"], "C # Studio")
        self.assertEqual(top["Publisher"], "O'Reilly")
        self.assertEqual(top["Moniker"], "c#")
        self.assertEqual(top_lists["Tags"], ["C # toolkit", "sdk"])


if __name__ == "__main__":
    unittest.main()
