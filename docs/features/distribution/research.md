# Distribution research synthesis

User request: a shell installer, Homebrew distribution in the owned
`AlexGladkov/homebrew-tap`, and WinGet (the user wrote “winglet”).

Both research threads confirm that no releases currently exist. The installer
therefore needs binary release infrastructure before its download commands can
work. The selected first distribution version is 0.2.1, with Linux x86_64 and
ARM64 static-musl archives plus native macOS Intel and Apple Silicon archives.
A checksummed binary formula avoids downloading Cargo dependencies during a
Homebrew build.

A release-specific shell script embeds its own default version. The convenience
latest URL fetches that script, which then downloads both archive and checksums
from the same pinned release. This avoids mixing different latest releases.
The script installs only Yashik; `yashik init` remains a separate action.

Native macOS CI must exercise the existing tests and adapt Linux-only fixture
paths. The release will remain draft until build and packaging checks pass.
The root operator owns release publication, tap updates, and real Ubuntu smoke.

WinGet cannot package the current Unix-only installer as a usable Windows
application. The user selected WinGet + WSL. A separately packaged Windows launcher will
bootstrap and invoke the Linux CLI inside an existing WSL distribution. The
WinGet manifest must distribute ZIP assets with a nested portable executable
named `yashik.exe` and command alias `yashik`. Hosted Windows CI can validate
the bridge protocol, but cannot substitute for a genuine WSL 2 machine test.
This adds Windows distribution without porting the core Unix installer.

Sources and unresolved platform details are in
[bootstrap research](research-bootstrap.md) and
[integration research](research-integrations.md).
