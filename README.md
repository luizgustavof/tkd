# tkd — a factory of TikTok / Shorts channels

> **English** · [Português](README.pt-BR.md)

A Rust app (the `tkd` CLI plus a Tauri desktop app) that runs short-video channels on its own. Every channel is an
agent with its own memory, its own account (one Edge profile per channel) and its own posting schedule. Claude
(`claude -p`) makes the creative calls; measured rules (ffmpeg, whisper, loudness, scene detection) check
everything that can be checked without a model.

Two channel formats (`format` in `channel.toml`):

- **clips** — live-stream clipper. Watches streamers on Twitch, Kick and YouTube, cuts the strongest moments from
  chat, audio and motion, checks that the live is in the channel's game, lets Claude pick the best clip, renders a
  9:16 version with captions and posts it crediting the streamer. Runs unattended in production.
- **edit** — footage edits. Studies reference profiles, rebuilds an edit's structure with the channel's clip bank
  and the character's real lines, renders with Remotion, reviews it (measured QA + Claude) and posts.

The user interface, logs and code comments are in Portuguese (pt-BR) for now; identifiers are in English.

## Install

**Windows, the easy way:** download the project and double-click `instalar.bat`. It installs whatever is missing
with winget (Git, Node.js, FFmpeg, Python 3.12, Rust, the MSVC build tools), the Python and Node packages, builds
the app and puts a **tkd** shortcut on the desktop. Safe to run again: it skips what is already there.

Then sign in to Claude once (`claude` in a terminal), open the shortcut and follow the **Primeiros passos**
(first steps) screen, or run `tkd doctor`.

Step-by-step guide (pt-BR): [`INSTALAR.md`](INSTALAR.md). Claude plan, models and cost: [`CLAUDE_CODE.md`](CLAUDE_CODE.md).

### Manual install

Node 20+, Rust, pnpm, ffmpeg on PATH, Microsoft Edge, and the `claude` CLI signed in.

- **yt-dlp** in the global Python with YouTube's JS solver: `pip install -U "yt-dlp[default]"` (brings the matching
  `yt-dlp-ejs`; without it HD formats disappear).
- **Project venv** `tools/rea-video/.venv` (Python 3.12):
  `pip install -r scripts/requirements.txt`, then `scrapling install`.
- `npm ci` in `browser/` and `video/`, `pnpm install` in `app/`, then `pnpm tauri build --no-bundle` in `app/`.

Everything runs on the CPU: ffmpeg never uses `-hwaccel` or DirectML.

## First channel

```bash
cp tkd.example.toml tkd.toml                  # Claude model per task, autopilot
cp -r channels/_example channels/my-channel   # edit slug, niche, persona, schedule
```

Or create it in the app (**Novo canal**) and connect TikTok/YouTube there. Before turning the autopilot on, use
**Post de teste** on the channel screen: it publishes for real but only you can see it (TikTok "only me", YouTube
unlisted), and it does not count toward the day's posts.

## Desktop app

Screens: **Canais** (channels: create, configure, connect accounts, schedule, notes for the agent), **Ao vivo** (what
each agent is doing right now, one row per watched live), **Agentes** (model and effort per task), **Cortes** (every
clip the clipper cut, with preview and player), **Consumo** (token usage per channel and task) and **Primeiros
passos** (setup check with one-click fixes).

The autopilot (`[app] autopilot` in `tkd.toml`) starts the live watchers and the posting cycle. Closing the app
mid-task is safe: children die with it (Windows Job Object) and the next start marks unfinished work as interrupted.

## CLI

| Command | What it does |
|---|---|
| `status` | State of each channel |
| `doctor` | Checks ffmpeg, node, `claude`, yt-dlp, the venv, Edge and the config |
| `login` / `logout` / `accounts [--check]` | TikTok/YouTube session in the channel's browser profile (`.profiles/<channel>/`) |
| `yt-login` | A **throwaway** Google account for yt-dlp cookies (never a channel's account) |
| `scout` / `analyze` / `study` | Download and measure references; distill `style-card.md` + `preset.json` |
| `footage` / `catalog` / `moments` / `music` | The channel's clip bank (search, cut, per-scene catalog, soundtrack) |
| `recreate` | Rebuild an edit from a reference |
| `ideate` / `render` / `review` / `publish` | Single pipeline steps |
| `run --channel X [--dry-run] [--test-post]` | Full pipeline for one video |
| `live --url URL` | Watch a live and cut the strong moments (without the autopilot) |
| `watch` | The autonomous cycle from the CLI (same as the app's autopilot) |
| `usage` | Token usage |
| `chaos check / list / run / watch-live` | Chaos tests ([`docs/CHAOS.md`](docs/CHAOS.md)) |

- `--dry-run` goes up to the publish screen and does not click.
- `--test-post` publishes for real but **unlisted on YouTube** and **"only me" on TikTok**; the script checks the
  privacy selector before clicking and aborts if it could not set it.
- `TKD_LOG=debug` turns on verbose logs. `TKD_MAX_WATCH` (default 6) sets how many lives are watched at once.

## Layout

```
src/              CLI (clap)
crates/tkd-core/  the engine: channels, agents, pipeline, clips, live, publish, events, state.db
app/              desktop app (Tauri 2 + React; bindings generated with `pnpm bindings`)
browser/          Playwright on the signed-in profile: login, TikTok/YouTube upload
scripts/          Python: live.py (clipper), clip_finish.py, qa.py, speech.py, moments.py, install.ps1...
agents/           Claude prompt templates (ideate, recreate, review_edit, review_clips...)
video/            Remotion project (compositions and `motionlib`)
channels/<slug>/  channel.toml, knowledge/ (style card, preset, clip bank), library/
out/              specs, renders, live/<session>/clips (generated)
docs/             chaos tests, quality rules, Remotion effects
```

Channels whose slug starts with `_` (like `_example`) stay out of the autopilot and only run when called by name.

What comes next: [`ROADMAP.md`](ROADMAP.md).

## Edit review

Before posting, every edit goes through measured QA (`scripts/qa.py`: loudness, peaks, frozen frames, captions) and
a Claude review. You can also plug in **your own local video-scoring model**: any command that takes the mp4 and
prints a JSON score (`[evaluator]` in `tkd.toml`, protocol in [`docs/EVALUATOR.md`](docs/EVALUATOR.md)). Clear
scores decide alone, doubtful ones go to Claude with the model's notes. No model ships with tkd; without one the
review uses QA + Claude only.

## Warnings

- The automation drives a real browser session, which goes against TikTok's terms: accounts can be restricted. The
  limits (posts per day, pauses, one browser at a time, ≥ 45 s between TikTok openings, a 200 requests/hour budget
  on YouTube) reduce that risk, they do not remove it.
- Every upload compares the signed-in `@handle` with the channel's `post_handle` and aborts on a mismatch.
- `.profiles/` holds each account's cookies: never commit or share it (it is in `.gitignore`).
- Downloaded videos (references, footage, lives) are local working material. Live clips are posted with credit to
  the streamer; only clip streamers who allow it.

## License

Code under [MIT](LICENSE): use, modify and sell it, including in commercial products, keeping the copyright notice.

Dependencies with their own licenses apply separately:

- **Remotion** (edit rendering): free for individuals and small companies; larger companies need a
  [Remotion license](https://www.remotion.dev/license). The clips format (ffmpeg only) does not depend on it.
- **Models and services** (Claude, whisper, ElevenLabs): each under its own terms.
- **Platforms**: post automation goes against TikTok's terms (see Warnings). Third-party video (footage, lives)
  belongs to whoever made it.
