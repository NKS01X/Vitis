#!/usr/bin/env bash
set -euo pipefail

echo "=========================================================="
echo " Starting Vitis Benchmarking Suite"
echo "=========================================================="
uname -a
lscpu | grep -E "Model name|CPU\(s\)|Thread|Core" || true
free -h | head -2 || true
echo "=========================================================="

if command -v docker &> /dev/null; then
    docker_start=$(date +%s.%N)
    docker build --no-cache -t vitis-bench -f bench/Dockerfile . >/dev/null
    docker_end=$(date +%s.%N)
    docker_duration=$(echo "$docker_end - $docker_start" | bc)
    printf "Docker build: %.2f seconds\n" "$docker_duration"

    b1_start=$(date +%s.%N)
    docker build --no-cache -f - . >/dev/null <<EOF
FROM ubuntu:24.04
WORKDIR /workspace
RUN apt-get update && apt-get install -y gcc make
COPY bench/src/hello.c ./hello.c
RUN gcc -O2 -o hello hello.c
EOF
    b1_end=$(date +%s.%N)
    b1_duration=$(echo "$b1_end - $b1_start" | bc)
    printf "Branch 1 (gcc): %.2f seconds\n" "$b1_duration"

    b2_start=$(date +%s.%N)
    docker build --no-cache -f - . >/dev/null <<EOF
FROM ubuntu:24.04
WORKDIR /workspace
RUN apt-get update && apt-get install -y nodejs npm
COPY bench/src/index.js ./index.js
RUN node index.js
EOF
    b2_end=$(date +%s.%N)
    b2_duration=$(echo "$b2_end - $b2_start" | bc)
    printf "Branch 2 (node): %.2f seconds\n" "$b2_duration"
fi

cargo bench --workspace -- --warm-up-time 1 --measurement-time 3
