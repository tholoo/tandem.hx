# Feature recordings

These assets show the real Tandem plugin running in Steelix. Agent responses and proposal contents are scripted through a local demo transport, so recording does not contact a model, use credentials, or touch a real project. The fixture lives in `scripts/demo/`.

- [Tour animation](tour.gif): walk from a call site to its implementation with `]t`.
- [Tour still](tour.png): narration, reading range, source, and conversation together.
- [Comparison](comparison.png): actual native Helix splits with old and current code.
- [Conversation](conversation.png): a follow-up question beside the relevant code.
- [Terminal recording](walkthrough.cast): the captured PTY output in asciicast v2 format. Download it and run `asciinema play walkthrough.cast` to replay locally.

PNG and GIF images are rendered from captured terminal cells using Pillow and a local monospace font. They are not generated UI concepts. The GIF shows the repository tour; the cast also includes the proposed change, comparison, and follow-up question.

## Reproduce

From `nix develop`, with Steelix and its runtime available:

```sh
python3 scripts/record-demo.py /path/to/hx /path/to/runtime \
  --font /path/to/monospace.ttf
```

The script creates an isolated repository and editor configuration, records real keyboard input, asserts that each feature is visible, and writes the assets here. The checked-in captures use JetBrains Mono at 112 columns by 32 rows. No font file is bundled.

Keep the caption about scripted responses when reusing these recordings. Review new recordings for personal paths, private code, and credentials before committing them.
