#!/usr/bin/env python3
"""Turn this branch's GPUI changes into gpui-pre-compatible crates.

gpui-pre (crates.io) is a weekly republish of Zed's GPUI crates under new
names, with small edits of its own. Our patches live in Zed's layout on this
branch, on top of the Zed commit a gpui-pre release was cut from. This script
downloads that gpui-pre release, applies our diff to each crate we changed,
and writes those crates (same names, same versions) to an output folder.
An app then points `[patch.crates-io]` at them.

Usage:
    karya/gpui-pre.py --version 0.3.12 --out ../gpui-pre-karya

Only crates this branch changes are written; every other gpui-pre crate keeps
coming from crates.io. A change to a crate's Cargo.toml cannot be mapped
(gpui-pre rewrites manifests), so it stops the script: put manifest edits in
karya/manifests/<gpui-pre crate>.toml.patch instead (a unified diff against
the gpui-pre manifest).
"""

import argparse
import io
import pathlib
import shutil
import subprocess
import sys
import tarfile
import tomllib
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent
UA = {"User-Agent": "karya-gpui-pre-builder"}


def fetch_crate(name: str, version: str) -> tarfile.TarFile:
    url = f"https://static.crates.io/crates/{name}/{name}-{version}.crate"
    with urllib.request.urlopen(urllib.request.Request(url, headers=UA)) as r:
        return tarfile.open(fileobj=io.BytesIO(r.read()), mode="r:gz")


def manifest(tar: tarfile.TarFile, name: str, version: str) -> dict:
    member = tar.extractfile(f"{name}-{version}/Cargo.toml")
    return tomllib.load(member)


def gpui_pre_deps(m: dict) -> set[str]:
    out = set()
    tables = [m.get("dependencies", {}), m.get("dev-dependencies", {})]
    for target in m.get("target", {}).values():
        tables.append(target.get("dependencies", {}))
    for table in tables:
        for key, spec in table.items():
            pkg = spec.get("package", key) if isinstance(spec, dict) else key
            if pkg.startswith("gpui-pre"):
                out.add(pkg)
    return out


def crate_map(version: str) -> tuple[dict[str, str], str]:
    """Map Zed crate dir -> gpui-pre crate name, and return the Zed commit."""
    seen, todo, mapping, rev = set(), ["gpui-pre", "gpui-pre-platform"], {}, None
    while todo:
        name = todo.pop()
        if name in seen:
            continue
        seen.add(name)
        tar = fetch_crate(name, version)
        m = manifest(tar, name, version)
        meta = m.get("package", {}).get("metadata", {}).get("gpui-pre", {})
        if "zed-crate" in meta:
            mapping[meta["zed-crate"]] = name
            rev = rev or meta.get("zed-rev")
        todo.extend(gpui_pre_deps(m) - seen)
    if not rev:
        sys.exit("no zed-rev found in gpui-pre metadata")
    return mapping, rev


def git(*args: str) -> str:
    return subprocess.run(
        ["git", "-C", str(ROOT), *args], check=True, capture_output=True, text=True
    ).stdout


def apply(diff: str, cwd: pathlib.Path, strip: int) -> None:
    subprocess.run(
        ["patch", f"-p{strip}", "--batch", "--forward", "--quiet"],
        input=diff, text=True, check=True, cwd=cwd,
    )


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--version", required=True, help="gpui-pre version, e.g. 0.3.12")
    ap.add_argument("--out", required=True, type=pathlib.Path)
    args = ap.parse_args()

    mapping, rev = crate_map(args.version)
    print(f"gpui-pre {args.version} is zed@{rev[:7]}; {len(mapping)} crates mapped")

    changed = sorted(
        {p.split("/")[1] for p in git("diff", "--name-only", rev, "HEAD", "--", "crates/").split()}
    )
    if not changed:
        sys.exit("this branch changes no crates")

    out = args.out.resolve()
    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)

    for zed_crate in changed:
        if zed_crate not in mapping:
            sys.exit(f"crates/{zed_crate} is changed but is not part of gpui-pre {args.version}")
        name = mapping[zed_crate]
        files = git("diff", "--name-only", rev, "HEAD", "--", f"crates/{zed_crate}/").split()
        if f"crates/{zed_crate}/Cargo.toml" in files:
            sys.exit(f"crates/{zed_crate}/Cargo.toml changed; use karya/manifests/{name}.toml.patch")

        dest = out / name
        with fetch_crate(name, args.version) as tar:
            tar.extractall(out, filter="data")
        (out / f"{name}-{args.version}").rename(dest)
        for junk in (".cargo_vcs_info.json", "Cargo.toml.orig", "Cargo.lock"):
            (dest / junk).unlink(missing_ok=True)

        # `patch`, not `git apply`: git apply resolves paths against any repo
        # the output folder happens to sit in.
        diff = git("diff", rev, "HEAD", "--", f"crates/{zed_crate}/")
        apply(diff, dest, strip=3)
        manifest_patch = ROOT / "karya" / "manifests" / f"{name}.toml.patch"
        if manifest_patch.exists():
            apply(manifest_patch.read_text(), dest, strip=1)
        print(f"  {zed_crate} -> {name} ({len(files)} files)")

    head = git("rev-parse", "HEAD").strip()
    (out / "README.md").write_text(
        f"gpui-pre {args.version} crates patched by karyalabs/zed@{head[:7]} "
        f"(base zed@{rev[:7]}). Generated by karya/gpui-pre.py; do not edit.\n"
    )
    print(f"wrote {', '.join(mapping[c] for c in changed)} to {out}")


if __name__ == "__main__":
    main()
