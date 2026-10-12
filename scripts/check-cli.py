#!/usr/bin/env python3
"""Smoke-test shipped CLI artifacts, including npm and native CA roots.

Run on the artifact's native OS with Python, platform inspection tools, OpenSSL and npm.
No account, registry publication or binary update is used.
Release-only notarization verification may contact Apple.
"""

import argparse
import functools
import http.server
import json
import os
import platform
from pathlib import Path
import shutil
import ssl
import subprocess
import tarfile
import tempfile
import threading


def run(*args, **kwargs):
    return subprocess.run(args, check=True, text=True, capture_output=True, timeout=120, **kwargs).stdout.strip()


def check_static(binary):
    header = run("readelf", "-h", str(binary))
    if "ELF64" not in header or "Advanced Micro Devices X86-64" not in header:
        raise RuntimeError("expected an x86_64 ELF binary")
    program = run("readelf", "-lW", str(binary))
    dynamic = run("readelf", "-dW", str(binary))
    if "INTERP" in program or "(NEEDED)" in dynamic:
        raise RuntimeError("Linux release must be static: found interpreter or shared library dependency")
    if "/nix/store/" in dynamic:
        raise RuntimeError("Linux release has a Nix runtime search path")


def check_macos(binary, release_signed=False):
    if run("lipo", "-archs", str(binary)) != "arm64":
        raise RuntimeError("expected a native arm64 Mach-O binary")
    if "EXECUTE" not in run("otool", "-hv", str(binary)):
        raise RuntimeError("expected a Mach-O executable")
    libraries = run("otool", "-L", str(binary)).splitlines()[1:]
    for library in libraries:
        path = library.strip().split(" (", 1)[0]
        if not path.startswith(("/usr/lib/", "/System/Library/")):
            raise RuntimeError(f"macOS release has a non-system library dependency: {path}")
    commands = run("otool", "-l", str(binary))
    if "/nix/store/" in commands:
        raise RuntimeError("macOS release has a Nix runtime path")
    # The Nix build ad-hoc signs arm64 binaries after fixing library paths.
    # PR artifacts need signature integrity, never an Apple release identity.
    run("codesign", "--verify", "--strict", str(binary))
    if release_signed:
        details = subprocess.run(["codesign", "-dv", "--verbose=4", str(binary)],
                                 check=True, capture_output=True, text=True, timeout=120)
        if "Authority=Developer ID Application:" not in details.stderr:
            raise RuntimeError("macOS release must have a Developer ID Application signature")
        run("codesign", "-vvvv", "-R=notarized", "--check-notarization", str(binary))


def check_packed_dependencies(platform_archive, wrapper_archive, target_platform):
    def manifest(archive):
        with tarfile.open(archive) as package:
            return json.load(package.extractfile("package/package.json"))
    platform = manifest(platform_archive)
    wrapper = manifest(wrapper_archive)
    name = f"@tonk/cli-{target_platform}"
    if platform.get("name") != name:
        raise RuntimeError(f"expected packed platform package {name}")
    target_os, target_cpu = target_platform.split("-")
    if platform.get("os") != [target_os] or platform.get("cpu") != [target_cpu]:
        raise RuntimeError("packed platform package has incorrect OS/CPU selectors")
    version = platform.get("version")
    if not version or wrapper.get("optionalDependencies", {}).get(name) != version:
        raise RuntimeError("packed wrapper must declare the exact platform optional dependency version")


def check_npm(binary, root, temp, env, version, target_platform, package_version=None):
    packages = temp / "packages"
    shutil.copytree(root / "rust/tonk-cli/npm", packages)
    if package_version:
        for manifest in packages.glob("*/package.json"):
            data = json.loads(manifest.read_text())
            data["version"] = package_version
            if "optionalDependencies" in data:
                data["optionalDependencies"] = dict.fromkeys(data["optionalDependencies"], package_version)
            manifest.write_text(json.dumps(data) + "\n")
    target = packages / target_platform / "bin/tonk"
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(binary, target)
    target.chmod(0o755)
    archives = []
    for name in (target_platform, "cli"):
        result = json.loads(run("npm", "pack", "--json", "--ignore-scripts", cwd=packages / name, env=env))
        archives.append(str(packages / name / result[0]["filename"]))
    # Explicitly installing both archives below must not mask broken published
    # metadata: wrapper-only registry installs discover the binary through this.
    check_packed_dependencies(*archives, target_platform)
    install = temp / "install"
    install.mkdir()
    (install / "package.json").write_text('{"name":"tonk-release-smoke","version":"1.0.0","private":true}')
    run("npm", "install", "--offline", "--ignore-scripts", "--no-audit", "--no-fund", *archives, cwd=install, env=env)
    installed = install / f"node_modules/@tonk/cli-{target_platform}/bin/tonk"
    if installed.read_bytes() != binary.read_bytes():
        raise RuntimeError("npm installed different binary bytes")
    actual = run("npm", "exec", "--offline", "--", "tonk", "--version", cwd=install, env=env)
    if actual != version:
        raise RuntimeError(f"npm launcher version mismatch: {actual!r} != {version!r}")
    direct_help = run(str(binary), "--help", env=env)
    launcher_help = run("npm", "exec", "--offline", "--", "tonk", "--help", cwd=install, env=env)
    if direct_help != launcher_help:
        raise RuntimeError("npm launcher help differs from the artifact")


