use std::ffi::{OsStr, OsString};
use std::io;
use std::path::Path;
use std::process::{Command, Output};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

const WSL_LIST: &[&str] = &["--list", "--quiet"];
const WSL_VERBOSE_LIST: &[&str] = &["--list", "--verbose"];
const INITIALIZE_PROBE: &str =
    r#"test -n "$HOME" && test -d "$HOME" && command -v sh >/dev/null 2>&1"#;
const VERSION_PROBE: &str = r#"exec "$HOME/.local/bin/yashik" --version"#;

pub fn help_text() -> &'static str {
    "Usage:\n  yashik [--distro NAME] setup [--force]\n  yashik [--distro NAME] init MANIFEST\n  yashik [--distro NAME] check MANIFEST\n  yashik [--distro NAME] doctor\n  yashik --version\n  yashik --help\n\nYashik runs its Linux installer inside an already installed WSL distribution.\nIt does not install WSL or change Windows features.\n"
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Help,
    Version,
    Exit(i32),
}

#[derive(Debug, PartialEq, Eq)]
enum Request {
    Help,
    Version,
    Setup {
        distro: Option<OsString>,
        force: bool,
    },
    Init {
        distro: Option<OsString>,
        manifest: OsString,
    },
    Check {
        distro: Option<OsString>,
        manifest: OsString,
    },
    Doctor {
        distro: Option<OsString>,
    },
}

#[derive(Clone, Debug)]
pub struct Captured {
    pub success: bool,
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl Captured {
    pub fn from_output(output: Output) -> Self {
        Self {
            success: output.status.success(),
            code: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
        }
    }
}

pub trait WslExecutor {
    fn capture(&mut self, arguments: &[OsString]) -> io::Result<Captured>;
    fn forward(&mut self, arguments: &[OsString]) -> io::Result<i32>;
}

pub struct SystemWsl;

impl WslExecutor for SystemWsl {
    fn capture(&mut self, arguments: &[OsString]) -> io::Result<Captured> {
        Command::new("wsl.exe")
            .args(arguments)
            .output()
            .map(Captured::from_output)
    }

    fn forward(&mut self, arguments: &[OsString]) -> io::Result<i32> {
        let status = Command::new("wsl.exe").args(arguments).status()?;
        Ok(status.code().unwrap_or(1))
    }
}

pub fn run(arguments: &[OsString], wsl: &mut impl WslExecutor) -> Result<Outcome, String> {
    let request = parse_request(arguments)?;
    match request {
        Request::Help => Ok(Outcome::Help),
        Request::Version => Ok(Outcome::Version),
        Request::Setup { distro, force } => {
            let distro = selected_distro(wsl, distro.as_deref())?;
            check_initialized(wsl, &distro)?;
            let mut forward = command_args(&[
                "--distribution",
                "--exec",
                "sh",
                "-c",
                &setup_script(),
                "yashik-setup",
            ]);
            forward.insert(1, OsString::from(&distro));
            if force {
                forward.push(OsString::from("--force"));
            }
            Ok(Outcome::Exit(run_forward(wsl, &forward)?))
        }
        Request::Init { distro, manifest } => {
            run_yashik_command(wsl, distro.as_deref(), "init", Some(manifest))
        }
        Request::Check { distro, manifest } => {
            run_yashik_command(wsl, distro.as_deref(), "check", Some(manifest))
        }
        Request::Doctor { distro } => run_yashik_command(wsl, distro.as_deref(), "doctor", None),
    }
}

fn run_yashik_command(
    wsl: &mut impl WslExecutor,
    requested_distro: Option<&OsStr>,
    command: &str,
    manifest: Option<OsString>,
) -> Result<Outcome, String> {
    let distro = selected_distro(wsl, requested_distro)?;
    check_initialized(wsl, &distro)?;
    check_yashik_version(wsl, &distro)?;

    let mut forwarded = vec![OsString::from(command)];
    if let Some(manifest) = manifest {
        forwarded.push(OsString::from(manifest_to_linux_path(
            wsl, &distro, &manifest,
        )?));
    }

    let mut arguments = command_args(&[
        "--distribution",
        "--exec",
        "sh",
        "-c",
        r#"exec "$HOME/.local/bin/yashik" "$@""#,
        "yashik-bridge",
    ]);
    arguments.insert(1, OsString::from(&distro));
    arguments.extend(forwarded);
    Ok(Outcome::Exit(run_forward(wsl, &arguments)?))
}

fn run_forward(wsl: &mut impl WslExecutor, arguments: &[OsString]) -> Result<i32, String> {
    wsl.forward(arguments).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            missing_wsl_error()
        } else {
            format!("could not start wsl.exe: {error}")
        }
    })
}

