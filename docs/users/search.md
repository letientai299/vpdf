# Search

<!-- help-topic:search -->

Type `/pattern` and Enter to search forward; `?pattern` starts backward.
Search is literal unless the pattern begins with `\v`. In Think Python, try
`/function` or `/return value`. In this manual, try `/navigation`.

`n` finds the next match in the search direction; `N` reverses it. A count
skips matches. With the cursor on a word, `*` starts a whole-word search; a
Visual selection supplies search text too. Press `Ctrl-o` to return from a
match jump. Returning does not discard the query.

## Matching text

A query with no uppercase letters ignores case. Any uppercase letter makes
it case sensitive: `/Function` differs from `/function`. Search normalizes
Unicode compatibility forms, repeated whitespace, wrapped lines, and line-end
hyphenation. A phrase can match across a printed line break or a hyphenated
word without copying that break or hyphen into the query.

Literal search folds Vietnamese vowel accents and `đ`: `/ca phe` finds
`cà phê`. It does not remove every accent in every language. Matches still
highlight the original printed glyphs. CJK continuation lines in a text block
can join without an added space. Whole-word matching uses dictionary boundaries
for Chinese and Japanese; a substring of a compound may not count as a word.

Prefix a query with `\v` for a regular expression. Try `/\vfunction|variable`
for either word, or `/\vchapter\s+[0-9]+` for a numbered chapter. These are
examples to adapt to the book, not a promise that each PDF contains them.
Regular expressions use normalized text without the literal Vietnamese accent
folding. Character classes see decomposed forms. Explicit `\b` keeps regex
word-boundary semantics. Invalid patterns report an error; shorten an expensive
pattern if it reaches the search limit. The engine's syntax differs from Vim's
full pattern language.

## Prompt and results

While typing, keys edit the query. Left and Right move the prompt cursor,
Backspace deletes, and `Ctrl-u` clears text before it. Up and Down recall
history; `Ctrl-x Ctrl-e` opens the configured external editor for longer input.
Escape cancels the prompt. Accept a new query to replace the current document
search; results from an older scan cannot replace the new query.

Press `gs` to open RESULTS. Use `j` and `k` to choose a result, Enter to follow
and focus content, or `o` to follow and keep list focus. Tab changes focus;
`q` closes the focused results panel. Searching while a panel owns focus
searches its rows. Return focus to content to search the PDF.

The header shows the query. Results group under a heading per page; each row is
an excerpt centered on the match. `h` and `l` fold a page, and a folded page
shows how many hits it hides. `▌` marks the hit the reader landed on.

Search scans pages in the background and adds matches as they arrive. A partial
result list is not a complete count. Rendering takes priority over scanning.
`:cancel` stops the pane's search job along with OCR, outline, and region jobs.
Start another search to request work again. `:nohlsearch` only hides highlights;
it does not cancel the job or erase the last query. `n` can show matches again.

Search reads embedded text and available page OCR. It does not recognize every
scanned page merely because you searched. Display an image page or request
`:ocr` before expecting its recognized text to be searchable. See
[Scanned pages and OCR][ocr] for coverage and cache limits.

Watch [find and return][search-demo].

[ocr]: ocr.md
[search-demo]: ../demos.md#find-and-return
