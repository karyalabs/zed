#!/bin/sh
# Build the gpui-pre-compatible crates from the current commit and publish
# them as the orphan branch crates/gpui-pre-<version>, which apps use as a
# git source in [patch.crates-io] (see karya/README.md).
#
#   karya/publish.sh 0.3.12
set -eu
version=$1
root=$(cd "$(dirname "$0")/.." && pwd)
head=$(git -C "$root" rev-parse HEAD)
out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT
python3 "$root/karya/gpui-pre.py" --version "$version" --out "$out/crates"
cd "$out/crates"
git init --quiet
git checkout --quiet -b "crates/gpui-pre-$version"
git add -A
git -c user.name="Claude" -c user.email="noreply@anthropic.com" \
    commit --quiet -m "gpui-pre $version crates from karyalabs/zed@$head"
git push --force "$(git -C "$root" remote get-url origin)" "crates/gpui-pre-$version"
