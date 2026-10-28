"""Run the non-publishing CI contract."""

import os
import platform
import shutil
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path

from tools.build.receipts import digest_file as checksum
from tools.build.receipts import run_check
from tools.build.worker import working_files
from tools.check.hooks import check_lint, execute


def archive_paths(root, env):
    system = {"Darwin": "macos", "Linux": "linux"}[platform.system()]
    machine = {"arm64": "aarch64"}.get(platform.machine(), platform.machine())
    target = Path(env.get("CARGO_TARGET_DIR", "target"))
    directory = root / target / "dist"
    archive = directory / f"vpdf-{system}-{machine}.tar.gz"
    return archive, archive.with_suffix(".gz.sha256")


def verify_archive(archive, digest, root, env):
    expected = f"{checksum(archive)}  {archive.name}\n"
    if digest.read_text() != expected:
        raise RuntimeError("Archive checksum mismatch")
    with tarfile.open(archive, "r:gz") as packed:
        members = packed.getmembers()
        if (
            len(members) != 1
            or members[0].name != "vpdf"
            or not members[0].isfile()
            or not members[0].mode & 0o111
        ):
            raise RuntimeError("Archive executable missing or invalid")
        with tempfile.TemporaryDirectory(prefix="vpdf-archive-") as temporary:
            binary = Path(temporary) / "vpdf"
            with (
                packed.extractfile(members[0]) as source,
                binary.open("wb") as out,
            ):
                shutil.copyfileobj(source, out)
            binary.chmod(members[0].mode & 0o777)
            execute([str(binary), "--version"], root, env)


def archive(root, env):
    destination, digest = archive_paths(root, env)
    with tempfile.TemporaryDirectory(dir=destination.parent) as temporary:
        staged = Path(temporary) / destination.name
        staged_digest = Path(temporary) / digest.name
        with tarfile.open(staged, "w:gz") as packed:
            packed.add(destination.parent / "vpdf", arcname="vpdf")
        staged_digest.write_text(f"{checksum(staged)}  {staged.name}\n")
        verify_archive(staged, staged_digest, root, env)
        os.replace(staged, destination)
        os.replace(staged_digest, digest)
    print(f"Verified archive: {destination}", flush=True)


def pack(root, env):
    execute(["mise", "run", "release"], root, env)
    archive(root, env)


def checks(root, env):
    print(f"CI platform: {platform.system()} {platform.machine()}", flush=True)
    before = working_files(root, env)
    execute(["mise", "run", "corpus"], root, env)
    if run_check(root, env, "lint", lambda: check_lint(root, env)):
        execute(["mise", "run", "help:build"], root, env)
    for task in ["test", "test:events"]:
        run_check(
            root,
            env,
            task,
            lambda task=task: execute(["mise", "run", task], root, env),
        )
    packed, digest = archive_paths(root, env)
    run_check(
        root,
        env,
        "ci:release",
        lambda: pack(root, env),
        artifacts=(
            packed.parent.parent / "release/vpdf",
            packed.parent / "vpdf",
            packed,
            digest,
        ),
        tools=("upx",) if platform.system() == "Linux" else (),
    )
    if working_files(root, env) != before:
        raise RuntimeError("Validation changed source files")
    print("CI checks passed", flush=True)


if __name__ == "__main__":
    try:
        checks(Path.cwd(), dict(os.environ))
    except (
        OSError,
        ValueError,
        KeyError,
        RuntimeError,
        subprocess.CalledProcessError,
    ) as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
