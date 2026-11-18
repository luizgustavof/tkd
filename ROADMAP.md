# Roadmap

tkd runs short-video channels on its own: every channel is an agent with its own memory, account and posting
schedule, built on Claude (`claude -p`) for the creative decisions and on measured rules (ffmpeg, whisper,
loudness, scene detection) for everything that can be checked without a model.

This file lists where the project is going. It is ordered by priority inside each section.

## Where it is today

| Area | State |
|---|---|
| **cortes** format (live clipper: Twitch, Kick, YouTube) | Running unattended in production: watches up to 6 lives at once, cuts by chat/audio/motion, checks the live's game category, Claude picks the best clip, 9:16 with captions, posts with streamer credit |
| **edit** format (footage edits) | Works end to end (study references → recreate structure → Remotion render → QA → post); still needs a human look before posting |
| Desktop app (Tauri + React) | Channels, live activity per watched stream, agents/models per task, clip browser with player, token usage |
| Reliability | Chaos test suite (`tkd chaos`): killed processes, network faults, rate limits, corrupt JSON, full disk |
| Platforms | Windows 10/11 (Job Objects, Edge profiles) |

## Recently done

- **Settings and state in SQLite (step 1).** `state.db` keeps the last valid `channel.toml` of every channel (a
  broken or deleted file loads from it), the used-clip list (`clip_uses`, migrated from `cortes.json`) and the
  per-slot attempts.
- **Lower priority for heavy children.** ffmpeg, Remotion, whisper and yt-dlp run under
  `BELOW_NORMAL_PRIORITY_CLASS` (`TKD_CHILD_PRIORITY=normal` opts out).
- **Minimum gap between posts.** Half of the channel's tightest slot spacing (10–90 min) after its last post, so
  missed slots after a reboot no longer go out minutes apart.
- **Character matching from the catalog first.** Speaker and per-frame characters decide; text is a whole-word
  fallback that ignores name parts shared with another known character.
- **Guided first run.** **Primeiros passos** screen and `tkd doctor`: ffmpeg, node, `claude`, yt-dlp + JS solver,
  venv, Edge, config, with copyable commands and one-click fixes.

## Next

1. **Settings and state in SQLite (step 2).** Each live clip's metadata (`out/live/*/clips/*.json`) and the clip
   banks into `state.db`, so the app queries instead of scanning folders; TOML export/import of a whole channel.
2. **`claude` login check without the credentials file** (keychain logins show as a warning today).

## Later

- **Work ahead when the PC is idle**: render the next slot's edit up to ~12 h early and only publish on time
  (live clips stay just-in-time, they age fast).
- **RAM-weighted job queue** (render, whisper, 9:16 encode) if lower priority alone is not enough.
- **Clipper quality**: learn per-streamer chat baselines; detect giveaway/command floods earlier (first version
  shipped: commands and copy-paste waves no longer count as reactions).
- **Pluggable evaluator interface**: a documented JSON contract for a local video-scoring model, so anyone can plug
  their own (the project ships without one; review falls back to measured QA + Claude).
- **Trend and music suggestions** from public data (sounds, formats) fed to the ideation prompt.
- **Cheaper runs**: skip YouTube comment fetching when a heatmap exists, smaller blurred background in the 9:16
  encode, TTS cache by hash.
- **Finish the English rename**: a few leftovers in `browser/` (the `cortes` module is now `clips`; the old
  `format = "cortes"` and `[tasks.review_cortes]` still load).

## Platform

- **Linux and macOS**: replace Job Objects with process groups, Edge profile paths per OS, CI on all three.
- **More destinations**: Instagram Reels after TikTok and YouTube Shorts.

## Non-goals

- **No GPU paths by default.** Everything runs on the CPU; hardware decoding crashed the display driver on the
  reference machine and is not worth the risk for an unattended tool.
- **No voice cloning of real people.** Edits use the character's real lines from the footage.
- **No mass account farming.** Limits (posts per day, ≥ 45 s between TikTok openings, one browser at a time,
  YouTube request budget) are part of the design, not settings to remove.

## Contributing

Issues and PRs are welcome. Keep code identifiers in English; user-facing text, logs and comments are in pt-BR
for now (translation of the UI is on the list once the strings are extracted). Run `cargo test` and the relevant
`tkd chaos` scenario for anything that touches capture, posting or file writes.