fn parse_request(arguments: &[OsString]) -> Result<Request, String> {
    let mut index = 0;
    let mut distro = None;
    while index < arguments.len() {
        let value = arguments[index].to_string_lossy();
        match value.as_ref() {
            "--distro" => {
                let name = arguments.get(index + 1).ok_or("--distro requires a name")?;
                validate_distro_name(name)?;
                if distro.replace(name.clone()).is_some() {
                    return Err("--distro may only be specified once".to_owned());
                }
                index += 2;
            }
            "--help" | "-h" => {
                if index + 1 != arguments.len() {
                    return Err(format!(
                        "unexpected argument after {}\n\n{}",
                        value,
                        help_text()
                    ));
                }
                return Ok(Request::Help);
            }
            "--version" | "-V" => {
                if index + 1 != arguments.len() {
                    return Err(format!(
                        "unexpected argument after {}\n\n{}",
                        value,
                        help_text()
                    ));
                }
                return Ok(Request::Version);
            }
            _ => break,
        }
    }

    let command = arguments
        .get(index)
        .ok_or_else(|| format!("missing command\n\n{}", help_text()))?
        .to_string_lossy()
        .into_owned();
    let mut operands = Vec::new();
    index += 1;
    while index < arguments.len() {
        let value = arguments[index].to_string_lossy();
        if value == "--distro" {
            let name = arguments.get(index + 1).ok_or("--distro requires a name")?;
            validate_distro_name(name)?;
            if distro.replace(name.clone()).is_some() {
                return Err("--distro may only be specified once".to_owned());
            }
            index += 2;
        } else {
            operands.push(arguments[index].clone());
            index += 1;
        }
    }

    match command.as_str() {
        "setup" => match operands.as_slice() {
            [] => Ok(Request::Setup {
                distro,
                force: false,
            }),
            [force] if force == "--force" => Ok(Request::Setup {
                distro,
                force: true,
            }),
            [unknown] => Err(format!(
                "unknown setup argument: {}",
                unknown.to_string_lossy()
            )),
            _ => Err("setup accepts only the optional --force flag".to_owned()),
        },
        "init" | "check" => {
            let [manifest] = operands.as_slice() else {
                return Err(format!("{command} requires exactly one manifest path"));
            };
            if manifest.to_string_lossy().starts_with('-') {
                return Err(format!(
                    "unknown {command} flag: {}",
                    manifest.to_string_lossy()
                ));
            }
            if command == "init" {
                Ok(Request::Init {
                    distro,
                    manifest: manifest.clone(),
                })
            } else {
                Ok(Request::Check {
                    distro,
                    manifest: manifest.clone(),
                })
            }
        }
        "doctor" => {
            if !operands.is_empty() {
                return Err("doctor accepts no command arguments".to_owned());
            }
            Ok(Request::Doctor { distro })
        }
        _ => Err(format!("unknown command: {command}\n\n{}", help_text())),
    }
}

fn validate_distro_name(name: &OsStr) -> Result<(), String> {
    let name = name
        .to_str()
        .ok_or_else(|| "WSL distro name must be valid Unicode".to_owned())?;
    if name.trim().is_empty() {
        return Err("WSL distro name must not be empty".to_owned());
    }
    if name.starts_with('-') {
        return Err("WSL distro name must not start with '-'".to_owned());
    }
    if name.chars().any(char::is_control) {
        return Err("WSL distro name must not contain control characters".to_owned());
    }
    Ok(())
}

