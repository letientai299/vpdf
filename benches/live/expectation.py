"""Scenario outcome checks shared by live backends."""

from pathlib import Path

NUMERIC = {
    "page",
    "pages",
    "pane",
    "pane_count",
    "tab",
    "hits",
    "hits_min",
    "selected_hit",
    "selected_hit_min",
    "query_generation",
    "query_generation_min",
    "document_generation",
}
BOOLEAN = {"hidden_released", "settled", "sharp", "overview"}


def validate(text):
    field, value = text.split(maxsplit=1)
    if field in NUMERIC:
        if (
            not value.isascii()
            or not value.isdecimal()
            or int(value) >= 1 << 64
        ):
            raise ValueError("Invalid expectation number")
    elif field in BOOLEAN:
        if value not in ("true", "false"):
            raise ValueError("Invalid expectation boolean")
    elif field == "search_outcome":
        if value not in ("idle", "running", "success", "cancelled", "error"):
            raise ValueError("Invalid search outcome")
    elif field not in (
        "document",
        "query",
        "panel",
        "moved",
        "selected_changed",
    ):
        raise ValueError("Unknown expectation field")
    return field, value


def matches(text, row, baseline=None):
    field, value = validate(text)
    if field in ("moved", "selected_changed"):
        if baseline is None or any(
            row.get(key) is None or row.get(key) != baseline.get(key)
            for key in ("document", "pane", "document_generation")
        ):
            return False
        if field == "moved":
            import math

            return all(
                isinstance(r.get(key), (int, float)) and math.isfinite(r[key])
                for r in (row, baseline)
                for key in ("page", "scroll")
            ) and (
                row["page"] != baseline["page"]
                or row["scroll"] != baseline["scroll"]
            )
        return (
            isinstance(row.get("query_generation"), int)
            and row["query_generation"] == baseline.get("query_generation")
            and isinstance(row.get("selected_hit"), int)
            and row["selected_hit"] > 0
            and isinstance(baseline.get("selected_hit"), int)
            and baseline["selected_hit"] > 0
            and row["selected_hit"] != baseline["selected_hit"]
        )
    if field == "document":
        return (
            isinstance(row.get(field), str) and Path(row[field]).stem == value
        )
    if field in ("query", "panel", "search_outcome"):
        return row.get(field) == value
    if field in NUMERIC:
        key = field.removesuffix("_min")
        actual = row.get(key)
        return isinstance(actual, int) and (
            actual >= int(value) if key != field else actual == int(value)
        )
    if field == "overview":
        return row.get(field) is (value == "true")
    if field == "hidden_released":
        resources = row.get("resources")
        if not isinstance(resources, list):
            return False
        hidden = [r for r in resources if r.get("visible") is False]
        result = bool(hidden) and all(
            all(
                r.get(key) == 0
                for key in (
                    "text_bytes",
                    "tile_bytes",
                    "search_workers",
                    "active_workers",
                )
            )
            for r in hidden
        )
    elif field == "settled":
        if any(key not in row for key in ("animating", "busy")):
            return False
        result = row["animating"] is False and row["busy"] is False
    else:
        if any(
            key not in row
            for key in ("complete", "busy", "missing", "stand_ins")
        ):
            return False
        result = (
            row["complete"] is True
            and row["busy"] is False
            and row["missing"] == 0
            and row["stand_ins"] == 0
        )
    return result == (value == "true")
