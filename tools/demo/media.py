"""Plan and publish reviewed demo attachments."""

import argparse
import fcntl
import json
import re
import shutil
import subprocess
import tempfile
import time
import uuid
from datetime import UTC, datetime
from pathlib import Path
from urllib.parse import urlsplit

from tools.demo.capture import CACHE, ROOT, digest, verify_output

STAGE_START = "<!-- vpdf-media-stage:start -->"
STAGE_END = "<!-- vpdf-media-stage:end -->"
LOCK = "tags/vpdf-media-publisher-lock"
URL = re.compile(r"https://github\.com/user-attachments/assets/[0-9a-f-]+")


def atomic_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    encoded = json.dumps(value, indent=2) + "\n"
    with tempfile.NamedTemporaryFile(
        mode="w", dir=path.parent, delete=False
    ) as file:
        temporary = Path(file.name)
        try:
            file.write(encoded)
            file.flush()
            file.close()
            temporary.replace(path)
        finally:
            temporary.unlink(missing_ok=True)
    return encoded


def command(args, **kwargs):
    return subprocess.check_output(
        args, text=True, timeout=120, **kwargs
    ).strip()


def canonical(url):
    parsed = urlsplit(url)
    if not URL.fullmatch(url) or parsed.query or parsed.fragment:
        raise RuntimeError("Noncanonical attachment URL")
    return url


def metadata(path, kind):
    if kind == "video":
        info = json.loads(
            command(
                [
                    "ffprobe",
                    "-v",
                    "error",
                    "-show_streams",
                    "-show_format",
                    "-of",
                    "json",
                    str(path),
                ]
            )
        )
        stream = next(s for s in info["streams"] if s["codec_type"] == "video")
        if stream["codec_name"] != "h264":
            raise RuntimeError("Video must use H.264")
        return {
            "dimensions": [stream["width"], stream["height"]],
            "codec": stream["codec_name"],
            "duration": float(info["format"]["duration"]),
            "frame_rate": stream["avg_frame_rate"],
            "pixel_format": stream["pix_fmt"],
        }
    values = command(
        ["magick", "identify", "-format", "%w %h", str(path)]
    ).split()
    return {"dimensions": [int(value) for value in values]}


def verify_remote(record, kind, destination=None):
    canonical(record["url"])
    with tempfile.TemporaryDirectory(prefix="vpdf-media-check-") as directory:
        path = Path(directory) / (
            "asset.mp4" if kind == "video" else "asset.jpg"
        )
        content_type = command(
            [
                "curl",
                "-q",
                "--fail",
                "--silent",
                "--show-error",
                "--location",
                "--proto",
                "=https",
                "--proto-redir",
                "=https",
                "--retry",
                "4",
                "--retry-all-errors",
                "--retry-delay",
                "2",
                "--retry-max-time",
                "20",
                "--max-time",
                "15",
                "--max-filesize",
                "10000000",
                "--output",
                str(path),
                "--write-out",
                "%{content_type}",
                record["url"],
            ]
        )
        prefix = "video/" if kind == "video" else "image/"
        if (
            not content_type.startswith(prefix)
            or digest(path) != record["sha256"]
        ):
            raise RuntimeError("Remote attachment content mismatch")
        if metadata(path, kind) != record["metadata"]:
            raise RuntimeError("Remote attachment metadata mismatch")
        if destination is not None:
            archive_file(path, destination, record["sha256"])


def archive_file(source, destination, expected):
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.exists():
        if digest(destination) != expected:
            raise RuntimeError("Recovery archive hash mismatch")
        return
    with tempfile.NamedTemporaryFile(
        dir=destination.parent, delete=False
    ) as file:
        staged = Path(file.name)
        try:
            file.close()
            shutil.copyfile(source, staged)
            if digest(staged) != expected:
                raise RuntimeError("Recovery source changed during copy")
            staged.replace(destination)
        finally:
            staged.unlink(missing_ok=True)


def find_comments(comments, owner):
    found = {}
    for comment in comments:
        markers = re.findall(
            r"^<!-- vpdf-media:([a-z0-9-]+) -->$", comment["body"], re.M
        )
        if len(markers) > 1:
            raise RuntimeError("Media comment has multiple markers")
        for marker in markers:
            if marker in found:
                raise RuntimeError(f"Duplicate media marker: {marker}")
            if comment["user"]["login"] != owner:
                raise RuntimeError(f"Unexpected media owner: {marker}")
            found[marker] = comment
    return found


