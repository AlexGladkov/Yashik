Adds managed Orca installation through `tools.orca`, alongside Herdr.

```yaml
version: 1
harnesses: {}
tools:
  herdr:
    version: 0.9.3
  orca:
    version: 1.4.221
```

Orca installation supports Linux x86_64 and ARM64. Yashik verifies the official release SHA-256, installs an extracted AppImage bundle without requiring FUSE, and provides `orca-ide` in its bin directory. `latest` resolves the current stable release; exact versions can be pinned.

Repeat installation and read-only `doctor` verify the managed launcher and package. Disabling or removing Orca requires confirmation before deleting verified managed files; Orca profiles, sessions and pairing data remain outside Yashik's ownership.

Installation does not start `orca serve` or create a persistent service. Installation requires an available curl and Electron shared libraries; an Orca-only manifest does not install OS packages. Running serve without a desktop additionally requires Xvfb, as documented in the [official headless guide](https://github.com/stablyai/orca/blob/main/docs/reference/headless-linux-server.md). This release does not add a macOS or native Windows Orca installation backend.
