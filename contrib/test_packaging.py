import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


CONTRIB = Path(__file__).resolve().parent


class PackagingTests(unittest.TestCase):
    def run_apprun(self, with_loaders, nested=False):
        with tempfile.TemporaryDirectory(prefix="ira package & ") as tmp:
            root = Path(tmp)
            (root / "usr/bin").mkdir(parents=True)
            (root / "runtime").mkdir(mode=0o700)
            shutil.copy(CONTRIB / "AppRun", root / "AppRun")
            binary = root / "usr/bin/ira"
            binary.write_text(
                "#!/usr/bin/env python3\n"
                "import json, os, sys\n"
                "keys = ['LD_LIBRARY_PATH', 'IRA_LOCALEDIR', 'IRA_HOST_DATA_DIRS', 'XDG_DATA_DIRS']\n"
                "print(json.dumps({'args': sys.argv[1:], 'env': {k: os.environ[k] for k in keys}}))\n"
            )
            binary.chmod(0o755)
            if with_loaders:
                conf = root / "usr/share/glycin-loaders/2+/conf.d/test.conf"
                conf.parent.mkdir(parents=True)
                conf.write_text("[loader:image/svg+xml]\nExec=/usr/libexec/glycin-loaders/2+/glycin-svg\n")
                binary.write_text(binary.read_text() +
                                  "from pathlib import Path\n"
                                  "print((Path(os.environ['XDG_DATA_DIRS'].split(':')[0]) / "
                                  "'glycin-loaders/2+/conf.d/test.conf').read_text())\n")
            result = subprocess.run(
                [str(root / "AppRun"), "--big-picture", "argument with spaces"],
                env={**os.environ, "XDG_RUNTIME_DIR": str(root / "runtime"),
                     "XDG_DATA_DIRS": "/parent/mount/usr/share:/host/share" if nested else "/host/share",
                     "IRA_HOST_DATA_DIRS": "/host/share" if nested else "", "LD_LIBRARY_PATH": "/host/lib"},
                capture_output=True, text=True, check=True,
            )
            output = json.loads(result.stdout.splitlines()[0])
            self.assertEqual(output["args"], ["--big-picture", "argument with spaces"])
            self.assertEqual(output["env"]["LD_LIBRARY_PATH"], "/host/lib")
            self.assertEqual(output["env"]["IRA_LOCALEDIR"], str(root / "usr/share/locale"))
            self.assertTrue(output["env"]["XDG_DATA_DIRS"].endswith("/host/share"))
            self.assertEqual(output["env"]["IRA_HOST_DATA_DIRS"], "/host/share")
            self.assertNotIn("/parent/mount", output["env"]["XDG_DATA_DIRS"])
            if with_loaders:
                self.assertIn(f"Exec={root}/usr/libexec/glycin-loaders/2+/glycin-svg", result.stdout)
                self.assertEqual(list((root / "runtime").iterdir()), [])

    def test_apprun_relocates_without_changing_library_search(self):
        self.run_apprun(False)

    def test_apprun_relocates_glycin_and_cleans_temporary_configs(self):
        self.run_apprun(True)

    def test_apprun_nested_launch_preserves_host_paths_without_parent_mount(self):
        self.run_apprun(True, nested=True)

    def test_install_stages_all_helpers_under_custom_prefix(self):
        with tempfile.TemporaryDirectory(prefix="ira install ") as tmp:
            root = Path(tmp)
            build = root / "build"
            build.mkdir()
            names = ["ira", "ira-input", "ira-overlay-ui", "libira_overlay_shim.so", "libira_overlay_vk.so"]
            for name in names:
                (build / name).write_text("fixture")
            subprocess.run(
                ["bash", str(CONTRIB / "install.sh")], check=True,
                env={**os.environ, "DESTDIR": str(root / "stage"), "PREFIX": "/opt/ira",
                     "IRA_BUILD_DIR": str(build)},
            )
            dest = root / "stage/opt/ira"
            for name in names:
                self.assertTrue(os.access(dest / "lib/ira" / name, os.X_OK))
            self.assertTrue((dest / "bin/ira").is_symlink())
            self.assertTrue(os.access(dest / "bin/ira", os.X_OK))
            self.assertTrue((dest / "share/applications/com.github.ira.desktop").is_file())
            self.assertTrue((dest / "share/metainfo/com.github.ira.metainfo.xml").is_file())
            self.assertTrue((dest / "share/gnome-shell/search-providers/com.github.ira.ini").is_file())
            service = (dest / "share/dbus-1/services/com.github.ira.SearchProvider.service").read_text()
            self.assertIn('Exec="/opt/ira/bin/ira" --search-provider', service)
            self.assertTrue((dest / "lib/udev/rules.d/70-ira-input.rules").is_file())


if __name__ == "__main__":
    unittest.main()
