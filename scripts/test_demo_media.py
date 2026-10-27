import copy
import json
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import demo_media as media

URL = "https://github.com/user-attachments/assets/11111111-1111-1111-1111-111111111111"
NEW_URL = URL.replace("11111111", "22222222")


class FakeGitHub:
    def __init__(self):
        self.base = "repos/owner/repo"
        self.issue = self.base + "/issues/7"
        self.items = []
        self.writes = []
        self.uploads = 0
        self.fail = None
        self.cleared = []
        self.locked = False

    def preflight(self):
        pass

    def acquire(self, journal, save):
        self.locked = True

    def release(self, journal, save):
        self.locked = False

    def comments(self):
        return copy.deepcopy(self.items)

    def upload(self, asset, path, pending, save):
        self.uploads += 1
        return NEW_URL

    def clear_stage(self, pending):
        self.cleared.append(pending["operation"])

    def api(self, endpoint, method="GET", payload=None):
        if method == "GET":
            return copy.deepcopy(
                next(
                    item
                    for item in self.items
                    if endpoint.endswith("/" + str(item["id"]))
                )
            )
        self.writes.append((endpoint, method))
        if self.fail == "before":
            raise TimeoutError("comment timeout")
        if method == "POST":
            item = {
                "id": len(self.items) + 11,
                "body": payload["body"],
                "user": {"login": "owner"},
            }
            self.items.append(item)
        else:
            item = next(
                item
                for item in self.items
                if endpoint.endswith("/" + str(item["id"]))
            )
            item["body"] = payload["body"]
        if self.fail == "after":
            raise TimeoutError("comment timeout")
        return copy.deepcopy(item)


class PublisherTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.args = media.parse_args(
            [
                "--records",
                str(self.root / "media.json"),
                "--manifest",
                str(self.root / "capture.json"),
                "--gallery",
                str(self.root / "gallery.md"),
                "--media",
                str(self.root),
                "--cache",
                str(self.root / "cache"),
            ]
        )
        self.args.gallery.write_text(
            "# Gallery\n\n![Search][search]\n\n[search]: assets/search.jpg\n[other]: assets/other.jpg\n"
        )
        image = self.root / "search.jpg"
        image.write_bytes(b"captured image")
        self.scene = {
            "image": image.name,
            "sha256": media.digest(image),
            "bytes": image.stat().st_size,
            "capture": "original",
        }
        self.info = {"dimensions": [2560, 1440]}
        self.asset = {
            "id": "search-shot",
            "kind": "image",
            "scene": "search",
            "reference": "search",
            "title": "Search",
            "attribution": "Original authors.",
        }
        self.config = {
            "repository": "owner/repo",
            "issue": 7,
            "owner": "owner",
            "assets": {
                "search-shot": {
                    k: v for k, v in self.asset.items() if k != "id"
                }
            },
        }
        media.atomic_json(self.args.records, self.config)
        media.atomic_json(
            self.args.manifest,
            {
                "captures": {"original": self.info},
                "scenes": {"search": self.scene},
            },
        )
        self.metadata = patch.object(media, "metadata", return_value=self.info)
        self.metadata.start()
        self.addCleanup(self.metadata.stop)
        self.remote = patch.object(media, "verify_remote")
        self.verify = self.remote.start()
        self.addCleanup(self.remote.stop)
        self.github = FakeGitHub()
        self.publisher = self.reload()

    def reload(self):
        publisher = media.Publisher(self.args)
        publisher.github = self.github
        return publisher

    def record(self, **extra):
        return {
            "id": "search-shot",
            "kind": "image",
            "url": URL,
            "sha256": self.scene["sha256"],
            "capture": "original",
            "metadata": self.info,
            "published_at": "old",
            "history": [],
            **extra,
        }

    def add_comment(self, record=None, owner="owner"):
        comment = {
            "id": 99,
            "body": media.render_comment(self.asset, record or self.record()),
            "user": {"login": owner},
        }
        self.github.items.append(comment)
        return comment

    def test_dry_run_never_writes(self):
        original = self.args.gallery.read_bytes()
        self.publisher.plan()
        self.assertEqual(self.github.writes, [])
        self.assertEqual(self.github.uploads, 0)
        self.assertFalse(self.publisher.journal_path.exists())
        self.assertEqual(self.args.gallery.read_bytes(), original)

    def test_create_readback_and_noop(self):
        self.publisher.apply()
        published = json.loads(self.args.records.read_text())["assets"][
            "search-shot"
        ]["publication"]
        self.assertEqual(published["comment_id"], 11)
        self.assertEqual(self.github.uploads, 1)
        self.assertIn("[search]: " + NEW_URL, self.args.gallery.read_text())
        before = self.args.records.read_bytes()
        self.reload().apply()
        self.assertEqual(self.args.records.read_bytes(), before)
        self.assertEqual(len(self.github.writes), 1)
        self.assertEqual(self.github.uploads, 1)
        self.assertEqual(len(self.github.items), 1)
        self.assertFalse(self.github.locked)

    def test_stale_id_and_exact_replacement(self):
        old = self.record(sha256="old-hash")
        comment = self.add_comment(old)
        self.config["assets"]["search-shot"]["publication"] = {
            **old,
            "comment_id": 123,
        }
        media.atomic_json(self.args.records, self.config)
        publisher = self.reload()
        publisher.apply()
        self.assertEqual(
            self.github.writes,
            [(self.github.base + "/issues/comments/99", "PATCH")],
        )
        record = media.comment_record(comment, self.asset)
        self.assertEqual(record["history"][0]["url"], URL)
        self.assertEqual(record["history"][0]["metadata"], self.info)
        self.assertEqual(record["capture"], "original")
        self.assertEqual(len(self.github.items), 1)

    def test_duplicate_and_owner_rejected(self):
        for duplicate in (False, True):
            with self.subTest(duplicate=duplicate):
                self.github.items = []
                self.add_comment(owner="owner" if duplicate else "stranger")
                if duplicate:
                    self.add_comment()
                with self.assertRaises(RuntimeError):
                    self.publisher.apply()
                self.assertEqual(self.github.uploads, 0)
                self.assertEqual(self.github.writes, [])

    def test_ambiguous_success_recovers_without_post(self):
        self.github.fail = "after"
        original = self.args.gallery.read_bytes()
        with self.assertRaises(TimeoutError):
            self.publisher.apply()
        self.assertEqual(self.args.gallery.read_bytes(), original)
        self.github.fail = None
        self.reload().apply()
        self.assertEqual(len(self.github.items), 1)
        self.assertEqual(self.github.uploads, 1)
        self.assertEqual(len(self.github.writes), 1)
        self.assertEqual(self.reload().journal["pending"], {})

    def test_ambiguous_missing_post_never_retries(self):
        self.github.fail = "before"
        original = self.args.gallery.read_bytes()
        with self.assertRaises(TimeoutError):
            self.publisher.apply()
        self.github.fail = None
        with self.assertRaisesRegex(RuntimeError, "Ambiguous comment"):
            self.reload().apply()
        self.assertEqual(self.github.uploads, 1)
        self.assertEqual(len(self.github.writes), 1)
        self.assertEqual(self.args.gallery.read_bytes(), original)

    def test_remote_failure_preserves_docs(self):
        original = self.args.gallery.read_bytes()
        self.verify.side_effect = RuntimeError("bad remote bytes")
        with self.assertRaises(RuntimeError):
            self.publisher.apply()
        self.assertEqual(self.args.gallery.read_bytes(), original)
        self.assertEqual(self.github.writes, [])
        self.verify.side_effect = None
        self.reload().apply()
        self.assertEqual(self.github.uploads, 1)

    def test_atomic_record_failure_recovers(self):
        original = self.args.gallery.read_bytes()
        real_atomic = media.atomic_json

        def fail_records(path, data):
            if path == self.args.records:
                raise OSError("disk full")
            real_atomic(path, data)

        with patch.object(media, "atomic_json", side_effect=fail_records):
            with self.assertRaises(OSError):
                self.publisher.apply()
        self.assertEqual(self.args.gallery.read_bytes(), original)
        self.reload().apply()
        self.assertEqual(len(self.github.items), 1)
        self.assertEqual(self.github.uploads, 1)

    def test_concurrent_catalog_edit_preserved(self):
        changed = {**self.config, "note": "external edit"}
        media.atomic_json(self.args.records, changed)
        original = self.args.records.read_bytes()
        gallery = self.args.gallery.read_bytes()
        with self.assertRaisesRegex(RuntimeError, "catalog changed"):
            self.publisher.apply()
        self.assertEqual(self.args.records.read_bytes(), original)
        self.assertEqual(self.args.gallery.read_bytes(), gallery)
        self.reload().apply()
        self.assertEqual(self.github.uploads, 1)
        self.assertEqual(len(self.github.items), 1)
        self.assertEqual(
            json.loads(self.args.records.read_text())["note"], "external edit"
        )

    def test_partial_batch_progress(self):
        self.config["assets"]["missing-shot"] = {
            **self.config["assets"]["search-shot"],
            "scene": "missing",
        }
        media.atomic_json(self.args.records, self.config)
        with self.assertRaises(KeyError):
            self.reload().apply()
        saved = json.loads(self.args.records.read_text())
        self.assertIn("publication", saved["assets"]["search-shot"])
        self.assertIn("[search]: " + NEW_URL, self.args.gallery.read_text())

    def test_changed_local_capture_rejected(self):
        (self.root / "search.jpg").write_bytes(b"changed")
        with self.assertRaisesRegex(RuntimeError, "Capture hash"):
            self.publisher.plan()
        self.assertEqual(self.github.writes, [])

    def test_offline_check_without_cached_media(self):
        self.publisher.apply()
        (self.root / "search.jpg").unlink()
        archive = (
            self.args.cache / "archive" / self.scene["sha256"] / "search.jpg"
        )
        archive.unlink()
        publisher = self.reload()
        before = self.args.gallery.read_bytes()
        with patch.object(media, "ROOT", self.root):
            publisher.check()
            publisher.plan()
        self.assertEqual(self.args.gallery.read_bytes(), before)
        self.assertEqual(self.github.uploads, 1)

    def test_remote_check_requires_publication(self):
        self.args.remote = True
        with self.assertRaisesRegex(RuntimeError, "no publication record"):
            self.publisher.check()

    def test_reference_boundaries(self):
        text = "[search]: assets/search.jpg\n[search-other]: keep.jpg\nprose assets/search.jpg\n"
        self.assertEqual(
            media.replace_references(text, {"search": URL}),
            "[search]: "
            + URL
            + "\n[search-other]: keep.jpg\nprose assets/search.jpg\n",
        )
        for text in ("[other]: keep.jpg", "[search]: one\n[search]: two"):
            with self.assertRaises(RuntimeError):
                media.replace_references(text, {"search": URL})


