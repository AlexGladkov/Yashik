# Orca upstream package and CLI contract

Verified against the official onOrca site and `stablyai/orca` source/release
metadata on 2026-10-06. This note records upstream behavior relevant to the
Yashik Orca installer; it does not describe Yashik implementation.

## Release and platform assets

The latest stable release is `v1.4.221`. Linux releases provide full-app
AppImages, `.deb` packages, and `.rpm` packages for x86_64 and ARM64/aarch64.
The x86_64 and ARM64 AppImages are 219,617,397 and 221,580,310 bytes. Their
GitHub Release API SHA-256 digests are:

| Asset | SHA-256 |
| --- | --- |
| `orca-linux.AppImage` | `7bf17b3619c2a4f2346b1894c652e2790b89277e501a78ce8911249a5e12f0f6` |
| `orca-linux-arm64.AppImage` | `5f4638f190ba0c0a2f7081cbb07ebe697d57bd17f09a9483ad739a612a253cbf` |

GitHub's release API exposes each asset's `digest` field; there is no separate
`checksums.txt` release asset. A resolver can query the official latest-release
API, select the architecture-specific asset, and verify its API digest, or use
a pinned tag and expected digest. The release's `latest-linux.yml` is updater
metadata with platform/version and SHA-512 fields; it is not a standalone CLI
manifest.

There is no standalone CLI artifact in the current release. The Orca CLI is
packaged inside the full Electron application. The repository has an open
proposal for a separate Node-only CLI bundle, which describes the current
Electron packaging dependency; that proposed bundle is not present in the
release assets.

## Linux headless and install-only behavior

The official headless guide supports Ubuntu 20.04, 22.04, and 24.04 and current
Debian stable, with glibc 2.31 or newer. On a host without a display, `serve`
needs Xvfb and the shared libraries Electron expects. D-Bus is not required.
AppImage FUSE mounting is optional: `--appimage-extract` works without FUSE;
the resulting `squashfs-root` is initially mode `0700`, so another service
account needs read/traverse access before use. Running as a dedicated
unprivileged service user preserves Chromium's sandbox. Headless `serve` runs
in the foreground and does not self-update.

The Linux CLI command is `orca-ide` to avoid colliding with the GNOME Orca
screen reader at `/usr/bin/orca`. Deb/RPM installation places it on `PATH`;
AppImage registration can add a per-user CLI link. The AppImage itself can be
run directly for `serve`; starting that command is a separate choice from
installing or checking the CLI.

The extracted Linux CLI wrapper is
`squashfs-root/resources/bin/orca-ide`. It resolves symlinks, derives its
resource/app directories relative to its own location, chooses the adjacent
Electron executable (`orca-ide`, `orca`, then `Orca`), and runs
`resources/app.asar.unpacked/out/cli/index.js` with `ELECTRON_RUN_AS_NODE=1`.
The wrapper has no `ORCA_APPIMAGE_PATH` override. Its launch resolution comes
from its own installed/extracted path.

The CLI handles exactly `--version` and `-v` before loading its runtime client.
That branch reads the generated
`resources/app.asar.unpacked/out/package.json` version and writes the version
plus a newline. The Electron executable still has to start as Node; the probe
does not start the GUI, connect to a running Orca runtime, or need network
access. It still depends on the system shared libraries needed to load that
Electron executable. The official guide says not to use the top-level
`orca-linux.AppImage --version`, because those flags may report Electron's own
version. For the extracted launcher, use:

```sh
squashfs-root/resources/bin/orca-ide --version
```

For `v1.4.221`, the successful output is `1.4.221` followed by a newline.
Yashik's test run on the extracted official x86_64 bundle returned exactly
that output without opening a display or starting a listener.

## macOS packaging comparison

The release also contains full macOS DMG/ZIP application bundles for Apple
Silicon and Intel. The source wrapper is
`resources/darwin/bin/orca`; it resolves the containing `.app`, selects
`Contents/MacOS/Orca`, and invokes
`Contents/Resources/app.asar.unpacked/out/cli/index.js` with
`ELECTRON_RUN_AS_NODE=1`. It shares the CLI/version implementation, but uses a
macOS-specific wrapper and app layout. The official headless server guide and
the verified Linux extraction path do not establish macOS server-install
support.

## Primary sources

- [Official install docs](https://www.onorca.dev/docs/install)
- [Official headless Linux server guide](https://github.com/stablyai/orca/blob/main/docs/reference/headless-linux-server.md)
- [Official releases](https://github.com/stablyai/orca/releases/latest)
- [Official latest-release API](https://api.github.com/repos/stablyai/orca/releases/latest)
- [Linux CLI wrapper](https://github.com/stablyai/orca/blob/main/resources/linux/bin/orca-ide)
- [CLI version dispatch](https://github.com/stablyai/orca/blob/main/src/cli/index.ts)
- [CLI version reader](https://github.com/stablyai/orca/blob/main/src/cli/cli-version.ts)
- [Packaging layout and version stamping](https://github.com/stablyai/orca/blob/main/config/electron-builder.config.cjs)
- [macOS CLI wrapper](https://github.com/stablyai/orca/blob/main/resources/darwin/bin/orca)
- [Open proposal for standalone CLI bundle](https://github.com/stablyai/orca/issues/23525)
