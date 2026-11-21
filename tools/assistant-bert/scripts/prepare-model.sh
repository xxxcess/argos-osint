#!/usr/bin/env bash
# Stage and check the pinned, cased SQuAD checkpoint before publishing it locally.
set -euo pipefail

if [[ $# != 1 || -z "$1" ]]; then
  echo "usage: bash tools/assistant-bert/scripts/prepare-model.sh <new-model-dir>" >&2
  exit 2
fi

umask 077
mkdir -p -- "$(dirname "$1")"
parent_dir=$(cd "$(dirname "$1")" && pwd -P)
model_dir="$parent_dir/$(basename "$1")"
if [[ -e "$model_dir" || -L "$model_dir" ]]; then
  echo "Choose a new QA model directory." >&2
  exit 1
fi
staging=$(mktemp -d "$parent_dir/.assistant-bert-model.XXXXXX")
trap 'rm -rf -- "$staging"' EXIT
model_url=https://huggingface.co/distilbert/distilbert-base-cased-distilled-squad/resolve/564e9b582944a57a3e586bbb98fd6f0a4118db7f
for file in config.json vocab.txt rust_model.ot; do
  curl -fsSL --retry 3 --max-time 300 --max-filesize 261000000 "$model_url/$file" -o "$staging/$file"
done
(
  cd "$staging"
  shasum -a 256 -c <<'CHECKSUMS'
0b5cb15ec08645604ef7085acfaf9c4131158ac22207a76634574cf2771b1515  config.json
eeaa9875b23b04b4c54ef759d03db9d1ba1554838f8fb26c5d96fa551df93d02  vocab.txt
8a9f9b2f153ac9ff230aca4548fa3286be9d2f9ea4eb7e9169665b1a8e983f44  rust_model.ot
CHECKSUMS
)
python3 - "$staging" "$model_dir" <<'PY'
import os
import sys

if os.path.lexists(sys.argv[2]):
    raise SystemExit("QA model destination already exists")
os.rename(sys.argv[1], sys.argv[2])
PY
