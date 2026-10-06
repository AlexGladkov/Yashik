import hashlib
import json
import os
import pathlib
import shutil
import stat
import subprocess
import tarfile
import tempfile
import unittest
from io import BytesIO


ROOT = pathlib.Path(__file__).resolve().parents[1]
INSTALL_SCRIPT = ROOT / "scripts" / "install.sh"
VERSION = "0.4.0"


class InstallerScriptTests(unittest.TestCase):
    def setUp(self):
        self.temp_root = pathlib.Path(tempfile.mkdtemp(prefix="yashik install tests "))
        self.fixture_dir = self.temp_root / "release assets"
        self.fixture_dir.mkdir()
        self.fake_bin = self.temp_root / "mock tools"
        self.fake_bin.mkdir()
        self.home = self.temp_root / "home with spaces"
        self.home.mkdir()
        self.download_log = self.temp_root / "download urls.txt"
        self.curl_args_log = self.temp_root / "curl args.txt"
        self._write_mock_tools()
        self.env = os.environ.copy()
        self.env.update(
            {
                "HOME": str(self.home),
                "YASHIK_FIXTURE_DIR": str(self.fixture_dir),
                "YASHIK_DOWNLOAD_LOG": str(self.download_log),
                "YASHIK_CURL_ARGS_LOG": str(self.curl_args_log),
                "YASHIK_TEST_OS": "Linux",
                "YASHIK_TEST_ARCH": "x86_64",
                "PATH": str(self.fake_bin) + os.pathsep + os.environ.get("PATH", ""),
            }
        )
        self.asset_name = "yashik-linux-x86_64.tar.gz"
        self.binary_payload = self.make_binary(VERSION)
        self.write_release()

    def tearDown(self):
        shutil.rmtree(self.temp_root, ignore_errors=True)

    def _write_mock_tools(self):
        uname = self.fake_bin / "uname"
        uname.write_text(
            "#!/bin/sh\n"
            "case \"$1\" in\n"
            "  -s) printf '%s\\n' \"$YASHIK_TEST_OS\" ;;\n"
            "  -m) printf '%s\\n' \"$YASHIK_TEST_ARCH\" ;;\n"
            "  *) exit 2 ;;\n"
            "esac\n",
            encoding="utf-8",
        )
        uname.chmod(0o755)

        curl = self.fake_bin / "curl"
        curl.write_text(
            "#!/usr/bin/env python3\n"
            "import json, os, pathlib, shutil, sys\n"
            "args = sys.argv[1:]\n"
            "try:\n"
            "    output = pathlib.Path(args[args.index('--output') + 1])\n"
            "    url = next(arg for arg in args if arg.startswith('https://'))\n"
            "except (ValueError, StopIteration, IndexError):\n"
            "    sys.exit(2)\n"
            "with open(os.environ['YASHIK_DOWNLOAD_LOG'], 'a', encoding='utf-8') as log:\n"
            "    log.write(url + '\\n')\n"
            "with open(os.environ['YASHIK_CURL_ARGS_LOG'], 'a', encoding='utf-8') as log:\n"
            "    log.write(json.dumps(args) + '\\n')\n"
            "source = pathlib.Path(os.environ['YASHIK_FIXTURE_DIR']) / url.rsplit('/', 1)[-1]\n"
            "if not source.is_file():\n"
            "    sys.exit(22)\n"
            "shutil.copyfile(source, output)\n",
            encoding="utf-8",
        )
        curl.chmod(0o755)

    @staticmethod
    def make_binary(version):
        return (
            "#!/bin/sh\n"
            "if [ \"${1-}\" = --version ]; then\n"
            f"  printf 'yashik {version}\\n'\n"
            "  exit 0\n"
            "fi\n"
            "exit 0\n"
        ).encode()

    def write_archive(self, entries=None):
        archive_path = self.fixture_dir / self.asset_name
        if entries is None:
            entries = [("yashik", self.binary_payload, tarfile.REGTYPE, 0o755, "")]
        with tarfile.open(archive_path, "w:gz") as archive:
            for name, payload, entry_type, mode, linkname in entries:
                info = tarfile.TarInfo(name)
                info.type = entry_type
                info.mode = mode
                info.linkname = linkname
                if entry_type in (tarfile.REGTYPE, tarfile.AREGTYPE):
                    info.size = len(payload)
                    archive.addfile(info, BytesIO(payload))
                else:
                    archive.addfile(info)
        return archive_path

    def write_manifest(self, content=None):
        archive_path = self.fixture_dir / self.asset_name
        if content is None:
            checksum = hashlib.sha256(archive_path.read_bytes()).hexdigest()
            content = f"{checksum}  {self.asset_name}\n"
        (self.fixture_dir / "SHA256SUMS").write_text(content, encoding="ascii")

    def write_release(self, entries=None, reported_version=None, manifest=None):
        if reported_version is not None:
            self.binary_payload = self.make_binary(reported_version)
        self.write_archive(entries)
        self.write_manifest(manifest)

    def run_installer(self, *args, env=None):
        run_env = self.env.copy()
        if env:
            run_env.update(env)
        return subprocess.run(
            ["/bin/sh", str(INSTALL_SCRIPT), *args],
            cwd=ROOT,
            env=run_env,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=20,
        )

    def expected_asset_for(self, os_name, arch):
        family = "linux" if os_name == "Linux" else "macos"
        machine = "x86_64" if arch in ("x86_64", "amd64") else "aarch64"
        return f"yashik-{family}-{machine}.tar.gz"

    def test_help_does_not_need_download_or_hash_tools(self):
        result = self.run_installer("--help", env={"PATH": "/no-tools"})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("--version VERSION", result.stdout)
        self.assertFalse(self.download_log.exists())

    def test_default_install_uses_pinned_version_and_handles_spaces(self):
        result = self.run_installer()
        target = self.home / ".local" / "bin" / "yashik"
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(target.read_bytes(), self.binary_payload)
        self.assertEqual(stat.S_IMODE(target.stat().st_mode), 0o755)
        self.assertIn("Installed Yashik 0.4.0", result.stdout)
        self.assertIn("export PATH='" + str(target.parent) + "':\"$PATH\"", result.stdout)
        self.assertEqual(
            self.download_log.read_text(encoding="utf-8").splitlines(),
            [
                "https://github.com/AlexGladkov/Yashik/releases/download/v0.4.0/"
                "yashik-linux-x86_64.tar.gz",
                "https://github.com/AlexGladkov/Yashik/releases/download/v0.4.0/SHA256SUMS",
            ],
        )

    def test_curl_restricts_initial_requests_and_redirects_to_https(self):
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = [
            json.loads(line)
            for line in self.curl_args_log.read_text(encoding="utf-8").splitlines()
        ]
        self.assertEqual(len(calls), 2)
        for arguments in calls:
            self.assertEqual(arguments[arguments.index("--proto") + 1], "=https")
            self.assertEqual(arguments[arguments.index("--proto-redir") + 1], "=https")
            self.assertIn("--tlsv1.2", arguments)

    def test_missing_curl_fails_before_download_or_destination_creation(self):
        no_curl = self.temp_root / "tools without curl"
        no_curl.mkdir()
        shutil.copyfile(self.fake_bin / "uname", no_curl / "uname")
        (no_curl / "uname").chmod(0o755)
        destination = self.temp_root / "must not exist"

        result = self.run_installer(
            "--bin-dir",
            str(destination),
            env={"PATH": str(no_curl)},
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("curl is required", result.stderr)
        self.assertFalse(destination.exists())
        self.assertFalse(self.download_log.exists())

    def test_version_prefix_and_alias_platform_asset_mapping(self):
        cases = [
            ("Linux", "amd64", "yashik-linux-x86_64.tar.gz"),
            ("Linux", "arm64", "yashik-linux-aarch64.tar.gz"),
            ("Darwin", "x86_64", "yashik-macos-x86_64.tar.gz"),
            ("Darwin", "aarch64", "yashik-macos-aarch64.tar.gz"),
        ]
        for os_name, arch, expected_asset in cases:
            with self.subTest(os=os_name, arch=arch):
                self.env["YASHIK_TEST_OS"] = os_name
                self.env["YASHIK_TEST_ARCH"] = arch
                self.asset_name = expected_asset
                self.write_release()
                result = self.run_installer("--version", "v0.4.0", "--bin-dir", str(self.temp_root / "custom bin"))
                self.assertEqual(result.returncode, 0, result.stderr)
                urls = self.download_log.read_text(encoding="utf-8").splitlines()
                self.assertIn("/download/v0.4.0/" + expected_asset, urls[-2])
                self.assertTrue((self.temp_root / "custom bin" / "yashik").is_file())

    def test_byte_identical_repeat_does_not_replace_file(self):
        target = self.home / ".local" / "bin" / "yashik"
        first = self.run_installer()
        self.assertEqual(first.returncode, 0, first.stderr)
        before = target.stat()
        second = self.run_installer()
        after = target.stat()
        self.assertEqual(second.returncode, 0, second.stderr)
        self.assertIn("already installed", second.stdout)
        self.assertEqual((before.st_ino, before.st_mtime_ns), (after.st_ino, after.st_mtime_ns))

    def test_byte_identical_non_executable_file_is_repaired_without_force(self):
        target = self.home / ".local" / "bin" / "yashik"
        target.parent.mkdir(parents=True)
        target.write_bytes(self.binary_payload)
        target.chmod(0o644)

        result = self.run_installer()

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Installed Yashik 0.4.0", result.stdout)
        self.assertEqual(target.read_bytes(), self.binary_payload)
        self.assertEqual(stat.S_IMODE(target.stat().st_mode), 0o755)

    def test_foreign_regular_file_requires_force_and_force_replaces_after_validation(self):
        target = self.home / ".local" / "bin" / "yashik"
        target.parent.mkdir(parents=True)
        target.write_bytes(b"keep this foreign program")
        before = target.stat()
        refused = self.run_installer()
        self.assertNotEqual(refused.returncode, 0)
        self.assertIn("--force", refused.stderr)
        self.assertEqual(target.read_bytes(), b"keep this foreign program")
        self.assertEqual((target.stat().st_ino, target.stat().st_mtime_ns), (before.st_ino, before.st_mtime_ns))

        replaced = self.run_installer("--force")
        self.assertEqual(replaced.returncode, 0, replaced.stderr)
        self.assertEqual(target.read_bytes(), self.binary_payload)

    def test_symlink_directory_and_fifo_targets_are_refused_even_with_force(self):
        bin_dir = self.temp_root / "special targets"
        bin_dir.mkdir()
        target = bin_dir / "yashik"
        sentinel = self.temp_root / "outside sentinel"
        sentinel.write_bytes(b"outside bytes")

        target.symlink_to(sentinel)
        result = self.run_installer("--bin-dir", str(bin_dir), "--force")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("symlink", result.stderr)
        self.assertEqual(sentinel.read_bytes(), b"outside bytes")
        target.unlink()

        target.mkdir()
        result = self.run_installer("--bin-dir", str(bin_dir), "--force")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("non-regular", result.stderr)
        target.rmdir()

        os.mkfifo(target)
        result = self.run_installer("--bin-dir", str(bin_dir), "--force")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("non-regular", result.stderr)
        self.assertTrue(stat.S_ISFIFO(target.stat().st_mode))

    def test_unsupported_platform_fails_before_creating_destination(self):
        destination = self.temp_root / "must not exist"
        result = self.run_installer("--bin-dir", str(destination), env={"YASHIK_TEST_OS": "FreeBSD"})
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("unsupported platform", result.stderr)
        self.assertFalse(destination.exists())
        self.assertFalse(self.download_log.exists())

        result = self.run_installer("--bin-dir", str(destination), env={"YASHIK_TEST_ARCH": "riscv64"})
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(destination.exists())

    def test_bad_arguments_and_version_are_rejected_before_download(self):
        invalid_arguments = [
            ("--unknown",),
            ("--version",),
            ("--bin-dir",),
            ("--version", "0.4.0;touch /tmp/pwned"),
            ("--version", "01.2.1"),
            ("--bin-dir", "relative/path"),
            ("--bin-dir", ""),
        ]
        for args in invalid_arguments:
            with self.subTest(args=args):
                result = self.run_installer(*args)
                self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.download_log.exists())

    def test_checksum_missing_duplicate_malformed_or_mismatched_preserves_target(self):
        target = self.home / ".local" / "bin" / "yashik"
        target.parent.mkdir(parents=True)
        original = b"preexisting binary"
        target.write_bytes(original)
        good_hash = hashlib.sha256((self.fixture_dir / self.asset_name).read_bytes()).hexdigest()
        bad_manifests = [
            "" ,
            f"{good_hash}  {self.asset_name}\n{good_hash}  {self.asset_name}\n",
            f"not-a-sha256  {self.asset_name}\n",
            f"{'0' * 64}  {self.asset_name}\n",
        ]
        for manifest in bad_manifests:
            with self.subTest(manifest=manifest[:40]):
                self.write_manifest(manifest)
                result = self.run_installer("--force")
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(target.read_bytes(), original)

    def test_archive_extra_link_or_traversal_member_is_rejected(self):
        good_binary = self.binary_payload
        cases = [
            [
                ("yashik", good_binary, tarfile.REGTYPE, 0o755, ""),
                ("extra", b"extra", tarfile.REGTYPE, 0o644, ""),
            ],
            [("yashik", b"marker", tarfile.SYMTYPE, 0o777, "outside")],
            [("../outside", good_binary, tarfile.REGTYPE, 0o755, "")],
            [("nested/yashik", good_binary, tarfile.REGTYPE, 0o755, "")],
        ]
        destination = self.temp_root / "archive checks"
        destination.mkdir()
        sentinel = self.temp_root / "outside"
        sentinel.write_bytes(b"safe")
        target = destination / "yashik"
        target.write_bytes(b"old")
        for entries in cases:
            with self.subTest(entries=[entry[0] for entry in entries]):
                self.write_release(entries=entries)
                result = self.run_installer("--bin-dir", str(destination), "--force")
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(target.read_bytes(), b"old")
                self.assertEqual(sentinel.read_bytes(), b"safe")

    def test_staged_binary_must_report_requested_version_before_replace(self):
        target = self.home / ".local" / "bin" / "yashik"
        target.parent.mkdir(parents=True)
        target.write_bytes(b"old binary")
        self.write_release(reported_version="9.9.9")
        result = self.run_installer("--force")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("exactly", result.stderr)
        self.assertEqual(target.read_bytes(), b"old binary")


if __name__ == "__main__":
    unittest.main()
