#!/usr/bin/env bash
# Install the pinned Atlas checkpoint into a new directory.
set -euo pipefail

if [[ $# != 1 || -z "$1" ]]; then
  echo "usage: bash tools/atlas-bert/scripts/prepare-model.sh <new-model-dir>" >&2
  exit 2
fi

worker_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
mkdir -p -- "$(dirname "$1")"
parent_dir=$(cd "$(dirname "$1")" && pwd)
model_dir="$parent_dir/$(basename "$1")"
if [[ -e "$model_dir" || -L "$model_dir" ]]; then
  echo "Choose a new model directory: $model_dir already exists." >&2
  exit 1
fi

work_dir=$(mktemp -d "$parent_dir/.atlas-bert-model.XXXXXX")
trap 'rm -rf -- "$work_dir"' EXIT
model="$work_dir/model"
converter="$work_dir/converter"
mkdir -p "$model" "$converter/src/bin" "$converter/utils"

model_url=https://huggingface.co/typeform/distilbert-base-uncased-mnli/resolve/cfa538a0fddbbd978fefe8966c1aeff7ad409c90
for file in config.json vocab.txt pytorch_model.bin; do
  curl -fsSL --retry 3 --max-time 180 "$model_url/$file" -o "$model/$file"
done
(
  cd "$model"
  shasum -a 256 -c <<'CHECKSUMS'
d6d658b44d7260410d8aa3f6cd585016656dfe57dd57855c070f86ddcb257385  config.json
07eced375cec144d27c900241f3e339478dec958f92fddbc551f295c992038a3  vocab.txt
e9c34cf80a78f251571afee70b7139ce1c5137a4c35a5a984ae519337e2092d0  pytorch_model.bin
CHECKSUMS
)

curl -fsSL --retry 3 --max-time 60 https://static.crates.io/crates/rust-bert/rust-bert-0.22.0.crate \
  -o "$converter/rust-bert.crate"
# Reuse the worker's maintained lock instead of the upstream crate's old lock.
python - "$worker_dir/Cargo.lock" "$converter/rust-bert.crate" <<'PY'
import hashlib
from pathlib import Path
import sys
import tomllib

packages = tomllib.loads(Path(sys.argv[1]).read_text())["package"]
expected = next(p["checksum"] for p in packages if p["name"] == "rust-bert" and p["version"] == "0.22.0")
if hashlib.sha256(Path(sys.argv[2]).read_bytes()).hexdigest() != expected:
    raise SystemExit("Rust-BERT converter archive checksum mismatch")
PY
tar -xOf "$converter/rust-bert.crate" rust-bert-0.22.0/utils/convert_model.py > "$converter/utils/convert_model.py"
tar -xOf "$converter/rust-bert.crate" rust-bert-0.22.0/src/convert-tensor.rs > "$converter/src/bin/convert-tensor.rs"
cp "$worker_dir/Cargo.toml" "$worker_dir/Cargo.lock" "$converter/"

if [[ ${LIBTORCH_USE_PYTORCH:-} == 1 || -z ${LIBTORCH:-} ]]; then
  torch_lib=$(python -c 'import pathlib, torch; print(pathlib.Path(torch.__file__).parent / "lib")')
  export LIBTORCH_USE_PYTORCH=1
else
  torch_lib="$LIBTORCH/lib"
fi
if [[ $(uname -s) == Darwin ]]; then
  export DYLD_LIBRARY_PATH="$torch_lib${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}"
else
  export LD_LIBRARY_PATH="$torch_lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
fi
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$worker_dir/target}"
cargo build --locked --manifest-path "$converter/Cargo.toml" --bin convert-tensor
TORCH_FORCE_WEIGHTS_ONLY_LOAD=1 CARGO_NET_OFFLINE=true \
  python "$converter/utils/convert_model.py" "$model/pytorch_model.bin"
test -s "$model/rust_model.ot"
rm "$model/pytorch_model.bin" "$model/model.npz"

python - "$model" "$model_dir" <<'PY'
import os
import sys

os.rename(sys.argv[1], sys.argv[2])
PY
printf 'Atlas checkpoint ready at %s\n' "$model_dir"
