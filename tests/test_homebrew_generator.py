import importlib.util
import pathlib
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "generate-homebrew.py"
SPEC = importlib.util.spec_from_file_location("generate_homebrew", SCRIPT)
generate_homebrew = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(generate_homebrew)


class HomebrewFormulaTests(unittest.TestCase):
    def setUp(self):
        self.checksums = {
            filename: "a" * 64 for filename in generate_homebrew.CHECKSUM_FILES
        }

    def test_production_formula_uses_tag_url_without_redundant_version(self):
        base = f"{generate_homebrew.PRODUCTION_BASE}/v0.2.1"
        formula = generate_homebrew.render("0.2.1", self.checksums, base)

        self.assertIn(
            'url "https://github.com/AlexGladkov/Yashik/releases/download/v0.2.1/yashik-macos-aarch64.tar.gz"',
            formula,
        )
        self.assertNotIn('  version "0.2.1"', formula)

    def test_local_fixture_formula_keeps_explicit_version(self):
        formula = generate_homebrew.render(
            "0.2.1", self.checksums, "http://127.0.0.1:18765"
        )

        self.assertIn('  version "0.2.1"', formula)
        self.assertIn('url "http://127.0.0.1:18765/yashik-linux-x86_64.tar.gz"', formula)


if __name__ == "__main__":
    unittest.main()
