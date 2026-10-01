import hashlib
import importlib.util
import pathlib
import stat
import struct
import subprocess
import sys
import tempfile
import unittest
import zipfile


ROOT = pathlib.Path(__file__).resolve().parents[1]


def load_script(name):
    path = ROOT / "scripts" / name
    spec = importlib.util.spec_from_file_location(path.stem.replace("-", "_"), path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


package_windows = load_script("package-windows.py")
generate_winget = load_script("generate-winget.py")


def make_pe(path, machine):
    data = bytearray(0x200)
    data[:2] = b"MZ"
    struct.pack_into("<I", data, 0x3C, 0x80)
    data[0x80:0x84] = b"PE\0\0"
    struct.pack_into("<HHIIIHH", data, 0x84, machine, 1, 0, 0, 0, 0xF0, 0x22)
    struct.pack_into("<H", data, 0x98, 0x20B)
    path.write_bytes(data)


class WindowsZipTests(unittest.TestCase):
    def test_pe_parser_reads_supported_machine_types(self):
        with tempfile.TemporaryDirectory() as temporary:
            binary = pathlib.Path(temporary) / "yashik.exe"
            make_pe(binary, 0x8664)
            self.assertEqual(package_windows.pe_machine(binary), 0x8664)
            make_pe(binary, 0xAA64)
            self.assertEqual(package_windows.pe_machine(binary), 0xAA64)

    def test_packages_one_regular_root_executable_deterministically(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            binary = root / "yashik.exe"
            make_pe(binary, 0x8664)
            first = root / "first.zip"
            second = root / "second.zip"

            package_windows.write_archive(binary, first)
            package_windows.write_archive(binary, second)

            self.assertEqual(first.read_bytes(), second.read_bytes())
            with zipfile.ZipFile(first) as archive:
                self.assertEqual(archive.namelist(), ["yashik.exe"])
                self.assertEqual(archive.read("yashik.exe"), binary.read_bytes())
                self.assertTrue(stat.S_ISREG(archive.infolist()[0].external_attr >> 16))
                self.assertEqual(archive.testzip(), None)
            self.assertEqual(
                hashlib.sha256(first.read_bytes()).hexdigest(),
                hashlib.sha256(second.read_bytes()).hexdigest(),
            )

    def test_cli_rejects_wrong_architecture_without_writing_archive(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            binary = root / "yashik.exe"
            output = root / "wrong.zip"
            make_pe(binary, 0x8664)
            result = subprocess.run(
                [
                    sys.executable,
                    str(ROOT / "scripts" / "package-windows.py"),
                    "--binary",
                    str(binary),
                    "--output",
                    str(output),
                    "--version",
                    "0.2.1",
                    "--architecture",
                    "aarch64",
                ],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(result.returncode, 2)
            self.assertIn("does not match aarch64", result.stderr)
            self.assertFalse(output.exists())


class WinGetManifestTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = pathlib.Path(self.temporary.name)
        self.checksums = self.root / "SHA256SUMS"
        self.x64_hash = "1" * 64
        self.arm_hash = "a" * 64
        self.checksums.write_text(
            f"{self.x64_hash}  yashik-windows-x86_64.zip\n"
            f"{self.arm_hash}  yashik-windows-aarch64.zip\n",
            encoding="ascii",
        )

    def test_generator_emits_three_version_pinned_manifests(self):
        checksums = generate_winget.parse_checksums(self.checksums)
        manifests = generate_winget.render("0.2.1", checksums)

        self.assertEqual(
            set(manifests),
            {
                "AlexGladkov.Yashik.yaml",
                "AlexGladkov.Yashik.installer.yaml",
                "AlexGladkov.Yashik.locale.en-US.yaml",
            },
        )
        installer = manifests["AlexGladkov.Yashik.installer.yaml"]
        self.assertIn("InstallerType: zip", installer)
        self.assertEqual(installer.count("NestedInstallerType: portable"), 2)
        self.assertEqual(installer.count("RelativeFilePath: yashik.exe"), 2)
        self.assertEqual(installer.count("PortableCommandAlias: yashik"), 2)
        self.assertIn("Architecture: x64", installer)
        self.assertIn("Architecture: arm64", installer)
        self.assertIn(f"ManifestVersion: {generate_winget.MANIFEST_VERSION}", installer)
        self.assertNotIn("Scope:", installer)
        self.assertIn(
            "https://github.com/AlexGladkov/Yashik/releases/download/v0.2.1/yashik-windows-x86_64.zip",
            installer,
        )
        self.assertIn(f'InstallerSha256: "{self.x64_hash.upper()}"', installer)
        self.assertIn(f'InstallerSha256: "{self.arm_hash.upper()}"', installer)
        self.assertIn("does not install WSL", manifests["AlexGladkov.Yashik.locale.en-US.yaml"])
        self.assertNotIn("Silent", installer)
        self.assertNotIn("latest/download", installer)

    def test_checksum_manifest_rejects_missing_duplicate_and_malformed_rows(self):
        self.checksums.write_text(f"{self.x64_hash}  yashik-windows-x86_64.zip\n", encoding="ascii")
        with self.assertRaisesRegex(ValueError, "missing"):
            generate_winget.parse_checksums(self.checksums)

        self.checksums.write_text(
            f"{self.x64_hash}  yashik-windows-x86_64.zip\n"
            f"{self.x64_hash}  yashik-windows-x86_64.zip\n"
            f"{self.arm_hash}  yashik-windows-aarch64.zip\n",
            encoding="ascii",
        )
        with self.assertRaisesRegex(ValueError, "duplicate"):
            generate_winget.parse_checksums(self.checksums)

        self.checksums.write_text("not-a-sha  yashik-windows-aarch64.zip\n", encoding="ascii")
        with self.assertRaisesRegex(ValueError, "malformed"):
            generate_winget.parse_checksums(self.checksums)

    def test_generator_writes_only_expected_manifest_files(self):
        output_dir = self.root / "manifests" / "0.2.1"
        result = subprocess.run(
            [
                sys.executable,
                str(ROOT / "scripts" / "generate-winget.py"),
                "--version",
                "0.2.1",
                "--checksums",
                str(self.checksums),
                "--output-dir",
                str(output_dir),
            ],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            sorted(path.name for path in output_dir.iterdir()),
            [
                "AlexGladkov.Yashik.installer.yaml",
                "AlexGladkov.Yashik.locale.en-US.yaml",
                "AlexGladkov.Yashik.yaml",
            ],
        )


if __name__ == "__main__":
    unittest.main()
