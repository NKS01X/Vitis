# syntax=docker/dockerfile:1
# Benchmark Dockerfile: parallel gcc + nodejs build (2-branch DAG)
# Used by: bench/run.sh to measure sequential Docker build time.

FROM ubuntu:24.04

WORKDIR /workspace

# Branch 1: C/gcc toolchain
RUN apt-get update && apt-get install -y gcc make
COPY bench/src/hello.c ./hello.c
RUN gcc -O2 -o hello hello.c

# Branch 2: Node.js toolchain
RUN apt-get update && apt-get install -y nodejs npm
COPY bench/src/index.js ./index.js
RUN node index.js

# Merge: collect artefacts
RUN tar -czf release.tar.gz hello index.js