fn selected_distro(
    wsl: &mut impl WslExecutor,
    requested: Option<&OsStr>,
) -> Result<OsString, String> {
    let listed = checked_capture(
        wsl,
        command_args(WSL_LIST),
        "could not list WSL distributions",
    )?;
    let names = parse_distro_names(&listed.stdout);
    if names.is_empty() {
        return Err(no_distro_error());
    }

    if let Some(requested) = requested {
        let requested_text = requested
            .to_str()
            .ok_or_else(|| "WSL distro name must be valid Unicode".to_owned())?;
        return names
            .iter()
            .find(|name| name == &requested_text)
            .map(OsString::from)
            .ok_or_else(|| {
                format!(
                    "WSL distribution {requested_text:?} is not installed. Install and initialize a WSL distribution, then retry."
                )
            });
    }

    let verbose = checked_capture(
        wsl,
        command_args(WSL_VERBOSE_LIST),
        "could not determine the default WSL distribution",
    )?;
    let default_name = parse_default_distro(&verbose.stdout, &names)
        .ok_or_else(|| "no default WSL distribution is configured; set one with `wsl --set-default NAME` or pass `--distro NAME`".to_owned())?;
    names
        .iter()
        .find(|name| name.as_str() == default_name.as_str())
        .map(OsString::from)
        .ok_or_else(|| {
            "the configured default WSL distribution is not registered; select an installed distro with `--distro NAME`".to_owned()
        })
}

fn check_initialized(wsl: &mut impl WslExecutor, distro: &OsStr) -> Result<(), String> {
    let arguments = distribution_args(distro, &["--exec", "sh", "-c", INITIALIZE_PROBE]);
    let result = wsl
        .capture(&arguments)
        .map_err(|error| map_wsl_error(error, "could not start the selected WSL distribution"))?;
    if result.success {
        Ok(())
    } else {
        Err(format!(
            "WSL distribution {:?} is installed but not initialized or could not start. Open it manually with `wsl --distribution {}` and finish its first-run setup, then retry.",
            distro.to_string_lossy(),
            distro.to_string_lossy()
        ))
    }
}

fn check_yashik_version(wsl: &mut impl WslExecutor, distro: &OsStr) -> Result<(), String> {
    let result = wsl
        .capture(&distribution_args(
            distro,
            &["--exec", "sh", "-c", VERSION_PROBE],
        ))
        .map_err(|error| map_wsl_error(error, "could not check Yashik inside WSL"))?;
    let reported = String::from_utf8_lossy(&result.stdout);
    if result.success && reported.trim_end_matches(['\r', '\n']) == format!("yashik {VERSION}") {
        Ok(())
    } else {
        Err(format!(
            "Yashik {VERSION} is not installed inside WSL (reported {}). Run `yashik --distro {:?} setup` first.",
            reported.trim(),
            distro.to_string_lossy()
        ))
    }
}

fn manifest_to_linux_path(
    wsl: &mut impl WslExecutor,
    distro: &OsStr,
    manifest: &OsStr,
) -> Result<String, String> {
    let text = manifest
        .to_str()
        .ok_or_else(|| "manifest path must be valid Unicode".to_owned())?;
    if text.starts_with('/') {
        return Ok(text.to_owned());
    }

    let input = Path::new(manifest);
    let absolute = if input.is_absolute() {
        input.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| format!("could not resolve the current Windows directory: {error}"))?
            .join(input)
    };
    let canonical = absolute
        .canonicalize()
        .map_err(|error| format!("could not resolve Windows manifest path: {error}"))?;
    let canonical_text = canonical.to_string_lossy();
    let windows_path = canonical_windows_path(&canonical_text);

    let result = wsl
        .capture(
            &distribution_args(distro, &["--exec", "wslpath", "-a", "-u"])
                .into_iter()
                .chain([OsString::from(windows_path)])
                .collect::<Vec<_>>(),
        )
        .map_err(|error| {
            map_wsl_error(error, "could not convert the manifest path with wslpath")
        })?;
    if !result.success {
        return Err(format!(
            "wslpath could not convert the Windows manifest path: {}",
            output_text(&result.stderr)
        ));
    }
    let converted = String::from_utf8_lossy(&result.stdout);
    let converted = converted.trim_end_matches(['\r', '\n']);
    if converted.is_empty() || !converted.starts_with('/') {
        return Err("wslpath returned an invalid absolute Linux manifest path".to_owned());
    }
    Ok(converted.to_owned())
}

