"""Reuse successful checks for matching local inputs."""

import hashlib
import json
import os
import platform
import shutil
import subprocess
import time
from pathlib import Path

from build_cache import cache_root
from build_worker import replace_file, working_files

# Execution metadata does not affect checked inputs.
TRANSIENT = {
    "PWD",
    "OLDPWD",
    "SHLVL",
    "_",
    "GIT_PREFIX",
    "CODEX_TOOL_CALL_ID",
    "MISE_TASK_NAME",
    "MISE_TASK_DIR",
    "MISE_TASK_FILE",
    "MISE_TASK_OUTPUT",
    "MISE_TASK_COLOR",
    "MISE_TASK_TIMINGS",
    "MISE_TASK_PGID_MANAGED",
    "MISE_ORIGINAL_CWD",
    "MISE_PROJECT_ROOT",
    "MISE_CONFIG_ROOT",
    "MISE_TRUSTED_CONFIG_PATHS",
    "VPDF_BUILD_WORKER_FD",
    "VPDF_VALIDATE_FORCE",
}
TOOLS = (
    "rustc",
    "cargo",
    "cargo-nextest",
    "rustfmt",
    "clippy-driver",
    "python3",
    "mise",
    "ruff",
    "rumdl",
    "taplo",
    "typst",
    "git",
    "cc",
    "make",
)


def digest_file(path):
    with path.open("rb") as stream:
        digest = hashlib.sha256()
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
        return digest.hexdigest()


def fingerprint(root, env):
    if (
        env.get("CI") == "true"
        or not env.get("VPDF_BUILD_WORKER")
        or env.get("VPDF_BUILD_MODE") == "isolated"
        or env.get("VPDF_CACHE") != "off"
        or env.get("NEXTEST_RETRIES", "0") != "0"
        or env.get("NEXTEST_CONFIG_FILE")
        or env.get("NEXTEST_USER_CONFIG_FILE")
        or any(
            key.startswith("VPDF_")
            and key
            not in {
                "VPDF_BUILD_WORKER",
                "VPDF_BUILD_WORKER_FD",
                "VPDF_BUILD_CACHE_ID",
                "VPDF_BUILD_CACHE_GIB",
                "VPDF_BUILD_MODE",
                "VPDF_CACHE",
                "VPDF_CORPUS",
                "VPDF_VALIDATE_FORCE",
            }
            for key in env
        )
        or any(
            key.startswith(("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS"))
            or key
            in {
                "RUSTC_WRAPPER",
                "RUSTC_WORKSPACE_WRAPPER",
                "CC",
                "CXX",
                "CFLAGS",
                "CXXFLAGS",
                "LDFLAGS",
                "TESSDATA_PREFIX",
                "LIBCLANG_PATH",
                "CARGO_BUILD_RUSTC",
                "CARGO_BUILD_RUSTDOC",
            }
            for key in env
        )
    ):
        return None
    try:
        home = root / env.get("HOME", Path.home())
        cargo_home = root / env.get("CARGO_HOME", home / ".cargo")
        if any(
            (directory / name).exists()
            for directory in [cargo_home, root / ".cargo"]
            for name in ["config", "config.toml"]
        ):
            return None
        if (root / ".config/nextest.toml").exists():
            return None
        config = root / env.get("XDG_CONFIG_HOME", home / ".config")
        if (config / "nextest/config.toml").exists():
            return None
        files = working_files(root, env)
        if any(mode == b"120000" for mode, _ in files.values()):
            return None
        digest = hashlib.sha256()

        def add(value):
            data = json.dumps(value, sort_keys=True).encode()
            digest.update(len(data).to_bytes(8, "big"))
            digest.update(data)

        add([1, platform.platform(), platform.machine()])
        for name, (mode, content) in sorted(files.items()):
            add([name, mode.decode(), hashlib.sha256(content).hexdigest()])
        for args in [
            ["rev-parse", "HEAD"],
            ["show-ref", "--tags"],
            ["config", "--null", "--list", "--show-origin", "--includes"],
        ]:
            result = subprocess.run(
                ["git", *args], cwd=root, env=env, capture_output=True
            )
            if result.returncode not in (
                {0, 1} if args[0] == "show-ref" else {0}
            ):
                return None
            add(result.stdout.hex())
        environment = dict(env)
        environment["PATH"] = os.pathsep.join(
            dict.fromkeys(
                str(Path(part).resolve())
                for part in env.get("PATH", "").split(os.pathsep)
                if Path(part).resolve() != root / "scripts/tools"
            )
        )
        add(
            {
                key: value
                for key, value in environment.items()
                if key not in TRANSIENT and not key.startswith("__MISE_")
            }
        )
        for name in TOOLS:
            tool = shutil.which(name, path=environment.get("PATH"))
            if not tool:
                return None
            result = subprocess.run(
                [tool, "--version"],
                cwd=root,
                env=env,
                capture_output=True,
                timeout=10,
            )
            if result.returncode:
                return None
            executable = Path(tool).resolve()
            info = executable.stat()
            add([str(executable), info.st_size, info.st_mtime_ns])
            add((result.stdout + result.stderr).hex())
        corpus = root / env.get("VPDF_CORPUS", "tmp/corpus")
        if corpus.is_symlink():
            return None
        add(str(corpus.resolve()))
        for path in sorted(corpus.rglob("*")):
            if path.is_symlink():
                return None
            if path.is_file():
                add([str(path.relative_to(corpus)), digest_file(path)])
        return digest.hexdigest()
    except (OSError, ValueError, subprocess.SubprocessError):
        return None


def receipt_path(root, env, identity, scope):
    key = hashlib.sha256(f"{identity}:{scope}".encode()).hexdigest()
    return cache_root(env, root) / "receipts" / f"{key}.json"


def completed(root, env, identity, scope):
    if identity is None:
        return False
    try:
        data = json.loads(receipt_path(root, env, identity, scope).read_text())
        return data == {"identity": identity, "scope": scope, "passed": True}
    except (OSError, ValueError):
        return False


def test_scope(scope):
    return isinstance(scope, str) and (
        scope == "test" or scope.startswith(("affected:", "test:"))
    )


def invalidate_tests(root, env, identity):
    directory = receipt_path(root, env, identity, "test").parent
    for path in directory.glob("*.json"):
        try:
            data = json.loads(path.read_text())
        except (OSError, ValueError):
            continue
        if (
            isinstance(data, dict)
            and data.get("identity") == identity
            and test_scope(data.get("scope"))
        ):
            path.unlink(missing_ok=True)


def run_check(root, env, scope, action, covered_by=None):
    started = time.monotonic()
    identity = fingerprint(root, env)
    candidates = [scope, *([covered_by] if covered_by else [])]
    if env.get("VPDF_VALIDATE_FORCE") != "1" and any(
        completed(root, env, identity, candidate) for candidate in candidates
    ):
        print(f"Validation reused: {scope}", flush=True)
        return
    if identity is not None:
        if scope == "test" or (
            env.get("VPDF_VALIDATE_FORCE") == "1" and test_scope(scope)
        ):
            invalidate_tests(root, env, identity)
        for candidate in candidates:
            receipt_path(root, env, identity, candidate).unlink(missing_ok=True)
    action()
    if identity is not None and fingerprint(root, env) == identity:
        path = receipt_path(root, env, identity, scope)
        path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        replace_file(
            path,
            json.dumps(
                {"identity": identity, "scope": scope, "passed": True}
            ).encode(),
            0o600,
        )
    print(
        f"Validation {scope}: {time.monotonic() - started:.3f}s",
        flush=True,
    )
