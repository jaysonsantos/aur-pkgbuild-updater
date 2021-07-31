#!/bin/bash
set -ex
set -o pipefail
here=$(readlink -f $0)
here=$(dirname $here)

git clone $2 $1
cd $1
cat $here/OUTDATED_PKGBUILD > PKGBUILD
git add PKGBUILD
git commit -m'Initial version'
git push
