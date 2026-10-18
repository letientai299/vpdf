#!/bin/sh
# Usage: corpus.sh [corpus|huge] [name ...]
# Fetches selected PDFs and verifies SHA-256 hashes.
set -eu
set_name=${1:-corpus}
if [ "$#" -gt 0 ]; then shift; fi
selection=$*
dir="$(cd "$(dirname "$0")/.." && pwd)/tmp/$set_name"
mkdir -p "$dir"

fetch() (
  name=$1 url=$2 expected=$3
  target="$dir/$name.pdf"
  # A partial download stays and resumes on the next run; a transfer
  # under 1 KB/s for a minute counts as stalled and retries.
  part="$dir/.$name.part"
  if [ ! -f "$target" ]; then
    curl --fail --silent --show-error --location --continue-at - \
      --retry 5 --retry-all-errors --speed-limit 1024 --speed-time 60 \
      "$url" -o "$part"
    mv "$part" "$target"
  fi
  if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$target" | cut -d ' ' -f 1)
  else
    actual=$(shasum -a 256 "$target" | cut -d ' ' -f 1)
  fi
  if [ "$actual" != "$expected" ]; then
    rm -f "$target"
    printf 'Corpus changed: %s (%s), removed\n' "$name" "$actual" >&2
    exit 1
  fi
  printf 'Verified %s\n' "$name"
)

# Name, URL, and SHA-256 per file.
case $set_name in
  corpus)
    set -- \
      book https://greenteapress.com/thinkpython2/thinkpython2.pdf \
      6033000bac3f9268ad75ee041bf8e5d9a3bc8a33bb156399cbd014918cb59f94 \
      paper https://arxiv.org/pdf/1706.03762 \
      bdfaa68d8984f0dc02beaca527b76f207d99b666d31d1da728ee0728182df697 \
      form https://www.irs.gov/pub/irs-pdf/f1040.pdf \
      3d31c226df0d189ced80e039d01cf0f8820c1019681a0f0ca6264de277b7e982 \
      ocr https://www.w3.org/WAI/WCAG22/working-examples/pdf-ocr/ocr-example.pdf \
      7a161be656c7e82986a1959890eadbdcd0876e15e01b28fd00900190de15eed5 \
      scan https://web.mit.edu/sts.035/www/PDFs/edvac.pdf \
      cc591b48ea4989a72ef725f1d2fc8eb35ab556e8e24f1cc3db33ec56abc74e32
    ;;
  huge)
    # 5,364-page manual; 3,329-page manual; layered vector maps; JPEG 2000
    # and JBIG2 scan; 12881x4993 image poster; soft masks and blend modes;
    # CMYK photo book; Japanese text with heavy shadings.
    set -- \
      sdm https://cdrdv2.intel.com/v1/dl/getContent/671200 \
      a4a62e6a7ba11a76c7753a195b825087306812aac39b930973f9168ee599f321 \
      postgres https://www.postgresql.org/files/documentation/pdf/18/postgresql-18-US.pdf \
      625d8080532051a4e54ee53bc54eeb6a9fa5ef6fc8053a848101a901c996198b \
      topo-nyc https://prd-tnm.s3.amazonaws.com/StagedProducts/Maps/USTopo/PDF/NY/NY_Central_Park_20160824_TM_geo.pdf \
      a3be2f4f2a3b993e6373e214fb14fbdd3753efb1736c67cfc134bf098b07d8f5 \
      topo-sf https://prd-tnm.s3.amazonaws.com/StagedProducts/Maps/USTopo/PDF/CA/CA_San_Francisco_North_20150311_TM_geo.pdf \
      7166940f677b9bd93735b92c2aaa47423c095fd14bdefcc863ec76ac62ae539c \
      darwin https://archive.org/download/originofspecies00darwuoft/originofspecies00darwuoft.pdf \
      6f04f39b2a590a7864c4e9d24fa17da24dea26f4e0a8b8cbc70fc11484c2aa3c \
      moon https://pubs.usgs.gov/sim/3316/downloads/sim3316_sheet1.pdf \
      2f8d988b02cfd0fb0fecddc1463c284c2bb186b1c8c77e142796d423e943ad37 \
      apple https://www.apple.com/environment/pdf/Apple_Environmental_Progress_Report_2024.pdf \
      87e86dbe8545c2006ad4e9af3e7e01b0ff713d4535171f924e06c32cc1bda5ba \
      blend https://raw.githubusercontent.com/mozilla/pdf.js/523aa9c1524df393645bb03ef233e0a4adc007b7/test/pdfs/blendmode.pdf \
      39ce34c2d32738c7c94970fd76559ecb9efd671f5ab99e515a853674e7a824ce \
      nasa https://www.nasa.gov/wp-content/uploads/2019/11/earth_at_night_508.pdf \
      8e622ca8f6d1ba0cf809549bddfee69e6754c3a3480d151c1fb54baf49b09be0 \
      cjk https://www.soumu.go.jp/johotsusintokei/whitepaper/ja/r07/pdf/00zentai.pdf \
      57afcba821c0975d195de01e20bb7e3e57167cfea0d06ef385dcb988aca14170
    ;;
  *)
    printf 'Unknown corpus set: %s\n' "$set_name" >&2
    exit 2
    ;;
esac

pids=
matched=
while [ $# -ge 3 ]; do
  case " $selection " in
    "  "|*" $1 "*)
      fetch "$1" "$2" "$3" &
      pids="$pids $!"
      matched="$matched $1"
      ;;
  esac
  shift 3
done
status=0
for pid in $pids; do
  wait "$pid" || status=1
done
for name in $selection; do
  case " $matched " in
    *" $name "*) ;;
    *) printf 'Unknown PDF: %s\n' "$name" >&2; status=2 ;;
  esac
done
exit $status