def comment_record(comment, asset):
    matches = re.findall(
        r"^<!-- vpdf-media-record:(.+) -->$", comment["body"], re.M
    )
    if len(matches) != 1:
        raise RuntimeError("Missing unique comment record")
    record = json.loads(matches[0])
    if record["id"] != asset["id"] or record["kind"] != asset["kind"]:
        raise RuntimeError("Comment asset identity mismatch")
    canonical(record["url"])
    if record["url"] not in comment["body"].split("\n\n"):
        if f"]({record['url']})" not in comment["body"]:
            raise RuntimeError("Comment media URL mismatch")
    return {**record, "comment_id": comment["id"]}


def same_media(record, scene, info):
    return (
        record is not None
        and record["sha256"] == scene["sha256"]
        and record["metadata"] == info
    )


def render_comment(asset, record):
    visible = f"![{asset['title']}]({record['url']})"
    if asset["kind"] == "video":
        visible = record["url"]
    history = (
        "\n".join(
            f"- `{item['sha256']}`: {item['url']}"
            for item in record.get("history", [])
        )
        or "None."
    )
    return (
        f"<!-- vpdf-media:{asset['id']} -->\n\n"
        f"### {asset['title']}\n\n{visible}\n\n"
        f"SHA-256: `{record['sha256']}`\n\n"
        f"Capture: `{record['capture']}`\n\n{asset['attribution']}\n\n"
        f"#### Previous attachments\n\n{history}\n\n"
        f"<!-- vpdf-media-record:{json.dumps(record, sort_keys=True)} -->"
    )


def replace_references(text, replacements):
    for reference, url in replacements.items():
        canonical(url)
        pattern = re.compile(
            rf"^(\[{re.escape(reference)}\]: )\S+[ \t]*$", re.M
        )
        if len(pattern.findall(text)) != 1:
            raise RuntimeError(f"Missing unique media reference: {reference}")
        text = pattern.sub(lambda match: match[1] + url, text)
    return text


def replace_documents(changes):
    staged = []
    installed = []
    try:
        for path, original, updated in changes:
            directory = tempfile.TemporaryDirectory(
                prefix=".media-", dir=path.parent
            )
            folder = Path(directory.name)
            candidate, backup = folder / "updated", folder / "previous"
            candidate.write_text(updated)
            candidate.chmod(path.stat().st_mode)
            backup.write_text(original)
            backup.chmod(path.stat().st_mode)
            staged.append(
                (directory, path, original, updated, candidate, backup)
            )
        for _, path, original, _, _, _ in staged:
            if path.read_text() != original:
                raise RuntimeError("Media consumer changed before replacement")
        for _, path, original, updated, candidate, backup in staged:
            if path.read_text() != original:
                raise RuntimeError("Media consumer changed during replacement")
            candidate.replace(path)
            installed.append((path, updated, backup))
    except BaseException:
        for path, updated, backup in reversed(installed):
            if path.read_text() != updated:
                raise RuntimeError("Media consumer changed during recovery")
            backup.replace(path)
        raise
    finally:
        for directory, *_ in staged:
            directory.cleanup()


def stage_parts(body):
    if STAGE_START not in body and STAGE_END not in body:
        return body + "\n\n", "", "\n"
    if body.count(STAGE_START) != 1 or body.count(STAGE_END) != 1:
        raise RuntimeError("Invalid upload staging markers")
    before, rest = body.split(STAGE_START)
    stage, after = rest.split(STAGE_END)
    return before, stage, after