fn canonical_windows_path(path: &str) -> String {
    if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        // wslpath expects ordinary UNC paths, not the verbatim namespace form.
        return format!(r"\\{unc}");
    }
    path.strip_prefix(r"\\?\").unwrap_or(path).to_owned()
}

fn setup_script() -> String {
    format!(
        r#"set -eu
version='{VERSION}'
base='https://github.com/AlexGladkov/Yashik/releases/download/v{VERSION}'
tmp=$(mktemp -d) || {{ echo 'setup: could not create a temporary directory' >&2; exit 1; }}
trap 'rm -rf "$tmp"' EXIT
trap 'exit 1' HUP INT TERM
download() {{
    url=$1
    destination=$2
    if ! command -v curl >/dev/null 2>&1; then
        echo 'setup: curl is required inside WSL' >&2
        exit 1
    fi
    curl --proto '=https' --proto-redir '=https' --tlsv1.2 \
        --fail --silent --show-error --location "$url" --output "$destination"
}}
download "$base/install.sh" "$tmp/install.sh"
download "$base/SHA256SUMS" "$tmp/SHA256SUMS"
expected=$(awk '
    {{
        filename = $NF
        sub(/^\*/, "", filename)
        if (filename == "install.sh") {{
            matches++
            if (NF != 2 || length($1) != 64 || $1 ~ /[^[:xdigit:]]/) invalid = 1
            else checksum = tolower($1)
        }}
    }}
    END {{ if (matches != 1 || invalid) exit 1; print checksum }}
' "$tmp/SHA256SUMS") || {{ echo 'setup: SHA256SUMS must contain exactly one valid install.sh entry' >&2; exit 1; }}
if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$tmp/install.sh" | awk '{{print $1}}')
elif command -v shasum >/dev/null 2>&1; then
    actual=$(shasum -a 256 "$tmp/install.sh" | awk '{{print $1}}')
else
    echo 'setup: sha256sum or shasum is required inside WSL' >&2
    exit 1
fi
[ "$actual" = "$expected" ] || {{ echo 'setup: install.sh SHA-256 mismatch' >&2; exit 1; }}
if [ "$#" -gt 1 ]; then echo 'setup: only --force may be forwarded' >&2; exit 2; fi
if [ "$#" -eq 1 ] && [ "$1" != '--force' ]; then echo 'setup: only --force may be forwarded' >&2; exit 2; fi
sh "$tmp/install.sh" --version "$version" "$@"
"#
    )
}

fn checked_capture(
    wsl: &mut impl WslExecutor,
    arguments: Vec<OsString>,
    context: &str,
) -> Result<Captured, String> {
    let output = wsl
        .capture(&arguments)
        .map_err(|error| map_wsl_error(error, context))?;
    if output.success {
        Ok(output)
    } else {
        let details = output_text(&output.stderr);
        if details.is_empty() {
            Err(context.to_owned())
        } else {
            Err(format!("{context}: {details}"))
        }
    }
}

fn map_wsl_error(error: io::Error, context: &str) -> String {
    if error.kind() == io::ErrorKind::NotFound {
        missing_wsl_error()
    } else {
        format!("{context}: {error}")
    }
}

fn missing_wsl_error() -> String {
    "wsl.exe was not found. Install and initialize WSL manually, then install a Linux distribution; Yashik does not enable Windows features or install WSL.".to_owned()
}

fn no_distro_error() -> String {
    "no WSL distribution is installed. Install a Linux distribution and initialize it manually, then retry; Yashik does not install WSL or enable Windows features.".to_owned()
}

fn parse_distro_names(bytes: &[u8]) -> Vec<String> {
    decode_wsl_text(bytes)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| line.strip_prefix('*').unwrap_or(line).trim().to_owned())
        .filter(|line| !line.is_empty() && !line.eq_ignore_ascii_case("NAME"))
        .collect()
}

