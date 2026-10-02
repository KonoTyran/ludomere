"""Exercise the private launcher boundary without invoking downloaded software."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


class LauncherBoundary(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        shutil.copyfile(Path(__file__).with_name("umu-run"), self.root / "umu-run")
        package = self.root / "vendor/umu"
        package.mkdir(parents=True)
        (package / "__init__.py").write_text('__version__ = "1.4.4"\n')
        (package / "umu_run.py").write_text('''
import os
from pathlib import Path
from types import SimpleNamespace
def get_umu_proton(*args):
    raise AssertionError("upstream Proton acquisition was reached")
def setup_umu(*args):
    raise AssertionError("upstream runtime acquisition was reached")
def resolve_runtime():
    return SimpleNamespace(name="sniper", path=Path(os.environ["UMU_FOLDERS_PATH"]) / "umu/steamrt3", as_tuple=lambda: ("sniper", "steamrt3", "1628350"))
''')
        (package / "__main__.py").write_text('''
import os
from . import umu_run
def main():
    assert os.environ["UMU_LOG"] == "info"
    assert "UMU_NO_PROTON" not in os.environ
    assert "UMU_NO_RUNTIME" not in os.environ
    assert "RUNTIMEPATH" not in os.environ
    assert os.environ["PYTHONDONTWRITEBYTECODE"] == "1"
    umu_run.get_umu_proton({}, None)
    runtime = umu_run.resolve_runtime()
    umu_run.setup_umu(runtime.path, runtime.as_tuple(), None)
    print("launch accepted")
    return 0
''')
        self.proton = self.root / "proton"
        self.proton.mkdir()
        (self.proton / "proton").touch()
        (self.proton / "toolmanifest.vdf").touch()
        self.runtime = self.root / "data/umu/steamrt3"
        self.runtime.mkdir(parents=True)
        self.env = {
            "PATH": "/usr/bin", "HOME": str(self.root),
            "PROTONPATH": str(self.proton), "UMU_FOLDERS_PATH": str(self.root / "data"),
            "UMU_LOG": "debug", "UMU_NO_PROTON": "1", "UMU_NO_RUNTIME": "1", "RUNTIMEPATH": "wrong",
        }

    def invoke(self, *args, env=None):
        return subprocess.run(["/usr/bin/python3", "-I", str(self.root / "umu-run"), *args], env=self.env if env is None else env, text=True, capture_output=True, check=False)

    def install_runtime(self):
        for file in (".installed.ok", "_v2-entry-point", "toolmanifest.vdf", "VERSIONS.txt", "pressure-vessel/bin/pv-verify"):
            path = self.runtime / file
            path.parent.mkdir(parents=True, exist_ok=True)
            path.touch()
        (self.runtime / "sniper_platform_fixture/files").mkdir(parents=True)

    def test_version_needs_no_setup(self):
        result = self.invoke("--version", env={"PATH": "/usr/bin"})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("1.4.4", result.stdout)

    def test_missing_runtime_never_reaches_upstream_acquisition(self):
        result = self.invoke("game.exe")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Download the missing Steam Linux Runtime", result.stderr)

    def test_ready_launch_replaces_acquisition_hooks(self):
        self.install_runtime()
        result = self.invoke("game.exe")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("launch accepted", result.stdout)
        self.assertFalse(list(self.root.rglob("__pycache__")))

    def test_proton_alias_and_config_cannot_trigger_downloads(self):
        self.install_runtime()
        result = self.invoke("--config", "untrusted.toml")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("configuration files", result.stderr)
        self.env["PROTONPATH"] = "GE-Proton"
        result = self.invoke("game.exe")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Choose an installed Proton", result.stderr)

    def test_unexpected_upstream_version_is_rejected(self):
        (self.root / "vendor/umu/__init__.py").write_text('__version__ = "99"\n')
        result = self.invoke("--version")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("bundled UMU 1.4.4", result.stderr)


if __name__ == "__main__":
    unittest.main()
