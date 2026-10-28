"""Reuse successful checks for matching local inputs."""

import hashlib
import json
import os
import platform
import shutil
import stat
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
    "VPDF_BUILD_RESOURCE_FDS",
    "VPDF_BUILD_REQUEST_ID",
    "VPDF_BUILD_ORIGIN",
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
    "pandoc",
    "qpdf",
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


def fingerprint(root, env, details=None, *, tools=()):
    details = {} if details is None else details

    def unavailable(reason):
        details["disabled"] = reason
        return None

    for condition, reason in [
        (env.get("CI") == "true", "ci"),
        (not env.get("VPDF_BUILD_WORKER"), "outside-worker"),
        (env.get("VPDF_BUILD_MODE") == "isolated", "isolated"),
        (env.get("VPDF_CACHE") != "off", "cache-policy"),
        (env.get("NEXTEST_RETRIES", "0") != "0", "test-retries"),
        (
            bool(
                env.get("NEXTEST_CONFIG_FILE")
                or env.get("NEXTEST_USER_CONFIG_FILE")
            ),
            "nextest-config",
        ),
    ]:
        if condition:
            return unavailable(reason)
    supported = {
        "VPDF_BUILD_WORKER",
        "VPDF_BUILD_WORKER_FD",
        "VPDF_BUILD_CACHE_ID",
        "VPDF_BUILD_CACHE_GIB",
        "VPDF_BUILD_MODE",
        "VPDF_CACHE",
        "VPDF_BENCH_EVENTS",
        "VPDF_CORPUS",
        "VPDF_VALIDATE_FORCE",
        "VPDF_BUILD_WORKERS",
        "VPDF_BUILD_RESOURCE_FDS",
        "VPDF_BUILD_REQUEST_ID",
        "VPDF_BUILD_ORIGIN",
    }
    if any(key.startswith("VPDF_") and key not in supported for key in env):
        return unavailable("custom-vpdf-input")
    if any(
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
    ):
        return unavailable("custom-compiler-input")
    try:
        home = root / env.get("HOME", Path.home())
        cargo_home = root / env.get("CARGO_HOME", home / ".cargo")
        if any(
            (directory / name).exists()
            for directory in [cargo_home, root / ".cargo"]
            for name in ["config", "config.toml"]
        ):
            return unavailable("cargo-config")
        config = root / env.get("XDG_CONFIG_HOME", home / ".config")
        if (root / ".config/nextest.toml").exists() or (
            config / "nextest/config.toml"
        ).exists():
            return unavailable("nextest-config")
        files = working_files(root, env)
        if any(mode == b"120000" for mode, _ in files.values()):
            return unavailable("source-symlink")
        digest = hashlib.sha256()
        components = {}

        def add(value, component):
            data = json.dumps(value, sort_keys=True).encode()
            data = len(data).to_bytes(8, "big") + data
            digest.update(data)
            components.setdefault(component, hashlib.sha256()).update(data)

        add([1, platform.platform(), platform.machine()], "platform")
        for name, (mode, content) in sorted(files.items()):
            add(
                [name, mode.decode(), hashlib.sha256(content).hexdigest()],
                "source",
            )
        for args, component in [
            (["rev-parse", "HEAD"], "revision"),
            (["show-ref", "--tags"], "tags"),
            (
                ["config", "--null", "--list", "--show-origin", "--includes"],
                "git-config",
            ),
        ]:
            result = subprocess.run(
                ["git", *args], cwd=root, env=env, capture_output=True
            )
            if result.returncode not in (
                {0, 1} if args[0] == "show-ref" else {0}
            ):
                return unavailable("git-input-unavailable")
            add(result.stdout.hex(), component)
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
            },
            "environment",
        )
        for name in dict.fromkeys((*TOOLS, *tools)):
            tool = shutil.which(name, path=environment.get("PATH"))
            if not tool:
                return unavailable("tool-unavailable")
            result = subprocess.run(
                [tool, "--version"],
                cwd=root,
                env=env,
                capture_output=True,
                timeout=10,
            )
            if result.returncode:
                return unavailable("tool-failed")
            executable = Path(tool).resolve()
            info = executable.stat()
            add([str(executable), info.st_size, info.st_mtime_ns], "tools")
            add((result.stdout + result.stderr).hex(), "tools")
        corpus = root / env.get("VPDF_CORPUS", "tmp/corpus")
        if corpus.is_symlink():
            return unavailable("corpus-symlink")
        add(str(corpus.absolute()), "corpus")
        for path in sorted(corpus.rglob("*")):
            if path.is_symlink():
                return unavailable("corpus-symlink")
            if path.is_file():
                add(
                    [str(path.relative_to(corpus)), digest_file(path)], "corpus"
                )
        details["components"] = {
            key: value.hexdigest() for key, value in components.items()
        }
        return digest.hexdigest()
    except (OSError, ValueError, subprocess.SubprocessError):
        return unavailable("input-unavailable")