class GitHubTests(unittest.TestCase):
    def setUp(self):
        self.github = media.GitHub(
            {"repository": "owner/repo", "issue": 7, "owner": "owner"}
        )

    def test_all_comment_pages(self):
        pages = [[{"id": number} for number in range(100)], [{"id": 101}]]
        with patch.object(self.github, "api", side_effect=pages) as api:
            comments = self.github.comments()
        self.assertEqual(len(comments), 101)
        self.assertIn("page=2", api.call_args.args[0])

    def test_retry_reads_only(self):
        for method, expected in [("GET", 3), ("POST", 1), ("PATCH", 1)]:
            with (
                self.subTest(method=method),
                patch.object(
                    media,
                    "command",
                    side_effect=subprocess.TimeoutExpired("gh", 1),
                ) as command,
                patch.object(media.time, "sleep"),
            ):
                with self.assertRaises(subprocess.TimeoutExpired):
                    self.github.api("endpoint", method)
                self.assertEqual(command.call_count, expected)

    def test_lock_contention(self):
        with (
            patch.object(
                self.github,
                "matching_lock",
                return_value={"object": {"sha": "other"}},
            ),
            patch.object(self.github, "api") as api,
        ):
            with self.assertRaisesRegex(RuntimeError, "already exists"):
                self.github.acquire({}, lambda: None)
            self.assertFalse(api.called)

    def test_lock_resume_and_ownership(self):
        journal = {"lock_sha": "mine"}
        with (
            patch.object(
                self.github,
                "matching_lock",
                return_value={"object": {"sha": "mine"}},
            ),
            patch.object(self.github, "api") as api,
        ):
            self.github.acquire(journal, lambda: None)
            self.assertFalse(api.called)
        with patch.object(
            self.github,
            "matching_lock",
            return_value={"object": {"sha": "other"}},
        ):
            with self.assertRaises(RuntimeError):
                self.github.release(journal, lambda: None)

    def test_staging_recovery(self):
        asset = {"id": "search-shot"}
        pending = {"operation": "op", "sha256": "hash", "upload_started": True}
        body = (
            "Human text\n\n"
            + media.STAGE_START
            + "\noperation=op asset=search-shot sha256=hash\n"
            + URL
            + "\n"
            + media.STAGE_END
            + "\nFooter"
        )
        with (
            patch.object(self.github, "api", return_value={"body": body}),
            patch.object(media, "command") as command,
        ):
            self.assertEqual(
                self.github.upload(
                    asset, Path("/image.jpg"), pending, lambda: None
                ),
                URL,
            )
            self.assertFalse(command.called)
        with patch.object(
            self.github, "api", return_value={"body": body.replace(URL, "")}
        ):
            with self.assertRaisesRegex(RuntimeError, "Ambiguous upload"):
                self.github.upload(
                    asset, Path("/image.jpg"), pending, lambda: None
                )

    def test_staging_preserves_human_text(self):
        before, stage, after = media.stage_parts("Human text\n  ")
        self.assertTrue(before.startswith("Human text\n  "))
        for body in [
            media.STAGE_START,
            media.STAGE_START + media.STAGE_END + media.STAGE_END,
        ]:
            with self.assertRaises(RuntimeError):
                media.stage_parts(body)

    def test_multiple_markers_rejected(self):
        comment = {
            "id": 1,
            "user": {"login": "owner"},
            "body": "<!-- vpdf-media:one -->\n<!-- vpdf-media:two -->",
        }
        with self.assertRaisesRegex(RuntimeError, "multiple markers"):
            media.find_comments([comment], "owner")

    def test_noncanonical_urls_rejected(self):
        for url in [
            URL + "?token=temporary",
            URL + "#fragment",
            "https://example.com/image.jpg",
        ]:
            with self.assertRaises(RuntimeError):
                media.canonical(url)


