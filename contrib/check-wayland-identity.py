#!/usr/bin/env python3
"""Check the window identity and remapping without submitting visible buffers.

Run in a Wayland session after building the desktop_identity_probe example.
"""

import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile


def main():
    binary = Path(sys.argv[1] if len(sys.argv) > 1 else
                  "target/debug/examples/desktop_identity_probe").resolve()
    with tempfile.TemporaryDirectory(prefix="ira-wayland-identity-") as directory:
        root = Path(directory)
        applications = root / "data/applications"
        applications.mkdir(parents=True)
        image = root / "Ira.AppImage"
        (applications / "probe-ira.desktop").write_text(
            '[Desktop Entry]\nType=Application\nName=Ira\n'
            f'Exec=env DESKTOPINTEGRATION=1 "{image}"\n')
        env = {**os.environ, "XDG_DATA_HOME": str(root / "data"),
               "XDG_CONFIG_HOME": str(root / "config"),
               "XDG_CACHE_HOME": str(root / "cache"),
               "XDG_DATA_DIRS": "/usr/local/share:/usr/share",
               "APPIMAGE": str(image), "APPDIR": "/", "GDK_BACKEND": "wayland",
               "GSK_RENDERER": "cairo", "WAYLAND_DEBUG": "client",
               "GTK_A11Y": "none", "GSETTINGS_BACKEND": "memory"}
        result = subprocess.run([str(binary)], env=env, capture_output=True,
                                text=True, timeout=30, check=True)
        identities = re.findall(r'xdg_toplevel#\d+\.set_app_id\("([^"]+)"\)', result.stderr)
        if identities.count("probe-ira") != 2 or identities[-1] != "probe-ira":
            raise RuntimeError(f"Window identity failed across remapping: {identities}")
        # Destroying/unmapping detaches a buffer with attach(nil). Any non-null
        # attachment would let this probe display content in the live session.
        attachments = re.findall(r'\.attach\(([^,]+),', result.stderr)
        if any(buffer != "nil" for buffer in attachments):
            raise RuntimeError("Identity probe submitted a visible buffer")
        print("Verified Wayland launcher identity across remapping; no visible buffers submitted.")


if __name__ == "__main__":
    main()
