# Distribution business-feature run

Request: install Yashik through a POSIX shell script, the user's
AlexGladkov/homebrew-tap, and WinGet. The user explicitly selected WinGet + WSL.
Publication within those repositories and a WinGet community submission are
part of the requested distribution work.

The local business-feature profile was applied:

| Stage | Assignment | Evidence |
| --- | --- | --- |
| Research | Consilium: Terra 5.6, xhigh | research-bootstrap.md, research-integrations.md, research.md |
| Plan | Strategist: Sol 6.1, xhigh | plan.md |
| Execute | Engineers: Luna 6, max | bootstrap and release workers; release worker additionally owns the Windows bridge |
| Review | Independent Engineer: Sol 6.1, xhigh | review.md |
| Validate | Tester: Luna 6, max; root performs authorized publication/network checks | validation.md |

The requested Terra review worker could not be allocated because the tool
returned `agent thread limit reached`. The user explicitly approved Sol review
for this feature. This is a per-run exception; the stored profile was not changed.
The reviewer did not author product code. The already available Luna release
worker was reused for the Windows work rather than silently changing Execute's
model. Root handles integration, targeted CI-driven fixture fixes, documentation,
GitHub publication, tap updates, and the real Ubuntu acceptance checks.

## Integration findings

- Rust 1.85 could not compile the locked jsonc-parser dependency's let chains.
  The actual minimum is Rust 1.88, pinned in release CI and documented.
- Rust 1.88 Clippy exposed one uninlined test format argument; fixed.
- Native Darwin compilation exposed variadic integer promotion and dev_t type
  differences. The four narrow casts were fixed and independently reviewed.
- Native Darwin tests exposed temporary directory aliases through /var.
  Only fixture roots were canonicalized; production no-follow checks remain.
- Independent review reproduced an HTTPS-to-HTTP redirect through Wget despite
  `--https-only`. The optional fallback was removed; curl-only transport restricts
  both initial and redirected protocols to HTTPS.
- A byte-identical non-executable existing target now receives atomic permission
  repair. An already executable identical target remains untouched.

Pre-tag workflow dispatches are build-only. A tag creates a draft release;
publication follows the relevant review/build/package gates. Exact final results
and remaining platform limitations belong in validation.md.

Windows integration additionally required independent fixes for distro-name
parsing, Windows-only executor fixtures, quoted all-numeric SHA256 values,
WinGet schema/header compatibility and unsupported portable Scope metadata,
exact command alias plus persistent PATH verification, and a real currently
published Ubuntu Base fixture. Downloaded PE artifacts confirmed a dependency
on VCRUNTIME140.dll before the static CRT fix. Both final architectures were
then inspected independently and import only Windows system DLLs.

WinGet local-manifest inventory is removed by exact display name in isolated CI;
the catalog ID is not resolvable before catalog publication. Noninteractive
source agreement acceptance is confined to the hosted test runner. A rare
Darwin CLI fixture ENOENT exposed timestamp-only directory naming; an atomic
counter now guarantees uniqueness within the test process. The observed error
is real; a timestamp collision is an inferred cause, not a separately reproduced
clock failure.