class DocumentTests(unittest.TestCase):
    def test_consumer_failure_rolls_back_all(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            first, second = root / "gallery.md", root / "README.md"
            first.write_text("gallery before")
            second.write_text("readme before")
            replace = Path.replace

            def fail_second(source, target):
                if source.name == "updated" and target == second:
                    raise OSError("blocked readme")
                return replace(source, target)

            with patch.object(Path, "replace", new=fail_second):
                with self.assertRaises(OSError):
                    media.replace_documents(
                        [
                            (first, "gallery before", "gallery after"),
                            (second, "readme before", "readme after"),
                        ]
                    )
            self.assertEqual(first.read_text(), "gallery before")
            self.assertEqual(second.read_text(), "readme before")
            self.assertEqual(
                sorted(p.name for p in root.iterdir()),
                ["README.md", "gallery.md"],
            )

    def test_concurrent_consumer_edit_preserved(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "gallery.md"
            path.write_text("external edit")
            with self.assertRaisesRegex(RuntimeError, "changed before"):
                media.replace_documents([(path, "original", "updated")])
            self.assertEqual(path.read_text(), "external edit")


class RemoteTests(unittest.TestCase):
    def test_anonymous_type_hash_and_metadata(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "image.jpg"
            source.write_bytes(b"reviewed capture")
            record = {
                "url": URL,
                "sha256": media.digest(source),
                "metadata": {"dimensions": [2560, 1440]},
            }
            for failure in ("none", "hash", "type", "dimensions"):
                with self.subTest(failure=failure):

                    def download(args):
                        self.assertEqual(args[:2], ["curl", "-q"])
                        self.assertNotIn("--user", args)
                        self.assertNotIn("--header", args)
                        target = Path(args[args.index("--output") + 1])
                        target.write_bytes(
                            b"changed"
                            if failure == "hash"
                            else source.read_bytes()
                        )
                        return (
                            "text/html" if failure == "type" else "image/jpeg"
                        )

                    info = (
                        {"dimensions": [1, 1]}
                        if failure == "dimensions"
                        else record["metadata"]
                    )
                    with (
                        patch.object(media, "command", side_effect=download),
                        patch.object(media, "metadata", return_value=info),
                    ):
                        if failure == "none":
                            media.verify_remote(record, "image")
                        else:
                            with self.assertRaises(RuntimeError):
                                media.verify_remote(record, "image")
