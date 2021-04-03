#!/usr/bin/env bash
set -ex
set -o pipefail
source $1
echo pkgver=$pkgver
echo source=$(echo $source | sed 's/.*\?:://')
echo sha256sums=$sha256sums