def diagnose(root, env, scope, identity, details, reused):
    status = "hit" if reused else "miss"
    if identity is None:
        status = "disabled"
    elif env.get("VPDF_VALIDATE_FORCE") == "1":
        status = "forced"
    changed = []
    try:
        origin = env.get("VPDF_BUILD_ORIGIN", str(root))
        name = hashlib.sha256(f"{origin}:{scope}".encode()).hexdigest()
        path = cache_root(env, root) / "diagnostics" / f"{name}.json"
        previous = {}
        if path.exists():
            previous = json.loads(path.read_text())
        current = details.get("components", {})
        if isinstance(previous, dict) and previous and current:
            changed = sorted(
                key
                for key in previous.keys() | current.keys()
                if previous.get(key) != current.get(key)
            )
        if current:
            path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
            replace_file(
                path, json.dumps(current, sort_keys=True).encode(), 0o600
            )
    except (OSError, ValueError):
        pass
    request = env.get("VPDF_BUILD_REQUEST_ID", "local")
    reason = details.get(
        "disabled", "identity" if changed else "no-matching-receipt"
    )
    print(
        f"Validation receipt: request={request} scope={scope} status={status} "
        f"reason={reason if not reused else 'matching-receipt'} "
        f"changed={','.join(changed) or 'none'}",
        flush=True,
    )


def receipt_path(root, env, identity, scope):
    key = hashlib.sha256(f"{identity}:{scope}".encode()).hexdigest()
    return cache_root(env, root) / "receipts" / f"{key}.json"


def output_state(root, paths):
    state = {}
    for name in paths:
        path = root / name
        info = path.lstat()
        if not stat.S_ISREG(info.st_mode):
            raise ValueError("Validation output must be a regular file")
        state[str(name)] = [stat.S_IMODE(info.st_mode), digest_file(path)]
    return state


def completed(root, env, identity, scope, artifacts=()):
    if identity is None:
        return False
    try:
        data = json.loads(receipt_path(root, env, identity, scope).read_text())
        expected = {"identity": identity, "scope": scope, "passed": True}
        if artifacts:
            expected["outputs"] = output_state(root, artifacts)
        return data == expected
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


def run_check(
    root, env, scope, action, covered_by=None, *, artifacts=(), tools=()
):
    started = time.monotonic()
    details = {}
    identity = fingerprint(root, env, details, tools=tools)
    candidates = [scope, *([covered_by] if covered_by else [])]
    reused = env.get("VPDF_VALIDATE_FORCE") != "1" and any(
        completed(root, env, identity, candidate, artifacts)
        for candidate in candidates
    )
    diagnose(root, env, scope, identity, details, reused)
    if reused:
        print(f"Validation reused: {scope}", flush=True)
        return True
    if identity is not None:
        if scope == "test" or (
            env.get("VPDF_VALIDATE_FORCE") == "1" and test_scope(scope)
        ):
            invalidate_tests(root, env, identity)
        for candidate in candidates:
            receipt_path(root, env, identity, candidate).unlink(missing_ok=True)
    action()
    if identity is not None and fingerprint(root, env, tools=tools) == identity:
        data = {"identity": identity, "scope": scope, "passed": True}
        if artifacts:
            data["outputs"] = output_state(root, artifacts)
        path = receipt_path(root, env, identity, scope)
        path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        replace_file(
            path,
            json.dumps(data).encode(),
            0o600,
        )
    print(
        f"Validation {scope}: {time.monotonic() - started:.3f}s",
        flush=True,
    )
    return False
