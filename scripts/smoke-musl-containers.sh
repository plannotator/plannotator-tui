#!/bin/sh
# Run scripts/smoke-musl.sh against a musl binary in minimal containers: Debian Bullseye
# (glibc 2.31, older than the GNU binary needs) and Alpine (no glibc). Shared by the ci and
# release workflows so both test the same images. The binary path is relative to the repo root.
set -eu
binary="${1:?usage: sh scripts/smoke-musl-containers.sh <binary>}"
for image in debian:bullseye-slim alpine:3.24; do
  echo "== $image"
  docker run --rm -v "$PWD:/work:ro" -w /work "$image" sh scripts/smoke-musl.sh "$binary"
done