def check_tls(binary, temp, env):
    # A local CA makes this deterministic and proves native roots actually load.
    # The deliberately wrong channel stops update before any binary replacement.
    cert, key = temp / "ca.pem", temp / "key.pem"
    run("openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1",
        "-subj", "/CN=Tonk smoke CA", "-addext", "basicConstraints=critical,CA:TRUE",
        "-keyout", str(key), "-out", str(cert))
    leaf, leaf_key, request = temp / "server.pem", temp / "server-key.pem", temp / "server.csr"
    run("openssl", "req", "-newkey", "rsa:2048", "-nodes", "-subj", "/CN=localhost",
        "-keyout", str(leaf_key), "-out", str(request))
    extensions = temp / "extensions.cnf"
    extensions.write_text("subjectAltName=DNS:localhost\nbasicConstraints=critical,CA:FALSE\nextendedKeyUsage=serverAuth\n")
    run("openssl", "x509", "-req", "-in", str(request), "-CA", str(cert), "-CAkey", str(key),
        "-CAcreateserial", "-days", "1", "-extfile", str(extensions), "-out", str(leaf))
    web = temp / "web/latest/download"
    web.mkdir(parents=True)
    (web / "manifest.json").write_text(json.dumps({
        "version": "0.0.0", "commit": "smoke", "channel": "staging", "built_at": "smoke"
    }))
    handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=str(temp / "web"))
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(leaf, leaf_key)
    server.socket = context.wrap_socket(server.socket, server_side=True)
    worker = threading.Thread(target=server.serve_forever, daemon=True)
    worker.start()
    try:
        empty = temp / "empty-certs"
        empty.mkdir()
        tls_env = {**env, "TONK_UPDATE_ENDPOINT": f"https://localhost:{server.server_port}",
                   "SSL_CERT_DIR": str(empty), "SSL_CERT_FILE": str(empty / "missing.pem"),
                   "NO_PROXY": "localhost,127.0.0.1", "no_proxy": "localhost,127.0.0.1"}
        expected = "release manifest says channel staging, but this install selected stable; refusing to update"
        for trusted in (False, True):
            if trusted:
                tls_env["SSL_CERT_FILE"] = str(cert)
            result = subprocess.run([str(binary), "update"], env=tls_env, text=True,
                                    capture_output=True, timeout=30)
            output = result.stdout + result.stderr
            if result.returncode == 0:
                raise RuntimeError("update unexpectedly accepted the wrong-channel fixture")
            if trusted and expected not in output:
                raise RuntimeError(f"trusted native CA did not reach manifest validation: {output}")
            if not trusted and (expected in output or "could not reach" not in output):
                raise RuntimeError(f"untrusted TLS certificate was not rejected as expected: {output}")
    finally:
        server.shutdown()
        server.server_close()
        worker.join()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--platform", choices=("linux-x64", "darwin-arm64"), required=True)
    parser.add_argument("--release-signed", action="store_true",
                        help="require Developer ID and notarization for a macOS release artifact")
    parser.add_argument("--expected-version", help="release tag version without the v prefix")
    args = parser.parse_args()
    native = (platform.system(), platform.machine())
    expected = {"linux-x64": ("Linux", "x86_64"), "darwin-arm64": ("Darwin", "arm64")}
    if native != expected[args.platform]:
        parser.error(f"{args.platform} requires a native runner, found {native}")
    if args.release_signed and args.platform != "darwin-arm64":
        parser.error("--release-signed is only applicable to macOS")
    if args.platform == "linux-x64" and Path("/nix/store").exists():
        parser.error("run in a clean image without /nix/store")
    binary = args.binary.resolve()
    if args.platform == "linux-x64":
        check_static(binary)
    else:
        check_macos(binary, args.release_signed)
    root = Path(__file__).resolve().parent.parent
    with tempfile.TemporaryDirectory(prefix="tonk-cli-smoke-") as directory:
        temp = Path(directory)
        # Execute a writable copy, exactly as archive extraction would produce.
        executable = temp / "tonk"
        shutil.copy2(binary, executable)
        executable.chmod(0o755)
        env = {**os.environ, "HOME": str(temp), "XDG_CONFIG_HOME": str(temp / "config"),
               "XDG_DATA_HOME": str(temp / "data"), "TONK_UPDATE_STATE": str(temp / "update"),
               "DO_NOT_TRACK": "1", "TONK_NO_UPDATE_CHECK": "1", "npm_config_cache": str(temp / "npm-cache")}
        version = run(str(executable), "--version", env=env)
        if not version.startswith("tonk "):
            raise RuntimeError(f"unexpected version output: {version!r}")
        if args.expected_version and version != f"tonk {args.expected_version}":
            raise RuntimeError(f"artifact does not report release version {args.expected_version}: {version}")
        run(str(executable), "--help", env=env)
        check_npm(executable, root, temp, env, version, args.platform, version.removeprefix("tonk "))
        check_tls(executable, temp, env)
        print(f"PASS: {version}: {args.platform} artifact, direct startup, offline npm launcher, trusted/untrusted TLS")


if __name__ == "__main__":
    main()
