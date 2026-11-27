# WinGet category catalog

Tempora categorizes apps it has no stronger evidence for from a catalog built out of the [WinGet package manifests](https://github.com/microsoft/winget-pkgs). The catalog is built here, ahead of time, and embedded in the recorder as `crates/tempora/data/winget-catalog.bin`. Tempora itself never goes online.

The catalog stores no names or paths. It maps hashes of match keys to a category:

| Key | Matches |
|---|---|
| Uninstall entry name | MSI product codes, Inno Setup app IDs, installer names |
| Display name + publisher | the Uninstall entry's `DisplayName` and `Publisher` |
| Package family | Store apps |
| Executable name + publisher | portable apps, confirmed by the exe's `CompanyName` |

At runtime the recorder finds the Uninstall entry whose folder contains an executable and looks that entry up. The format, the keys and the text normalization are defined once, in `crates/tempora/src/recognition/catalog.rs`. The catalog is built by an example in that crate, so the build makes its keys with the same code the recorder looks them up with.

## Files

- `labels.tsv`: every package ID with its category and where the label came from (`rule`, `model`, `group`, `override`). This is the source of truth; edit `overrides.tsv` rather than this file.
- `overrides.tsv`: hand corrections, applied last.
- `PROMPT.md`: the labelling instructions given to the model, included at the top of every chunk.
- `extract.py`, `build_chunks.py`, `merge.py`: the pipeline below, ending with the `build_catalog` example in `crates/tempora/examples/`.
- `test_extract.py`: the manifest forms `extract.py` understands. Run it with `python -I -B scripts/catalog/test_extract.py` after changing the extractor.

## Rebuilding

Work in `winget-catalog/` at the repository root; it is ignored by Git. Run everything from the repository root, and the Python scripts with `python -I`.

1. Download the manifests. The shipped catalog was built from commit `6e9eedbe174aa13ea333d434e69cd6b6b3a3fe40`; download that commit to reproduce it byte for byte:
   ```
   curl -L -o winget-catalog/winget-pkgs.tar.gz https://codeload.github.com/microsoft/winget-pkgs/tar.gz/6e9eedbe174aa13ea333d434e69cd6b6b3a3fe40
   ```
   To update, download `refs/heads/master` instead. `extract.py` prints the commit the archive came from; write it above when the new catalog is committed.
2. Extract one record per package (names, descriptions, tags and match keys):
   ```
   python -I scripts/catalog/extract.py winget-catalog/winget-pkgs.tar.gz winget-catalog/catalog/packages.jsonl
   ```
3. Only to relabel: write paste-ready chunks, label them (by hand in a chat, or with a coding agent), then merge and check the answers:
   ```
   python -I scripts/catalog/build_chunks.py winget-catalog/catalog 150
   python -I scripts/catalog/merge.py winget-catalog/catalog
   ```
   Copy `winget-catalog/catalog/labels.tsv` over `scripts/catalog/labels.tsv` once the check reports no problems.
4. Build the catalog:
   ```
   cargo run -p tempora --example build_catalog -- winget-catalog/catalog/packages.jsonl scripts/catalog/labels.tsv crates/tempora/data/winget-catalog.bin
   ```
5. Run `cargo test -p tempora catalog`; it checks the embedded catalog against a few well-known apps.

Packages added to WinGet after the last labelling have no label and are left out until the next relabelling.