class GitHub:
    def __init__(self, config):
        self.config = config
        self.base = f"repos/{config['repository']}"
        self.issue = f"{self.base}/issues/{config['issue']}"

    def api(self, path, method="GET", payload=None):
        args = ["gh", "api", path, "--method", method]
        kwargs = {}
        if payload is not None:
            args += ["--input", "-"]
            kwargs["input"] = json.dumps(payload)
        for attempt in range(3):
            try:
                result = command(args, **kwargs)
                return json.loads(result) if result else None
            except (subprocess.CalledProcessError, subprocess.TimeoutExpired):
                if method != "GET" or attempt == 2:
                    raise
                time.sleep(2**attempt)

    def comments(self):
        result = []
        page = 1
        while True:
            batch = self.api(f"{self.issue}/comments?per_page=100&page={page}")
            result.extend(batch)
            if len(batch) < 100:
                return result
            page += 1

    def preflight(self):
        if "--attach" not in command(["gh", "issue", "edit", "--help"]):
            raise RuntimeError("GitHub CLI lacks attachment support")
        repo = self.api(self.base)
        if repo["private"] or not repo["permissions"]["push"]:
            raise RuntimeError("Public repository push access required")
        if self.api("user")["login"] != self.config["owner"]:
            raise RuntimeError("GitHub publication account mismatch")
        issue = self.api(self.issue)
        if (
            issue["state"] != "closed"
            or issue["title"] != "Documentation media"
        ):
            raise RuntimeError("Configured media issue must remain closed")
        if "pull_request" in issue:
            raise RuntimeError("Media host must be an issue")

    def matching_lock(self):
        refs = self.api(f"{self.base}/git/matching-refs/{LOCK}")
        return next((ref for ref in refs if ref["ref"] == f"refs/{LOCK}"), None)

    def acquire(self, journal, save):
        if journal.get("lock_sha"):
            lock = self.matching_lock()
            if lock and lock["object"]["sha"] == journal["lock_sha"]:
                return
            if lock:
                raise RuntimeError("Remote publisher lock changed")
        elif self.matching_lock():
            raise RuntimeError("Remote publisher lock already exists")
        if not journal.get("lock_sha"):
            repo = self.api(self.base)
            branch = self.api(
                f"{self.base}/git/ref/heads/{repo['default_branch']}"
            )
            tag = self.api(
                f"{self.base}/git/tags",
                "POST",
                {
                    "tag": "vpdf-media-lock",
                    "message": journal["operation"],
                    "object": branch["object"]["sha"],
                    "type": "commit",
                },
            )
            journal["lock_sha"] = tag["sha"]
            save()
        self.api(
            f"{self.base}/git/refs",
            "POST",
            {
                "ref": f"refs/{LOCK}",
                "sha": journal["lock_sha"],
            },
        )

    def release(self, journal, save):
        lock = self.matching_lock()
        if lock and lock["object"]["sha"] != journal["lock_sha"]:
            raise RuntimeError("Remote publisher lock changed")
        if lock:
            self.api(f"{self.base}/git/refs/{LOCK}", "DELETE")
        journal.pop("lock_sha", None)
        save()

    def upload(self, asset, path, pending, save):
        body = self.api(self.issue)["body"] or ""
        before, stage, after = stage_parts(body)
        token = f"operation={pending['operation']} asset={asset['id']} sha256={pending['sha256']}"
        if token in stage:
            urls = URL.findall(stage)
            if len(urls) == 1:
                return urls[0]
            if pending.get("upload_started"):
                raise RuntimeError(
                    "Ambiguous upload; inspect staging before retry"
                )
        elif stage.strip():
            raise RuntimeError("Another upload occupies staging")
        if pending.get("upload_started"):
            raise RuntimeError("Upload staging changed; inspect journal")
        label = "" if asset["kind"] == "video" else "asset"
        new_body = (
            before
            + STAGE_START
            + f"\n{token}\n\n![{label}]({path})\n"
            + STAGE_END
            + after
        )
        if (self.api(self.issue)["body"] or "") != body:
            raise RuntimeError("Issue changed before upload")
        pending["upload_started"] = True
        save()
        with tempfile.TemporaryDirectory(
            prefix="vpdf-media-upload-"
        ) as directory:
            body_file = Path(directory) / "body.md"
            body_file.write_text(new_body)
            command(
                [
                    "gh",
                    "issue",
                    "edit",
                    str(self.config["issue"]),
                    "--repo",
                    self.config["repository"],
                    "--body-file",
                    str(body_file),
                    "--attach",
                    str(path),
                ]
            )
        updated = self.api(self.issue)["body"] or ""
        current_before, current_stage, current_after = stage_parts(updated)
        if (current_before, current_after) != (
            before,
            after,
        ) or token not in current_stage:
            raise RuntimeError("Issue changed during upload")
        urls = URL.findall(current_stage)
        if len(urls) != 1:
            raise RuntimeError("Upload lacks unique attachment URL")
        command(
            [
                "curl",
                "-q",
                "--fail",
                "--silent",
                "--show-error",
                "--location",
                "--proto",
                "=https",
                "--proto-redir",
                "=https",
                "--max-time",
                "20",
                "--output",
                "/dev/null",
                f"https://github.com/{self.config['repository']}/issues/{self.config['issue']}",
            ]
        )
        return urls[0]

    def clear_stage(self, pending):
        body = self.api(self.issue)["body"] or ""
        before, stage, after = stage_parts(body)
        if not stage.strip():
            return
        if f"operation={pending['operation']} " not in stage:
            raise RuntimeError("Upload staging ownership changed")
        if (self.api(self.issue)["body"] or "") != body:
            raise RuntimeError("Issue changed before staging cleanup")
        self.api(
            self.issue,
            "PATCH",
            {"body": before + STAGE_START + "\n" + STAGE_END + after},
        )


