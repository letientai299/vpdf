# Demo media publication

Capture and encoding run offline. Publication is a separate, explicit action.
The [gallery][gallery] uses canonical attachments on the [closed hosting
issue][host]. App icons, source artwork, capture identities, and publication
records remain in git; encoded demos and recovery copies remain outside git.

## Setup

Install [GitHub CLI][gh-install] and authenticate with `gh auth login`.
Publication needs push access to the public repository and the account recorded
in [the asset catalog][catalog]. Fine-grained tokens need Issues and Contents
write access: comments use Issues; the shared publisher lock uses Contents.
Check `gh issue edit --help` for `--attach`; older installations need upgrading.
See [GitHub's attachment guide][attachments] for supported upload behavior.

The catalog records the dedicated issue number. Keep that issue closed. The
publisher never creates or replaces a missing issue and refuses an open issue.
Issue creation and the first smoke upload require explicit authorization.

Capture helpers and attribution remain in the [capture guide][capture]. The
catalog assigns logical IDs independently of file hashes. Each asset points to
its scene in the [capture manifest][manifest], a gallery reference, a title, and
attribution. Review both the image and its attribution before applying.

## Record interactions

Video capture requires macOS with [ScreenCaptureKit][sck], the Xcode
command-line tools for Swift, [FFmpeg][ffmpeg] with H.264 encoding, and the
screenshot helpers in the capture guide. Install FFmpeg from its upstream
download page; on macOS, `brew install ffmpeg` supplies the encoder. Use the
script's `--help` for source, geometry, placement, font, recipe, and output
controls.

```sh
mise run build
mise run demo:video -- --list-scenes
mise run demo:video -- --recipe find-return
```

Keep the test window visible. Capture uses a background Kitty window, rejects
focus on that process, and permits the user to switch between other apps. The
live-runner lock and build-worker admission gate prevent competing captures and
repository builds. A busy worker delays capture within a bounded startup wait.
No capture command uploads media.

The [recipes][video-script] separate setup from demonstrated inputs. They log
monotonic dispatch times and state assertions. ScreenCaptureKit records the real
window, including PDF graphics; idle frames retain the last image. The manifest
records requested and delivered rates, frame statuses, writer drops, delivery
gaps, and an unknown source-drop count because that API does not expose one.
These fields describe recording, not reader performance.

ImageMagick renders a fixed RGB key strip from those same input times. FFmpeg
places it below the source viewport, encodes H.264 at normal speed, and extracts
a poster. This works with FFmpeg builds that lack text-overlay filters. No cuts,
time compression, or source-viewport resizing occur. Inspect the video, its key
strip, and the poster before publishing. The gallery provides text transcripts;
posters and clips have separate logical IDs and comments.

## Plan and apply

```sh
mise run demo:check
mise run demo:publish
mise run demo:publish -- --asset search-shot
```

Dry runs validate local hashes, dimensions, and sizes. With an issue configured,
they enumerate every comment page, reject duplicate markers or unexpected
owners, and verify unchanged attachments anonymously. They print the proposed
upload, exact comment update or creation, and gallery reference replacement.
They do not write to GitHub or update gallery links.

After review and authorization:

```sh
mise run demo:publish -- --asset search-shot --apply
mise run demo:check -- --asset search-shot --remote
```

The publisher uploads through `gh issue edit --attach`. A marked block in the
issue body holds the upload until its exact marked comment is durable. The
[comment API][comments] updates that comment by ID. Cached IDs are hints; exact
markers determine ownership. Capture identity stays separate from publication
time. Metadata corrections reuse verified attachments; changed bytes upload
new files. Canonical GitHub attachment URLs, hashes, dimensions, and superseded
attachments remain in the comment and catalog. The issue stays closed.

Only managed Markdown reference definitions change, including additional
consumers listed in the catalog. Publication records save before links change;
retries repair a failed local link update without uploading again. Each
completed asset saves independently, so a later failure preserves that progress.
A failed upload, comment readback, or anonymous check leaves the previous links
intact. Staging an upload also requests the public issue page before checking
its canonical URL. Newly uploaded URLs can briefly return 404; bounded read
retries and the journal preserve progress without uploading again.

## Storage and recovery

Capture defaults and recovery archives live under the host's
`~/.cache/vpdf/demo-media/<checkout>/`, outside synced notes and Git. The
capture script rejects output overrides under nonignored repository paths.
`--output`, `--raw`, and `--manifest` control encoded output, working captures,
and the tracked capture manifest separately. Keep the archive directory backed
up; GitHub hosting remains an external dependency. Never delete superseded
attachments during sync: old Git revisions must retain their original bytes.

The publisher takes a local file lock and atomically creates the repository ref
`refs/tags/vpdf-media-publisher-lock` for coordination across machines. Its
unique annotated tag identifies the operation. It removes only its own lock
after success. An interrupted operation retains its host-local journal and
remote lock. Rerun with the same cache and configuration to resume. Do not start
another publisher, copy a live journal to another host, or remove a live
publisher's lock.

Inspect `journal.json` in the selected `--cache` directory and the issue staging
block after a failure. A completed upload or comment is read back before retry.
An ambiguous write with no durable URL or comment stops for inspection; it never
blindly repeats a comment POST. GET failures use bounded retries; failed writes
stop immediately, including rate limits. Wait for any reported reset before
resuming.

If a timed-out comment is absent after the original request has settled, confirm
that the full issue comment list contains no matching marker before clearing
only that asset's `comment_started` journal field. If an upload has no URL,
inspect the issue body and confirm that its original command has stopped before
clearing only `upload_started`. Preserve uploaded URLs whenever present. Never
clear the journal or staging block while a comment write could still complete.

If a host or journal is lost, confirm its process is stopped, inspect the remote
lock object's operation and the issue staging block, and recover the uploaded
URL and matching comment before removing a stale lock. Lock deletion is a manual
recovery action; the publisher never expires or steals another publisher's lock.
Retain the old catalog and capture manifest until recovery completes.

## Smoke gate

Use one reviewed screenshot. Publish it, publish a reviewed replacement under
the same logical ID, and rerun unchanged. Verify the same comment ID, one
marker, no additional attachment on the unchanged run, and anonymous current and
old URL bytes. Check the actual GitHub Markdown image presentation and the
issue's closed state. Keep the local recovery copies. Complete this gate before
migrating or removing any tracked demo images.

Verify videos with GitHub-rendered Markdown and anonymous playback. GitHub
replaces attachment-linked posters with inline players. Link each poster to the
video's stable marked comment to keep it visible; a separate direct video link
can supply the inline player. Posters have their own logical IDs, captures, and
comments. Capture and review remain separate from publication.

[gallery]: ../demos.md
[catalog]: ../assets/media.json
[manifest]: ../assets/capture.json
[capture]: dev.md#gallery-capture
[gh-install]: https://github.com/cli/cli#installation
[attachments]: https://docs.github.com/en/github-cli/github-cli/attaching-files-with-github-cli
[comments]: https://docs.github.com/en/rest/issues/comments
[host]: https://github.com/letientai299/vpdf/issues/9
[sck]: https://developer.apple.com/documentation/screencapturekit/capturing-screen-content-in-macos
[ffmpeg]: https://ffmpeg.org/download.html
[video-script]: ../../tools/demo/video.py
