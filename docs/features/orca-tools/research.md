# Research: managed Orca tool

User requested onOrca (https://www.onorca.dev) as another Yashik tool alongside Herdr, in the context of provisioning a Linux test server. Installation is authorized. A one-question clarification asks whether persistent `serve` is also wanted; installation can proceed independently, without starting a listener or creating a service.

## Official installation contract

Official repository: https://github.com/stablyai/orca . Official headless guide: https://github.com/stablyai/orca/blob/main/docs/reference/headless-linux-server.md . Install docs: https://www.onorca.dev/docs/install . Release API: https://api.github.com/repos/stablyai/orca/releases/latest .

On 2026-10-06 the latest stable release is v1.4.221. The release includes Linux x86_64 and ARM64 AppImages, Debian/RPM packages, macOS ZIP/DMG bundles and a Windows installer. GitHub API asset digests provide SHA-256; there is no separate checksum file. Current Linux AppImage sizes are 219617397 and 221580310 bytes; respective SHA-256 values are 7bf17b3619c2a4f2346b1894c652e2790b89277e501a78ce8911249a5e12f0f6 and 5f4638f190ba0c0a2f7081cbb07ebe697d57bd17f09a9483ad739a612a253cbf. Resolve latest/exact stable tags against fixed official endpoints and require the digest before execution.

Orca packages its CLI with Electron; no standalone native CLI release exists. A no-desktop server can use `serve`, but needs Electron shared libraries and Xvfb. The headless guide supports glibc >=2.31, including Ubuntu 20.04/22.04/24.04 and Debian stable. FUSE is optional through AppImage extraction. The Linux shell command is `orca-ide`, avoiding collision with the GNOME screen reader. The correct build probe is extracted `resources/bin/orca-ide --version`; directly probing the AppImage --version can return Electron's version. Persistent serve/service management is a separate behavior from installation.

The test server is Ubuntu 24.04 x86_64 with 3.6GB free disk and several Electron libraries already installed. Xvfb and an existing orca-ide command were not found. No server packages or services were changed during research.

## Architecture and acceptance

Research Consilium found the extension seams in src/schema.rs (closed tools map and strict custom Herdr visitor), src/effective.rs, src/validation.rs, src/main.rs and src/install/engine.rs. src/install/tools.rs is specifically Herdr-bound; retain its semantics, prefer a dedicated Orca reconciler and independent tool/orca journal record. Doctor currently dispatches all recorded tools to Herdr and hardcodes its label; explicit per-ID dispatch and unknown-ID rejection are required.

Existing State.tools defaults make old state compatible. Bundles need ownership of the whole installed tree and stable launcher, not just an AppImage hash; extend records only with backward-compatible defaults or a distinct compatible shape. Preserve unmanaged files, reject drift and unsafe paths, activate atomically after checksum/version validation, and never claim ownership on failure. Removal requires confirmation and verified managed paths; user profiles/sessions are outside ownership.

Acceptance covers strict schema/defaults/pins/disabled/coexistence; Orca-only check without state creation; correct platform and version resolution; checksum/probe failure without ownership; install/repeat unchanged; unmanaged collision and tampered bundles/launchers; removal pending and owned-only removal; offline read-only doctor; old Herdr state and all current Herdr tests. AppImage extraction and command execution must have bounded output/time/resource use. Real Linux acceptance does not establish macOS or Windows runtime acceptance.

The preexisting modification to .agents/skills/business-feature/SKILL.md is outside this feature and must be preserved.

## Actual official Linux package probe

Root downloaded the pinned v1.4.221 x86_64 AppImage and verified its official SHA-256 before extraction. `--appimage-extract` completed in a separate reference directory. Expanded bundle size is about 647MiB; bundled links are relative and internal. The bundled bash launcher selects the executable beside resources, exports ELECTRON_RUN_AS_NODE=1 and executes resources/app.asar.unpacked/out/cli/index.js. Running the extracted resources/bin/orca-ide --version returned exactly `1.4.221` with success in under a second, without a display or listener. This is Linux local package evidence, not server installation acceptance.

The existing util::hash_tree collects complete file contents and is unsuitable for large bundle memory use; use a streaming, bounded bundle ownership fingerprint. Bound downloaded bytes, extracted tree size/count and subprocess output/time.

## Consilium synthesis

Detailed independent findings are recorded in research-contract.md and research-architecture.md. Researchers agree on extracted AppImage plus orca-ide launcher, fixed official release metadata and digest verification, and installation independent of serve. Architecture researcher suggested x86_64-only first; official release contract confirms both x86_64 and ARM64 AppImages, so a Linux-only backend can select both while accurately limiting live runtime acceptance to x86_64. macOS and native Windows remain separate unsupported installation backends until their bundle lifecycle is designed and tested. No research disagreement blocks the Linux installation feature.