class Publisher:
    def __init__(self, args):
        self.args = args
        self.records_source = args.records.read_text()
        self.config = json.loads(self.records_source)
        self.manifest = json.loads(args.manifest.read_text())
        self.github = GitHub(self.config)
        self.journal_path = args.cache / "journal.json"
        self.journal = (
            json.loads(self.journal_path.read_text())
            if self.journal_path.exists()
            else {"operation": str(uuid.uuid4()), "pending": {}}
        )
        identity = {
            "repository": self.config["repository"],
            "issue": self.config["issue"],
        }
        if self.journal.get("target", identity) != identity:
            raise RuntimeError("Journal publication target mismatch")
        self.journal["target"] = identity

    def save(self):
        atomic_json(self.journal_path, self.journal)

    def local_asset(self, asset, allow_missing=False):
        scene = self.manifest["scenes"][asset["scene"]]
        if scene["capture"] not in self.manifest["captures"]:
            raise RuntimeError("Missing scene capture identity")
        if Path(scene["image"]).name != scene["image"]:
            raise RuntimeError("Capture filename escapes media directory")
        path = self.args.media / scene["image"]
        if not path.exists():
            path = ROOT / "docs/assets" / scene["image"]
        if not path.exists():
            path = (
                self.args.cache / "archive" / scene["sha256"] / scene["image"]
            )
        if not path.is_file():
            record = asset.get("publication")
            if (
                allow_missing
                and record
                and record["sha256"] == scene["sha256"]
                and record["capture"] == scene["capture"]
            ):
                expected = scene.get(
                    "dimensions",
                    self.manifest["captures"][scene["capture"]]["dimensions"],
                )
                if record["metadata"]["dimensions"] != expected:
                    raise RuntimeError("Publication dimensions mismatch")
                return None, scene, record["metadata"]
            raise RuntimeError(f"Capture file missing: {asset['id']}")
        if digest(path) != scene["sha256"]:
            raise RuntimeError(f"Capture hash mismatch: {asset['id']}")
        if (
            path.stat().st_size != scene["bytes"]
            or path.stat().st_size >= 10_000_000
        ):
            raise RuntimeError("Capture size mismatch or upload limit exceeded")
        info = metadata(path, asset["kind"])
        expected = scene.get(
            "dimensions",
            self.manifest["captures"][scene["capture"]]["dimensions"],
        )
        if info["dimensions"] != expected:
            raise RuntimeError("Capture dimensions mismatch")
        return path, scene, info

    def selected(self):
        assets = self.config["assets"]
        if self.args.asset and set(self.args.asset) - assets.keys():
            raise RuntimeError("Unknown publication asset")
        selected = [
            dict(assets[key], id=key) for key in (self.args.asset or assets)
        ]
        for asset in selected:
            if not re.fullmatch(r"[a-z0-9-]+", asset["id"]) or asset[
                "kind"
            ] not in {"image", "video"}:
                raise RuntimeError("Invalid publication asset identity")
        return selected

    def plan(self):
        found = {}
        if self.config["issue"]:
            self.github.preflight()
            found = find_comments(self.github.comments(), self.config["owner"])
        for asset in self.selected():
            _, scene, info = self.local_asset(asset, allow_missing=True)
            comment = found.get(asset["id"])
            old = comment_record(comment, asset) if comment else None
            reusable = same_media(old, scene, info)
            unchanged = reusable and old["capture"] == scene["capture"]
            if reusable:
                verify_remote(old, asset["kind"])
            action = (
                "verify/skip upload"
                if unchanged
                else "reuse attachment"
                if reusable
                else "upload"
            )
            target = (
                f"reuse comment {comment['id']}"
                if unchanged
                else f"PATCH comment {comment['id']}"
                if comment
                else "POST comment"
            )
            print(
                f"{asset['id']}: {action}; {target}; replace [{asset['reference']}] (dry run)"
            )
        if not self.config["issue"]:
            print("Configure the dedicated closed issue before applying.")

    def publish_asset(self, asset):
        path, scene, info = self.local_asset(asset, allow_missing=True)
        archive = self.args.cache / "archive" / scene["sha256"] / scene["image"]
        if path is None:
            verify_remote(asset["publication"], asset["kind"], archive)
        else:
            archive_file(path, archive, scene["sha256"])
        found = find_comments(self.github.comments(), self.config["owner"])
        comment = found.get(asset["id"])
        old = comment_record(comment, asset) if comment else None
        reusable = same_media(old, scene, info)
        pending = self.journal["pending"].get(asset["id"])
        if pending and pending["sha256"] != scene["sha256"]:
            raise RuntimeError("Pending publication uses another capture")
        if reusable and old["capture"] == scene["capture"]:
            record = old
        else:
            if not pending:
                pending = {
                    "operation": str(uuid.uuid4()),
                    "sha256": scene["sha256"],
                    "previous_body": comment["body"] if comment else None,
                }
                self.journal["pending"][asset["id"]] = pending
                self.save()
            if pending.get("comment_started"):
                raise RuntimeError(
                    "Ambiguous comment write; inspect remote before retry"
                )
            if "url" not in pending:
                pending["url"] = (
                    old["url"]
                    if reusable
                    else self.github.upload(asset, archive, pending, self.save)
                )
                self.save()
            history = list(old.get("history", [])) if old else []
            if old:
                history.append(
                    {
                        key: old[key]
                        for key in (
                            "url",
                            "sha256",
                            "capture",
                            "published_at",
                            "metadata",
                        )
                    }
                )
            record = {
                "id": asset["id"],
                "kind": asset["kind"],
                "sha256": scene["sha256"],
                "capture": scene["capture"],
                "url": pending["url"],
                "metadata": info,
                "published_at": datetime.now(UTC).isoformat(),
                "history": history,
            }
            verify_remote(record, asset["kind"])
            current = find_comments(
                self.github.comments(), self.config["owner"]
            ).get(asset["id"])
            if (current["body"] if current else None) != pending[
                "previous_body"
            ]:
                raise RuntimeError("Media comment changed before update")
            pending["comment_started"] = True
            self.save()
            body = render_comment(asset, record)
            if current:
                endpoint = f"{self.github.base}/issues/comments/{current['id']}"
                response = self.github.api(endpoint, "PATCH", {"body": body})
            else:
                response = self.github.api(
                    f"{self.github.issue}/comments", "POST", {"body": body}
                )
            record["comment_id"] = response["id"]
        endpoint = f"{self.github.base}/issues/comments/{record['comment_id']}"
        durable = self.github.api(endpoint)
        if durable["user"]["login"] != self.config["owner"]:
            raise RuntimeError("Comment readback owner mismatch")
        checked = find_comments([durable], self.config["owner"])
        if (
            asset["id"] not in checked
            or comment_record(durable, asset) != record
        ):
            raise RuntimeError("Comment readback mismatch")
        verify_remote(record, asset["kind"])
        if self.args.records.read_text() != self.records_source:
            raise RuntimeError("Media catalog changed during publication")
        asset_record = self.config["assets"][asset["id"]]
        if asset_record.get("publication") != record:
            asset_record["publication"] = record
            self.records_source = atomic_json(self.args.records, self.config)
        if pending:
            self.github.clear_stage(pending)
            self.journal["pending"].pop(asset["id"])
            self.save()
        self.update_docs(asset, record)
        print(f"{asset['id']}: verified comment {record['comment_id']}")

    def document_changes(self, asset, record):
        consumers = [(self.args.gallery, asset["reference"])]
        for consumer in asset.get("consumers", []):
            path = (ROOT / consumer["path"]).resolve()
            if not path.is_relative_to(ROOT):
                raise RuntimeError("Media consumer escapes repository")
            consumers.append((path, consumer["reference"]))
        changes = []
        for path, reference in consumers:
            original = path.read_text()
            updated = replace_references(original, {reference: record["url"]})
            if updated != original:
                changes.append((path, original, updated))
        return changes

    def update_docs(self, asset, record):
        replace_documents(self.document_changes(asset, record))

    def apply(self):
        if not self.config["issue"]:
            raise RuntimeError("Dedicated closed media issue is not configured")
        self.github.preflight()
        self.save()
        self.github.acquire(self.journal, self.save)
        try:
            for asset in self.selected():
                self.publish_asset(asset)
            self.github.preflight()
        except BaseException:
            print(
                "Publication interrupted; retain journal and remote lock for recovery."
            )
            raise
        self.github.release(self.journal, self.save)

    def check(self):
        text = self.args.gallery.read_text()
        if not self.args.asset:
            image_refs = set(re.findall(r"!\[[^\]]*\]\[([^\]]+)\]", text))
            managed = {
                asset["reference"] for asset in self.config["assets"].values()
            }
            if image_refs - managed:
                raise RuntimeError(
                    "Gallery images lack publication catalog entries"
                )
        for asset in self.selected():
            _, scene, info = self.local_asset(asset, allow_missing=True)
            record = asset.get("publication")
            if self.args.remote and not record:
                raise RuntimeError("Asset has no publication record")
            if record:
                scene = self.manifest["scenes"][asset["scene"]]
                if (
                    record["sha256"] != scene["sha256"]
                    or record["capture"] != scene["capture"]
                ):
                    raise RuntimeError("Publication capture identity mismatch")
                if (
                    record["id"] != asset["id"]
                    or record["kind"] != asset["kind"]
                    or record["metadata"] != info
                ):
                    raise RuntimeError("Publication asset metadata mismatch")
                if self.document_changes(asset, record):
                    raise RuntimeError("Managed publication reference mismatch")
                if self.args.remote:
                    self.github.preflight()
                    found = find_comments(
                        self.github.comments(), self.config["owner"]
                    )
                    if comment_record(found[asset["id"]], asset) != record:
                        raise RuntimeError("Publication comment mismatch")
                    verify_remote(record, asset["kind"])
                    for previous in record["history"]:
                        verify_remote(
                            {
                                **previous,
                                "metadata": previous["metadata"],
                            },
                            asset["kind"],
                        )
            print(f"{asset['id']}: provenance checked")


