#!/bin/bash
set -euo pipefail

model="${RWKV_MODEL_PATH:-$HOME/Models/rwkv/rwkv7-g1l-1.5b-Q6_K.gguf}"
if ! command -v llama-cli >/dev/null 2>&1; then
  echo '请先安装运行时：brew install llama.cpp' >&2
  exit 1
fi
if [[ ! -f "$model" ]]; then
  echo "找不到模型：$model" >&2
  exit 1
fi

args=(-m "$model" -ngl 99 -c 4096 -n 256 --jinja --reasoning off
  --temp 1 --top-k 128 --top-p 0.5 --min-p 0
  --repeat-penalty 1 --frequency-penalty 0.5 --presence-penalty 0.5)
if [[ $# -gt 0 ]]; then
  args+=(--single-turn -p "$*")
fi
exec llama-cli "${args[@]}"
