# Research: managed Herdr tools

Request: declare Herdr in yashik-compose.yaml, release a new Yashik version, and test on the existing SSH test server. The earlier setup interview selected codex, claude, omp, opencode and all VoltAgent agents, plus five MCP servers; their complete setup remains a separate follow-up after this feature.

## Consilium findings

Architecture researcher: root Manifest currently has only harnesses/mcp/skills/agents/rules and rejects unknown fields. EffectiveManifest and all installer resources are harness-bound. A root tools registry and an independent installation phase are required so tools are installed once and can work with zero harnesses. Tools need separate backwards-compatible state records, operation reports and read-only doctor verification; do not model Herdr as an MCP or harness.

Herdr integration researcher: official metadata https://herdr.dev/latest.json contains current version/assets/sha256 and a releases map for exact pins. Unix assets are bare binaries from https://github.com/herdrdev/herdr/releases/download/v<VERSION>/herdr-<linux|macos>-<x86_64|aarch64>. Older entries may lack SHA256: reject missing checksums rather than installing unchecked binaries. The direct upstream installer https://herdr.dev/install.sh installs latest only; Yashik should implement a controlled download/verify/activate recipe. Version probe is `herdr --version`, producing `herdr <version>` (https://github.com/herdrdev/herdr/blob/master/src/main.rs). Documentation: https://herdr.dev/docs/install/. Installation must not launch sessions, authenticate, start a server or restart existing Herdr processes. Checksums establish integrity relative to publisher metadata, not independent signature authenticity.

Release researcher: latest published Yashik is v0.2.1. Tag workflow .github/workflows/release.yml checks versions, fmt, clippy, Rust/Python tests and packages four Unix/two Windows targets before creating a draft release. Update root and Windows Cargo.toml/Cargo.lock and scripts/install.sh DEFAULT_VERSION together. Publish after CI asset checks. Homebrew formula and WinGet manifests are generated workflow artifacts with separate distribution procedures. Read docs/features/distribution/{plan,run,validation}. Linux test does not prove macOS or Windows ARM64 runtime support.

## Existing work and environment

Working tree contains prior default-manifest feature edits in README.md, docs/design.md, src/main.rs, tests/cli.rs and tools/windows-launcher/src/lib.rs plus docs/features/default-manifest. Preserve them and test together before release; do not bundle the unrelated skill profile edit. Initial tracked diff saved at /tmp/yashik-before-herdr.patch.

SSH test-server-root points to 89.111.154.122 and is reachable. uname reports Linux x86_64. User's directory is /root/opt/yashik-setup, not /opt/yashik-setup. Neither yashik nor herdr is currently on default SSH PATH.

## Recommended decisions and acceptance risks

Use closed root `tools: {herdr: {enabled: true, version: latest}}`; defaults enabled/latest, optional exact version. Unknown tools/fields and invalid versions fail validation. Preserve schema version 1 compatibility for manifests without tools. Install under Yashik-owned versioned data directories and activate ~/.local/bin/herdr without silently overwriting unmanaged files. State records must permit old state.json to load. Repeat exact installs should verify and report unchanged; changed/tampered files must be detected before execution. Removed/disabled tools should follow existing CLI retention/pending-removal semantics rather than delete sessions or personal data. Test malformed publisher metadata, checksum/version failure, unsupported platforms, conflicts, retry, old-state deserialization and doctor tampering.

New public manifest capability warrants Yashik 0.3.0. No unresolved user preference is required for this bounded implementation; publishing was explicitly requested.

## Live metadata transport finding and bounded fallback

On 2026-10-06, the first packaged `0.3.0` run on `test-server-root` reached the Herdr metadata request but failed before installation. A direct server GET of `https://herdr.dev/latest.json` timed out after 20 seconds with 20,508 of the declared 177,574 bytes; HEAD returned 200. On the same server, `https://raw.githubusercontent.com/herdrdev/herdr/master/distribution/latest.json` returned the full 177,574 bytes. Root downloaded both complete responses locally and found the same SHA-256, `daad8d6fe4524fbb78caa287b7936a6a7400262dbe872c8bc3532c3b371bf05e`. This proves equality for those observations, not permanent equivalence of the two mutable endpoints.

Use the fixed raw URL in the official `herdrdev/herdr` repository only as a metadata transport fallback when the primary GET fails at the network/HTTP transfer layer, including timeout or incomplete transfer. Both requests retain HTTPS-only redirects, the existing metadata size cap and a short time limit. A complete response that is malformed JSON, has an invalid or missing version/asset/checksum, or exceeds the size cap must fail closed without trying the other source. Local process, capture and resource failures must also fail without fallback. Keep the existing strict GitHub release asset URL, SHA-256 and executable version checks. The installed state can continue to record the canonical `https://herdr.dev/latest.json` as the metadata identity; choosing a transport endpoint does not change ownership or integrity rules.

The fallback is a narrow response to the observed server transport failure, not a user-configurable endpoint or a way to accept inconsistent publisher data. Add fixture coverage for primary transport failure followed by valid mirror metadata, both endpoints failing, and validly transferred but semantically invalid primary metadata. Repeat the real server installation and state inspection before publishing `v0.3.0`.
