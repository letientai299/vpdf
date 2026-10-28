import unittest

from tools.bench.live.expectation import matches, validate


class ExpectationTests(unittest.TestCase):
    def test_idle_does_not_prove_outcomes(self):
        for text, row in (
            ("page 5", dict(page=1, busy=False, complete=True)),
            ("hits_min 1", dict(hits=0, busy=False, complete=True)),
            (
                "search_outcome success",
                dict(search_outcome="error", busy=False),
            ),
            ("sharp true", dict(complete=True, busy=False)),
            ("hidden_released true", dict(resources=[])),
            ("sharp false", {}),
            ("settled false", {}),
        ):
            self.assertFalse(matches(text, row), text)
        self.assertTrue(matches("document sdm", dict(document="/tmp/sdm.pdf")))
        self.assertTrue(matches("hits_min 1", dict(hits=2)))

    def test_release_requires_every_hidden_owner(self):
        clean = dict(
            visible=False,
            text_bytes=0,
            tile_bytes=0,
            search_workers=0,
            active_workers=0,
        )
        self.assertTrue(
            matches("hidden_released true", dict(resources=[clean]))
        )
        for key in (
            "text_bytes",
            "tile_bytes",
            "search_workers",
            "active_workers",
        ):
            with self.subTest(key=key):
                dirty = dict(clean, **{key: 1})
                self.assertFalse(
                    matches(
                        "hidden_released true", dict(resources=[clean, dirty])
                    )
                )

    def test_checkpoint_requires_change(self):
        before = dict(
            pane=1,
            document="sdm.pdf",
            document_generation=0,
            page=1,
            scroll=0.0,
            selected_hit=1,
            query_generation=8,
        )
        for text in ("moved keys", "selected_changed results"):
            self.assertFalse(matches(text, before, before))
            self.assertFalse(matches(text, before))
        after = dict(before, scroll=10.0, selected_hit=2)
        self.assertTrue(matches("moved keys", after, before))
        self.assertTrue(matches("selected_changed results", after, before))
        self.assertFalse(matches("moved keys", dict(after, pane=2), before))
        self.assertFalse(
            matches(
                "selected_changed results",
                dict(after, query_generation=9),
                before,
            )
        )

    def test_bad_expectations_fail_before_launch(self):
        for text in (
            "page",
            "hits_min -1",
            "missing 0",
            "sharp maybe",
            "search_outcome complete",
        ):
            with self.assertRaises(ValueError):
                validate(text)
