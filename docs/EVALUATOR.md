# Local evaluator bridge

tkd can hand every finished **edit** to a video-scoring model of your own before (or instead of) asking Claude.
No model ships with tkd: you plug yours in with a command in `tkd.toml`. Without one, the review uses measured QA
(`scripts/qa.py`) plus Claude, exactly as before.

## Turning it on

```toml
[evaluator]
name = "my-model"                       # shown in logs and on the Ao vivo screen
command = ["python", "my_eval/eval.py", "{video}", "--caption", "{caption}"]
# optional (defaults shown)
min_footage = 0.5    # only videos where at least this share of the time is footage (0 = every edit)
doubt_low = 6.4      # score <= this: rejected without Claude
doubt_high = 7.6     # score >= this: approved without Claude
audit_every = 5      # 1 in N approvals is still checked by Claude (0 = never)
parallel = 2         # evaluations at once (applies on the next app start)
timeout_min = 15     # a run that takes longer is killed and the review falls back to Claude
```

- `command[0]` is looked up on `PATH`, or relative to the project folder when it contains a slash.
- `{video}` (absolute mp4 path), `{caption}` (post caption) and `{root}` (project folder) are replaced in the
  arguments. The same values come in the env vars `TKD_EVAL_VIDEO` and `TKD_EVAL_CAPTION`.
- The command runs in the project folder with stdin closed.

To try the bridge without a model, use the bundled example (ffmpeg only, measures a frozen opening, black frames
and clipped audio):

```toml
[evaluator]
name = "example"
command = ["python", "scripts/example_evaluator.py", "{video}", "--caption", "{caption}"]
```

`tkd doctor` (or the **Primeiros passos** screen) shows whether the evaluator is configured and found.

## Output

Print anything you like to stderr. The **last stdout line starting with `{`** must be one JSON object:

```json
{
  "model": "my-model-v2",
  "overall": 7.2,
  "scores": {"hook": 8, "pacing": 6.5},
  "tags": ["anime", "sad"],
  "summary": "strong hook, the middle drags",
  "problems": [
    {"t": [4.0, 7.5], "type": "slow_middle", "severity": "medium", "what": "three static shots in a row", "fix": "cut to the beat"}
  ],
  "strengths": [{"t": [0.0, 1.5], "what": "opening line lands on the drop"}],
  "rules": ["frozen_open"],
  "rule_problems": [],
  "seconds": 41.0
}
```

Only `overall` (0–10) is required. `severity` is `high`, `medium` or `low`; a `high` item in `rule_problems`
always sends the video to Claude even with a high score. Portuguese keys are accepted too (`resumo`, `problemas`,
`pontos_fortes`, `tipo`, `gravidade`, `o_que`, `consertar`; severity `alta`/`media`/`baixa`).

A non-zero exit code, a timeout or a missing/invalid `overall` counts as "the model failed": the review continues
with Claude and the run is logged.

## How the decision works

| Score | What happens |
|---|---|
| `<= doubt_low` | rejected by the model alone |
| between `doubt_low` and `doubt_high` | Claude reviews, with the model's evaluation in the prompt |
| `>= doubt_high` | approved by the model alone, unless a `high` rule problem was found or the video falls in the audit sample |

Every model evaluation is saved to `out/reviews/<video-id>.local.json`, and the decision (who decided and why) to
the `reviews` table in `state.db`.

Live clips (`format = "clips"`) never go through the evaluator: Claude already scores them when picking the clip.

## Keeping your model private

Put it anywhere outside git: `evaluator/` and `private/` at the project root are already in `.gitignore`, or use an
absolute path in `command`. Your `tkd.toml` is ignored as well, so your evaluator settings never get committed.