fn parse_default_distro(bytes: &[u8], registered_names: &[String]) -> Option<String> {
    for line in decode_wsl_text(bytes).lines() {
        let line = line.trim_start();
        let Some(columns) = line.strip_prefix('*') else {
            continue;
        };
        let columns = columns.trim_start();
        if let Some(name) = registered_names
            .iter()
            .filter(|name| {
                let Some(suffix) = columns.strip_prefix(name.as_str()) else {
                    return false;
                };
                if !suffix.chars().next().is_some_and(char::is_whitespace) {
                    return false;
                }
                let suffix_columns = suffix.split_whitespace().collect::<Vec<_>>();
                matches!(suffix_columns.last(), Some(&"1" | &"2")) && suffix_columns.len() >= 2
            })
            .max_by_key(|name| name.len())
        {
            return Some(name.clone());
        }
    }
    None
}

fn decode_wsl_text(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xff, 0xfe]) {
        return decode_utf16_le(rest);
    }
    let (pairs, _) = bytes.as_chunks::<2>();
    if pairs.iter().any(|pair| pair[1] == 0) {
        return decode_utf16_le(bytes);
    }
    String::from_utf8_lossy(bytes).into_owned()
}

fn decode_utf16_le(bytes: &[u8]) -> String {
    let (pairs, _) = bytes.as_chunks::<2>();
    let units = pairs.iter().map(|pair| u16::from_le_bytes(*pair));
    String::from_utf16_lossy(&units.collect::<Vec<_>>())
        .trim_end_matches('\0')
        .to_owned()
}

fn output_text(bytes: &[u8]) -> String {
    decode_wsl_text(bytes).trim().to_owned()
}

fn distribution_args(distro: &OsStr, command: &[&str]) -> Vec<OsString> {
    let mut arguments = command_args(&["--distribution"]);
    arguments.push(distro.to_os_string());
    arguments.extend(command_args(command));
    arguments
}

