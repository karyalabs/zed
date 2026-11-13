# Karya Labs GPUI

This fork carries the GPUI changes that gpui-av (and other Karya Labs apps)
need before Zed has them: HDR and 10-bit video on macOS, zero-copy video on
Linux and the web, frame pacing. The plan of record is section 4.3 of
`karyalabs/gpui-av` `docs/spec.md`.

## Rules

- Each patch branch starts at the Zed commit that a `gpui-pre` release was cut
  from (read `zed-rev` in that release's `Cargo.toml`; 0.3.12 is
  `d2a4fb6ded7c470be25d1541281da3882495b65b`).
- Patches are additive: new formats, new surface kinds, opt-in modes. Anything
  that builds on gpui-pre must build on ours.
- Every patch is also proposed to Zed. When Zed merges it, it is dropped here.
- Only `crates/gpui*` and this folder change. Zed's own CI only runs on `main`
  and tags, so patch branches start none of it.

## Using it from an app

`karya/gpui-pre.py` turns the branch's changes into crates with the gpui-pre
names and versions:

```sh
karya/gpui-pre.py --version 0.3.12 --out ../gpui-pre-karya
```

It writes only the crates the branch changes. An app patches exactly those:

```toml
[patch.crates-io]
gpui-pre-apple = { path = "../gpui-pre-karya/gpui-pre-apple" }
```

gpui-kit, gpuicn and anything else built on gpui-pre then compile against the
patched crates unchanged. Where the generated crates are published (a branch
of this repo, so apps can use a `git` source) is still to be decided.

## Moving to a new gpui-pre release

Start a branch at the new release's `zed-rev`, cherry-pick the patches that Zed
has not merged, and run the script. A patch fails to apply where gpui-pre's
own edits overlap ours; fix it on the branch.
