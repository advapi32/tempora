#!/usr/bin/env bash
set -euo pipefail

cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."

find_iscc() {
    local folder compiler
    if command -v iscc || command -v ISCC.exe; then
        return
    fi

    for folder in "$(printenv 'ProgramFiles(x86)' || true)" \
        "${PROGRAMFILES:-${ProgramFiles:-}}" "${ProgramW6432:-}" \
        "${SystemDrive:-C:}/Program Files (x86)" "${SystemDrive:-C:}/Program Files" \
        "${LOCALAPPDATA:-}/Programs"; do
        [[ -n "$folder" ]] || continue
        compiler="$(cygpath -u "$folder")/Inno Setup 6/ISCC.exe"
        if [[ -f "$compiler" ]]; then
            printf '%s\n' "$compiler"
            return
        fi
    done
    return 1
}

if [[ ${1:-} == --find-iscc ]]; then
    find_iscc
    exit
fi

version=$(awk -F '"' '
    /^[[:space:]]*\[workspace\.package\][[:space:]]*$/ { in_package = 1; next }
    /^[[:space:]]*\[/ { in_package = 0 }
    in_package && /^[[:space:]]*version[[:space:]]*=/ { print $2; exit }
' Cargo.toml)
if [[ -z "$version" ]]; then
    printf '%s\n' 'Could not read the workspace version from Cargo.toml.' >&2
    exit 1
fi

target=x86_64-pc-windows-msvc
CARGO_TARGET_DIR=target cargo build --release --locked --target "$target"

mkdir -p dist
archive="Tempora-$version-x64-portable.zip"
staging=$(mktemp -d dist/.package.XXXXXXXX)
files=(tempora-recorder.exe tempora.exe README.md OFL-Geist.txt)
if [[ -f LICENSE ]]; then
    files+=(LICENSE)
fi

cleanup() {
    local file
    for file in "${files[@]}" "$archive"; do
        rm -f -- "$staging/$file"
    done
    rmdir -- "$staging"
}
trap cleanup EXIT

cp "target/$target/release/tempora-recorder.exe" "target/$target/release/tempora.exe" \
    README.md crates/tempora-dashboard/src/dashboard/fonts/OFL-Geist.txt "$staging/"
if [[ -f LICENSE ]]; then
    cp LICENSE "$staging/"
fi

# Use the libarchive tar shipped with Windows. Git Bash's GNU tar cannot write ZIP files.
system_tar="$(cygpath -u "${SystemRoot:-${SYSTEMROOT:-C:/Windows}}")/System32/tar.exe"
"$system_tar" --format zip -cf "$staging/$archive" -C "$staging" "${files[@]}"
mv -f -- "$staging/$archive" "dist/$archive"
printf 'Wrote dist/%s\n' "$archive"

if compiler=$(find_iscc); then
    MSYS2_ARG_CONV_EXCL='*' "$compiler" "/DAppVersion=$version" 'installer\Tempora.iss'
else
    printf '%s\n' 'Inno Setup 6 (ISCC.exe) was not found. The portable zip is ready. The installer was skipped.'
fi