fn command_args(arguments: &[&str]) -> Vec<OsString> {
    arguments.iter().map(OsString::from).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    #[cfg(any(unix, windows))]
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::{symlink, PermissionsExt};
    #[cfg(any(unix, windows))]
    use std::path::PathBuf;
    #[cfg(unix)]
    use std::process::Command;
    #[cfg(any(unix, windows))]
    use std::time::{SystemTime, UNIX_EPOCH};

    #[derive(Default)]
    struct MockWsl {
        captures: VecDeque<Captured>,
        capture_args: Vec<Vec<OsString>>,
        forward_args: Vec<Vec<OsString>>,
        forward_code: i32,
    }

    impl MockWsl {
        fn queue(&mut self, output: Captured) {
            self.captures.push_back(output);
        }

        fn names(&mut self, names: &[&str]) {
            self.queue(successful(utf16(&names.join("\n"))));
        }

        fn initialized(&mut self) {
            self.queue(successful(b"ready".to_vec()));
        }

        fn version_matches(&mut self) {
            self.queue(successful(format!("yashik {VERSION}\n").into_bytes()));
        }

        fn explicit_distro_prefix(&mut self, name: &str) {
            self.names(&[name]);
            self.initialized();
        }

        fn default_distro_prefix(&mut self, name: &str) {
            self.names(&[name]);
            self.queue(successful(utf16(&format!(
                "  NAME                 STATE           VERSION\n* {name}             Stopped         1\n"
            ))));
            self.initialized();
        }
    }

    impl WslExecutor for MockWsl {
        fn capture(&mut self, arguments: &[OsString]) -> io::Result<Captured> {
            self.capture_args.push(arguments.to_vec());
            self.captures
                .pop_front()
                .ok_or_else(|| io::Error::other("unexpected mock capture"))
        }

        fn forward(&mut self, arguments: &[OsString]) -> io::Result<i32> {
            self.forward_args.push(arguments.to_vec());
            Ok(self.forward_code)
        }
    }

    fn successful(stdout: Vec<u8>) -> Captured {
        Captured {
            success: true,
            code: Some(0),
            stdout,
            stderr: Vec::new(),
        }
    }

    fn utf16(text: &str) -> Vec<u8> {
        let mut bytes = vec![0xff, 0xfe];
        for unit in text.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes
    }

    fn strings(arguments: &[OsString]) -> Vec<String> {
        arguments
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect()
    }

    #[cfg(any(unix, windows))]
    fn unique_temp_path() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("yashik-windows-{}-{nonce}", std::process::id()))
    }

    #[test]
    fn version_and_help_are_local_and_do_not_call_wsl() {
        let mut wsl = MockWsl::default();
        assert_eq!(
            run(&[OsString::from("--version")], &mut wsl),
            Ok(Outcome::Version)
        );
        assert_eq!(
            run(&[OsString::from("--help")], &mut wsl),
            Ok(Outcome::Help)
        );
        assert!(wsl.capture_args.is_empty());
        assert!(wsl.forward_args.is_empty());
    }

    #[test]
    fn distro_names_allow_spaces_but_reject_empty_flags_and_controls() {
        assert!(validate_distro_name(OsStr::new("Ubuntu Dev")).is_ok());
        assert!(validate_distro_name(OsStr::new(" ")).is_err());
        assert!(validate_distro_name(OsStr::new("-Ubuntu")).is_err());
        assert!(validate_distro_name(OsStr::new("Ubuntu\nDev")).is_err());
    }

    #[test]
    fn parses_utf16_wsl_names_and_default_with_embedded_spaces() {
        let names = parse_distro_names(&utf16("Ubuntu Dev\nDebian\n"));
        assert_eq!(names, vec!["Ubuntu Dev", "Debian"]);
        let verbose = utf16("  NAME                 STATE           VERSION\r\n* Ubuntu Dev           Stopped         2\r\n  Debian               Running         2\r\n");
        assert_eq!(
            parse_default_distro(&verbose, &names).as_deref(),
            Some("Ubuntu Dev")
        );
    }

    #[test]
    fn parses_default_distribution_name_containing_a_state_word() {
        let names = vec!["Ubuntu".to_owned(), "Ubuntu Stopped Dev".to_owned()];
        let verbose = b"NAME                 STATE           VERSION\n* Ubuntu Stopped Dev    Stopped         2\n";
        assert_eq!(
            parse_default_distro(verbose, &names).as_deref(),
            Some("Ubuntu Stopped Dev")
        );
    }

    #[test]
    fn parses_default_distribution_with_a_localized_state_column() {
        let names = vec!["Ubuntu Stopped Dev".to_owned()];
        let verbose = "NAME                 STATE           VERSION\n* Ubuntu Stopped Dev    Остановлен      2\n";
        assert_eq!(
            parse_default_distro(verbose.as_bytes(), &names).as_deref(),
            Some("Ubuntu Stopped Dev")
        );
    }

    #[test]
    fn setup_uses_selected_distro_and_only_forwards_force_when_requested() {
        let mut wsl = MockWsl::default();
        wsl.explicit_distro_prefix("Ubuntu Dev");
        wsl.forward_code = 0;
        let args = ["--distro", "Ubuntu Dev", "setup", "--force"].map(OsString::from);
        assert_eq!(run(&args, &mut wsl), Ok(Outcome::Exit(0)));
        assert_eq!(
            strings(&wsl.forward_args[0])[0..4],
            ["--distribution", "Ubuntu Dev", "--exec", "sh"]
        );
        let forwarded = strings(&wsl.forward_args[0]);
        assert_eq!(forwarded.last().map(String::as_str), Some("--force"));
        assert!(forwarded[5].contains("releases/download/v0.2.1'"));
        assert!(forwarded[5].contains("download \"$base/install.sh\""));
        assert!(forwarded[5].contains("SHA256SUMS"));
        assert_eq!(forwarded[7], "--force");
        let script = &forwarded[5];
        assert!(script.contains("--proto-redir '=https'"));
        assert!(script.contains("curl is required inside WSL"));
        assert!(!script.contains("wget"));
    }

    #[test]
    fn setup_without_force_does_not_forward_a_force_flag() {
        let mut wsl = MockWsl::default();
        wsl.default_distro_prefix("Ubuntu");
        let args = ["setup"].map(OsString::from);
        assert_eq!(run(&args, &mut wsl), Ok(Outcome::Exit(0)));
        assert_eq!(
            strings(&wsl.forward_args[0]).last().map(String::as_str),
            Some("yashik-setup")
        );
    }

    #[test]
    fn version_gate_stops_forwarding_when_wsl_binary_is_missing_or_wrong() {
        let mut missing = MockWsl::default();
        missing.default_distro_prefix("Ubuntu");
        missing.queue(Captured {
            success: false,
            code: Some(127),
            stdout: Vec::new(),
            stderr: b"not found".to_vec(),
        });
        let error = run(&[OsString::from("doctor")], &mut missing).unwrap_err();
        assert!(error.contains("setup"));
        assert!(missing.forward_args.is_empty());

        let mut wrong = MockWsl::default();
        wrong.default_distro_prefix("Ubuntu");
        wrong.queue(successful(b"yashik 0.1.0\n".to_vec()));
        assert!(run(&[OsString::from("doctor")], &mut wrong).is_err());
        assert!(wrong.forward_args.is_empty());
    }

    #[test]
    fn linux_absolute_manifest_is_forwarded_directly_and_exit_code_is_preserved() {
        let mut wsl = MockWsl::default();
        wsl.default_distro_prefix("Ubuntu");
        wsl.version_matches();
        wsl.forward_code = 17;
        let args = ["init", "/home/alex/yashik configs/a $b.yaml"].map(OsString::from);
        assert_eq!(run(&args, &mut wsl), Ok(Outcome::Exit(17)));
        let forwarded = strings(&wsl.forward_args[0]);
        assert_eq!(
            forwarded.last().map(String::as_str),
            Some("/home/alex/yashik configs/a $b.yaml")
        );
        assert!(!forwarded
            .iter()
            .any(|argument| argument.contains("wslpath")));
    }

    #[test]
    #[cfg(windows)]
    fn windows_manifest_is_canonicalized_and_given_to_wslpath_as_one_argument() {
        let root = unique_temp_path();
        fs::create_dir_all(&root).unwrap();
        let manifest = root.join("space $ & ' manifest.yaml");
        fs::write(&manifest, "version: 1\n").unwrap();
        let canonical = fs::canonicalize(&manifest).unwrap();
        let canonical_text = canonical.to_string_lossy().into_owned();
        let expected = canonical_windows_path(&canonical_text);

        let mut wsl = MockWsl::default();
        wsl.explicit_distro_prefix("Ubuntu");
        wsl.version_matches();
        wsl.queue(successful(
            b"/mnt/c/temp/space $ & ' manifest.yaml\n".to_vec(),
        ));
        let args = ["--distro", "Ubuntu", "check", manifest.to_str().unwrap()].map(OsString::from);
        assert_eq!(run(&args, &mut wsl), Ok(Outcome::Exit(0)));
        let conversion_call = wsl
            .capture_args
            .iter()
            .find(|arguments| {
                arguments
                    .iter()
                    .any(|argument| argument == OsStr::new("wslpath"))
            })
            .expect("wslpath conversion call");
        let conversion = strings(conversion_call);
        assert_eq!(
            conversion[0..6],
            ["--distribution", "Ubuntu", "--exec", "wslpath", "-a", "-u"]
        );
        assert_eq!(conversion[6], expected);
        assert_eq!(
            strings(&wsl.forward_args[0]).last().map(String::as_str),
            Some("/mnt/c/temp/space $ & ' manifest.yaml")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unknown_flags_and_extra_operands_fail_before_wsl_calls() {
        for args in [
            vec!["--unknown"],
            vec!["init", "manifest.yaml", "extra"],
            vec!["check", "--unknown"],
            vec!["doctor", "extra"],
            vec!["setup", "--other"],
        ] {
            let mut wsl = MockWsl::default();
            let args = args.into_iter().map(OsString::from).collect::<Vec<_>>();
            assert!(run(&args, &mut wsl).is_err());
            assert!(wsl.capture_args.is_empty());
        }
    }

    #[test]
    fn verbatim_windows_paths_become_regular_drive_and_unc_paths() {
        assert_eq!(
            canonical_windows_path(r"\\?\C:\Temp\λ file.yaml"),
            r"C:\Temp\λ file.yaml"
        );
        assert_eq!(
            canonical_windows_path(r"\\?\UNC\server\share\file.yaml"),
            r"\\server\share\file.yaml"
        );
        assert_eq!(
            canonical_windows_path(r"\\server\share\file.yaml"),
            r"\\server\share\file.yaml"
        );
    }

    #[cfg(unix)]
    fn setup_shell(root: &Path, include_curl: bool) -> PathBuf {
        let bin = root.join("bin");
        fs::create_dir_all(&bin).unwrap();
        for command in ["awk", "cp", "mktemp", "rm", "sha256sum"] {
            let source = Path::new("/usr/bin").join(command);
            if source.exists() {
                symlink(source, bin.join(command)).unwrap();
            }
        }
        if include_curl {
            let curl = bin.join("curl");
            fs::write(
                &curl,
                "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$*\" >> \"$YASHIK_SETUP_CURL_LOG\"\nurl=\noutput=\nwhile [ \"$#\" -gt 0 ]; do\n  case \"$1\" in\n    --output) output=$2; shift 2; continue ;;\n    https://*) url=$1 ;;\n  esac\n  shift\ndone\n[ -n \"$url\" ] && [ -n \"$output\" ] || exit 2\ncp \"$YASHIK_SETUP_FIXTURE/${url##*/}\" \"$output\"\n",
            )
            .unwrap();
            fs::set_permissions(&curl, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let fake_sh = bin.join("sh");
        fs::write(
            &fake_sh,
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$YASHIK_SETUP_EXECUTED\"\n",
        )
        .unwrap();
        fs::set_permissions(&fake_sh, fs::Permissions::from_mode(0o755)).unwrap();
        bin
    }

    #[cfg(unix)]
    fn run_setup_script(
        root: &Path,
        include_curl: bool,
        checksum: &str,
    ) -> (std::process::Output, PathBuf, PathBuf) {
        let fixture = root.join("fixture");
        fs::create_dir_all(&fixture).unwrap();
        let script = b"verified installer fixture\n";
        fs::write(fixture.join("install.sh"), script).unwrap();
        fs::write(
            fixture.join("SHA256SUMS"),
            format!("{checksum}  install.sh\n"),
        )
        .unwrap();
        let bin = setup_shell(root, include_curl);
        let executed = root.join("installer-ran");
        let curl_log = root.join("curl-args");
        let output = Command::new("/bin/sh")
            .arg("-c")
            .arg(setup_script())
            .arg("yashik-setup")
            .env("PATH", &bin)
            .env("YASHIK_SETUP_FIXTURE", &fixture)
            .env("YASHIK_SETUP_EXECUTED", &executed)
            .env("YASHIK_SETUP_CURL_LOG", &curl_log)
            .output()
            .unwrap();
        (output, executed, curl_log)
    }

    #[cfg(unix)]
    #[test]
    fn setup_script_verifies_the_pinned_installer_before_executing_it() {
        let valid_root = unique_temp_path();
        fs::create_dir_all(&valid_root).unwrap();
        let digest = "c78518d18e57979ae32c87d35c08ee20eb388d98b488c568162e57734dcfab79";
        let (valid, executed, curl_log) = run_setup_script(&valid_root, true, digest);
        assert!(
            valid.status.success(),
            "{}",
            String::from_utf8_lossy(&valid.stderr)
        );
        assert!(executed.is_file());
        assert!(String::from_utf8_lossy(&fs::read(curl_log).unwrap()).contains("--proto-redir"));
        fs::remove_dir_all(&valid_root).unwrap();

        let invalid_root = unique_temp_path();
        fs::create_dir_all(&invalid_root).unwrap();
        let (invalid, executed, _) = run_setup_script(&invalid_root, true, &"0".repeat(64));
        assert!(!invalid.status.success());
        assert!(String::from_utf8_lossy(&invalid.stderr).contains("SHA-256 mismatch"));
        assert!(!executed.exists(), "unverified installer was executed");
        fs::remove_dir_all(&invalid_root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn setup_script_reports_missing_curl_without_running_a_downloader() {
        let root = unique_temp_path();
        fs::create_dir_all(&root).unwrap();
        let (result, executed, _) = run_setup_script(&root, false, &"0".repeat(64));
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("curl is required inside WSL"));
        assert!(!executed.exists());
        fs::remove_dir_all(root).unwrap();
    }
}
