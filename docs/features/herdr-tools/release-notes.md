Yashik 0.3.0 adds managed Herdr installation through the new root `tools` section:

```yaml
version: 1
harnesses: {}
tools:
  herdr: {}
```

Herdr is installed once independently of harnesses. Use `version: latest` (the default) or pin an exact release. Yashik verifies the official binary checksum and version, records the installation, and checks it with `doctor`. If the Herdr site cannot transfer metadata, Yashik tries the fixed metadata URL in the official Herdr GitHub repository. Invalid metadata or exceeded download limits fail the installation. Installation supports Linux and macOS on x86_64 and aarch64; the Windows bridge uses the Linux recipe inside WSL.

`yashik init` now reads `./yashik-compose.yaml` when called without a path. Explicit paths remain supported, including through the Windows bridge.

Yashik does not start Herdr sessions or authenticate external services. Existing manifests remain compatible.
