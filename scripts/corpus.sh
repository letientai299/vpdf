#!/bin/sh
set -eu
corpus_dir="${VPDF_CORPUS:-${XDG_CACHE_HOME:-$HOME/.cache}/vpdf/corpus}"
mkdir -p "$corpus_dir"
fetch() {
  name=$1
  url=$2
  expected=$3
  target="$corpus_dir/$name.pdf"
  if [ ! -f "$target" ]; then
    download=$(mktemp "$corpus_dir/.download.XXXXXX")
    trap 'rm -f "$download"' EXIT HUP INT TERM
    curl --fail --location --retry 2 --max-time 120 "$url" -o "$download"
    mv "$download" "$target"
  fi
  if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$target" | cut -d ' ' -f 1)
  else
    actual=$(shasum -a 256 "$target" | cut -d ' ' -f 1)
  fi
  if [ "$actual" != "$expected" ]; then
    printf 'Corpus changed: %s (%s)\n' "$name" "$actual" >&2
    exit 1
  fi
  printf 'Verified %s\n' "$name"
}
fetch book https://greenteapress.com/thinkpython2/thinkpython2.pdf \
  6033000bac3f9268ad75ee041bf8e5d9a3bc8a33bb156399cbd014918cb59f94
fetch paper https://arxiv.org/pdf/1706.03762 \
  bdfaa68d8984f0dc02beaca527b76f207d99b666d31d1da728ee0728182df697
fetch form https://www.irs.gov/pub/irs-pdf/f1040.pdf \
  3d31c226df0d189ced80e039d01cf0f8820c1019681a0f0ca6264de277b7e982
fetch ocr https://www.w3.org/WAI/WCAG22/working-examples/pdf-ocr/ocr-example.pdf \
  7a161be656c7e82986a1959890eadbdcd0876e15e01b28fd00900190de15eed5
