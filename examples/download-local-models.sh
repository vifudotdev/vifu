#!/usr/bin/env zsh
# Download Vifu's local Qwen3-VL and Whisper models into the private model home.
set -euo pipefail

vifu_home="${VIFU_HOME:-$HOME/.vifu}"
model_dir="$vifu_home/models"
mkdir -p "$model_dir"

fetch_model() {
  local model_url=$1
  local model_name=$2
  local expected_bytes=$3
  local expected_sha256=$4
  local partial_path="$model_dir/$model_name.part"
  local final_path="$model_dir/$model_name"

  if test -f "$final_path" \
    && test "$(wc -c < "$final_path" | tr -d ' ')" = "$expected_bytes" \
    && test "$(shasum -a 256 "$final_path" | awk '{print $1}')" = "$expected_sha256"; then
    print "already-present $model_name"
    return
  fi
  print "downloading $model_name"
  curl -fL --retry 5 --retry-delay 5 --continue-at - --output "$partial_path" "$model_url"
  test "$(wc -c < "$partial_path" | tr -d ' ')" = "$expected_bytes"
  test "$(shasum -a 256 "$partial_path" | awk '{print $1}')" = "$expected_sha256"
  mv "$partial_path" "$final_path"
  print "complete $model_name"
}

qwen_revision="b93a7ee713758252c555be4210c00540df954dc2"
whisper_revision="5359861c739e955e79d9a303bcbc70fb988958b1"

fetch_model \
  "https://huggingface.co/unsloth/Qwen3-VL-8B-Instruct-GGUF/resolve/$qwen_revision/Qwen3-VL-8B-Instruct-Q4_K_M.gguf?download=true" \
  "Qwen3-VL-8B-Instruct-Q4_K_M.gguf" \
  "5027785568" \
  "108e7ff92b78eefd3db4741885104acba514255c11b617d3c7b197a5f46efe89"
fetch_model \
  "https://huggingface.co/unsloth/Qwen3-VL-8B-Instruct-GGUF/resolve/$qwen_revision/mmproj-F16.gguf?download=true" \
  "mmproj-F16.gguf" \
  "1159030336" \
  "d406d03ebabefdef86a2c86bf0c1b65f9e046f7a81c218f25de4931b46a07fc4"
fetch_model \
  "https://huggingface.co/ggerganov/whisper.cpp/resolve/$whisper_revision/ggml-base.en.bin?download=true" \
  "ggml-base.en.bin" \
  "147964211" \
  "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002"