def parse_args(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--apply",
        action="store_true",
        help="Perform remote writes and update gallery references",
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="Check local provenance and publication references",
    )
    parser.add_argument(
        "--remote",
        action="store_true",
        help="Verify anonymous current and historical attachments",
    )
    parser.add_argument(
        "--asset",
        action="append",
        help="Logical asset ID; repeat to select several",
    )
    parser.add_argument(
        "--records", type=Path, default=ROOT / "docs/assets/media.json"
    )
    parser.add_argument(
        "--manifest", type=Path, default=ROOT / "docs/assets/capture.json"
    )
    parser.add_argument("--gallery", type=Path, default=ROOT / "docs/demos.md")
    parser.add_argument("--media", type=Path, default=CACHE / "encoded")
    parser.add_argument("--cache", type=Path, default=CACHE)
    args = parser.parse_args(argv)
    if args.apply and args.check:
        parser.error("--apply and --check are mutually exclusive")
    return args


def main():
    args = parse_args()
    verify_output(args.cache)
    args.cache.mkdir(parents=True, exist_ok=True)
    with (args.cache / "publisher.lock").open("a") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            raise RuntimeError("Local publisher already running") from error
        publisher = Publisher(args)
        action = (
            publisher.check
            if args.check
            else publisher.apply
            if args.apply
            else publisher.plan
        )
        action()


if __name__ == "__main__":
    main()
