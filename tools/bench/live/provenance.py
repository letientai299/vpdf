"""Separate embedded build identity from the runtime checkout."""

import subprocess


def embedded(version):
    fields = dict(
        line.split(": ", 1) for line in version.splitlines() if ": " in line
    )
    if "Source revision" in fields:
        return dict(
            revision=fields["Source revision"]
            if fields["Source revision"] != "unknown"
            else None,
            dirty={"true": True, "false": False}.get(
                fields.get("Local changes")
            ),
            patch=fields.get("Source patch")
            if fields.get("Source patch") != "unknown"
            else None,
        )
    for line in version.splitlines():
        words = line.split()
        if len(words) == 4 and words[0] == "build-source":
            return dict(
                revision=None if words[1] == "unknown" else words[1],
                dirty={"true": True, "false": False}.get(words[2]),
                patch=None if words[3] == "unknown" else words[3],
            )
    return dict(revision=None, dirty=None, patch=None)


def checkout(path):
    def git(*args):
        try:
            result = subprocess.run(
                ["git", "-C", str(path), *args], capture_output=True, text=True
            )
        except OSError:
            return None
        return result.stdout if result.returncode == 0 else None

    revision = git("rev-parse", "HEAD")
    status = git("status", "--porcelain")
    return dict(
        runtime_checkout_revision=revision.strip() if revision else None,
        runtime_checkout_dirty=bool(status) if status is not None else None,
    )
