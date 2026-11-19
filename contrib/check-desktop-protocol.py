#!/usr/bin/env python3
"""Exercise the desktop protocol exported by an actual binary or AppImage."""

import ast
import ctypes
import ctypes.util
from contextlib import contextmanager
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import xml.etree.ElementTree as ET

MANAGER_DESKTOP_ID = "managed-ira.desktop"


def call(destination, path, method, *arguments):
    result = subprocess.run(
        ["gdbus", "call", "--session", "--dest", destination,
         "--object-path", path, "--method", method, "--", *arguments],
        capture_output=True, text=True, timeout=3, check=True,
    )
    return result.stdout.strip()


@contextmanager
def virtual_display():
    display = subprocess.Popen(["Xvfb", "-displayfd", "1", "-screen", "0", "1024x768x24"],
                               stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    try:
        number = display.stdout.readline().strip()
        if not number:
            raise RuntimeError("Xvfb did not publish a display number")
        yield f":{number}"
    finally:
        display.terminate()
        display.wait(timeout=10)


def probe(binary):
    with tempfile.TemporaryDirectory(prefix="ira-protocol-") as directory:
        root = Path(directory)
        runtime = root / "runtime"
        runtime.mkdir(mode=0o700)
        env = {**os.environ, "HOME": str(root), "XDG_RUNTIME_DIR": str(runtime),
               "XDG_CONFIG_HOME": str(root / "config"), "XDG_DATA_HOME": str(root / "data"),
               "XDG_CACHE_HOME": str(root / "cache"), "GSETTINGS_BACKEND": "memory",
               "XDG_DATA_DIRS": "/usr/local/share:/usr/share", "GDK_BACKEND": "x11",
               "GSK_RENDERER": "cairo", "NO_AT_BRIDGE": "1", "LC_ALL": "C.UTF-8"}
        for key in ("WAYLAND_DISPLAY", "APPIMAGE", "APPDIR", "IRA_HOST_DATA_DIRS"):
            env.pop(key, None)
        env["XDG_DATA_DIRS"] = f"{root / 'data/flatpak/exports/share'}:{env['XDG_DATA_DIRS']}"
        prepare_launcher(binary, root)
        with virtual_display() as display:
            env["DISPLAY"] = display
            with (root / "ira.log").open("w+") as log:
                application = subprocess.Popen([str(binary)], env=env, stdout=log,
                                               stderr=subprocess.STDOUT, start_new_session=True)
                try:
                    inspect_tray(application)
                    inspect_window_identity(env)
                    inspect_registration(binary, root, env)
                except Exception:
                    log.seek(0)
                    print(log.read(), file=sys.stderr)
                    raise
                finally:
                    try:
                        os.killpg(application.pid, signal.SIGTERM)
                    except ProcessLookupError:
                        pass
                    application.wait(timeout=10)


def inspect_tray(application):
    deadline = time.monotonic() + 30
    name = None
    while time.monotonic() < deadline:
        names, = ast.literal_eval(call("org.freedesktop.DBus", "/org/freedesktop/DBus",
                                      "org.freedesktop.DBus.ListNames"))
        name = next((name for name in names if name.startswith("org.kde.StatusNotifierItem-")), None)
        if name:
            break
        if application.poll() is not None:
            raise RuntimeError("Ira exited before exporting its tray")
        time.sleep(0.05)
    if not name:
        raise RuntimeError("Ira did not export its tray within 30 seconds")
    xml, = ast.literal_eval(call(name, "/StatusNotifierItem", "org.freedesktop.DBus.Introspectable.Introspect"))
    interface = ET.fromstring(xml).find("./interface[@name='org.kde.StatusNotifierItem']")
    if interface is None or interface.find("./method[@name='Activate']") is not None:
        raise RuntimeError("Packaged tray advertises Activate; it still triggers click/double-click disambiguation")
    is_menu = call(name, "/StatusNotifierItem", "org.freedesktop.DBus.Properties.Get",
                   "org.kde.StatusNotifierItem", "ItemIsMenu")
    if "true" not in is_menu:
        raise RuntimeError(f"Packaged tray is not menu-only: {is_menu}")
    started = time.monotonic()
    for _ in range(10):
        call(name, "/MenuBar", "com.canonical.dbusmenu.AboutToShow", "0")
        layout = call(name, "/MenuBar", "com.canonical.dbusmenu.GetLayout", "0", "-1", "[]")
        if "Hide to background" not in layout or "Show Ira" in layout or "Quit" not in layout:
            raise RuntimeError(f"Unexpected initial tray actions: {layout}")
    print(f"Artifact verified: menu-only interface; 10 menu-open/layout pairs in {time.monotonic() - started:.3f}s")


def prepare_launcher(binary, root):
    applications = root / "data/applications"
    applications.mkdir(parents=True, exist_ok=True)
    launcher = applications / MANAGER_DESKTOP_ID
    executable = str(binary).replace("%", "%%").replace("\\", "\\\\").replace('"', '\\"')
    original = f'[Desktop Entry]\nType=Application\nName=Ira\nExec=env DESKTOPINTEGRATION=1 "{executable}"\n'
    if launcher.exists():
        if launcher.read_text() != original:
            raise RuntimeError("Artifact modified the manager-owned launcher")
    else:
        launcher.write_text(original)
    return applications, launcher, original


def inspect_window_identity(env):
    x11 = ctypes.CDLL(ctypes.util.find_library("X11"))
    pointer, window = ctypes.c_void_p, ctypes.c_ulong
    x11.XOpenDisplay.argtypes, x11.XOpenDisplay.restype = [ctypes.c_char_p], pointer
    x11.XDefaultRootWindow.argtypes, x11.XDefaultRootWindow.restype = [pointer], window
    x11.XQueryTree.argtypes = [pointer, window, ctypes.POINTER(window), ctypes.POINTER(window),
                              ctypes.POINTER(ctypes.POINTER(window)), ctypes.POINTER(ctypes.c_uint)]
    class Hint(ctypes.Structure):
        _fields_ = [("name", pointer), ("class_name", pointer)]
    x11.XGetClassHint.argtypes = [pointer, window, ctypes.POINTER(Hint)]
    x11.XFree.argtypes, x11.XCloseDisplay.argtypes = [pointer], [pointer]
    display = x11.XOpenDisplay(env["DISPLAY"].encode())
    if not display:
        raise RuntimeError("Could not inspect the artifact's X11 display")
    try:
        parent, root, children, count = window(), window(), ctypes.POINTER(window)(), ctypes.c_uint()
        if not x11.XQueryTree(display, x11.XDefaultRootWindow(display), ctypes.byref(root),
                              ctypes.byref(parent), ctypes.byref(children), ctypes.byref(count)):
            raise RuntimeError("Could not enumerate artifact windows")
        identities = []
        try:
            for index in range(count.value):
                hint = Hint()
                if x11.XGetClassHint(display, children[index], ctypes.byref(hint)):
                    try:
                        identities.append((ctypes.string_at(hint.name).decode(), ctypes.string_at(hint.class_name).decode()))
                    finally:
                        x11.XFree(hint.name)
                        x11.XFree(hint.class_name)
        finally:
            x11.XFree(children)
        expected = MANAGER_DESKTOP_ID.removesuffix(".desktop")
        if not identities or any(class_name != expected for _, class_name in identities):
            raise RuntimeError(f"Window identity does not match the existing {MANAGER_DESKTOP_ID} launcher: {identities}")
        print(f"Artifact verified: windows match the manager-owned {MANAGER_DESKTOP_ID} launcher")
    finally:
        x11.XCloseDisplay(display)


def inspect_registration(binary, root, env):
    applications, launcher, original = prepare_launcher(binary, root)
    tools = root / "bin"
    tools.mkdir()
    systemctl = tools / "systemctl"
    systemctl.write_text('#!/bin/sh\n[ "$1" = --user ] && [ "$2" = daemon-reload ]\n')
    systemctl.chmod(0o755)
    env = {**env, "PATH": f"{tools}:{env['PATH']}"}
    dropin = root / "config/systemd/user/org.gnome.Shell@.service.d/70-ira-desktop.conf"
    exported_provider = root / "data/flatpak/exports/share/gnome-shell/search-providers/com.github.ira.ini"
    for mode in ("--install-desktop-integration", "--uninstall-desktop-integration"):
        result = subprocess.run([str(binary), mode], env=env, capture_output=True, text=True, timeout=20)
        if result.returncode:
            raise RuntimeError(f"Artifact {mode} failed: {result.stderr}")
        if launcher.read_text() != original or list(applications.iterdir()) != [launcher]:
            raise RuntimeError("Artifact changed the app manager's launcher or created duplicates")
        if mode == "--install-desktop-integration":
            text = dropin.read_text()
            if "Environment=\"XDG_DATA_DIRS=" not in text or any(part in text for part in (".mount_", "appimage_extracted_", "ira-loaders.")):
                raise RuntimeError(f"Artifact leaked temporary AppImage paths into session setup: {text}")
            provider = (root / "data/gnome-shell/search-providers/com.github.ira.ini").read_text()
            if f"DesktopId={MANAGER_DESKTOP_ID}\n" not in provider:
                raise RuntimeError(f"Artifact registered the wrong desktop launcher: {provider}")
            if exported_provider.read_text() != provider:
                raise RuntimeError("Artifact did not register its discoverable user export")
        elif dropin.exists():
            raise RuntimeError("Artifact did not remove its user-session override")
        elif exported_provider.exists():
            raise RuntimeError("Artifact did not remove its exported provider")
    print("Artifact verified: user-only install/uninstall; existing launcher preserved; no temporary session paths")


def main():
    inside = len(sys.argv) == 3 and sys.argv[1] == "--inside"
    if len(sys.argv) != (3 if inside else 2):
        raise SystemExit(f"Usage: {sys.argv[0]} /absolute/path/to/ira-or-AppImage")
    binary = Path(sys.argv[-1]).resolve()
    if not inside:
        with tempfile.TemporaryDirectory(prefix="ira-protocol-bus-") as directory:
            config = Path(directory) / "bus.conf"
            config.write_text(f"<busconfig><type>session</type><listen>unix:tmpdir={directory}</listen>"
                              "<auth>EXTERNAL</auth><policy context='default'><allow own='*'/>"
                              "<allow send_destination='*'/><allow receive_sender='*'/></policy></busconfig>")
            result = subprocess.run(["dbus-run-session", f"--config-file={config}", "--", sys.executable,
                                     str(Path(__file__).resolve()), "--inside", str(binary)])
        raise SystemExit(result.returncode)
    probe(binary)


if __name__ == "__main__":
    main()
