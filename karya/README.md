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

The patched crates are published as the branch `crates/gpui-pre-<version>`
of this repo, with the gpui-pre names and versions. An app or workspace
patches the crates that branch holds (today `gpui-pre-apple` and
`gpui-pre-macos`):

```toml
[patch.crates-io]
gpui-pre-apple = { git = "https://github.com/karyalabs/zed", branch = "crates/gpui-pre-0.3.12" }
gpui-pre-macos = { git = "https://github.com/karyalabs/zed", branch = "crates/gpui-pre-0.3.12" }
```

gpui-kit, gpuicn and anything else built on gpui-pre then compile against
the patched crates unchanged. `[patch]` applies only to the workspace that
declares it, so a crate published to crates.io still builds on stock
gpui-pre for its users.

After merging a patch into `gpui-pre/<version>`, publish the crates again:

```sh
karya/publish.sh 0.3.12
```

To try a branch without publishing, `karya/gpui-pre.py --version 0.3.12
--out <folder>` writes the crates locally (gpui-av's `scripts/karya-gpui`
does this for any ref).

## Moving to a new gpui-pre release

Start a branch at the new release's `zed-rev`, cherry-pick the patches that Zed
has not merged, and run the script. A patch fails to apply where gpui-pre's
own edits overlap ours; fix it on the branch.
