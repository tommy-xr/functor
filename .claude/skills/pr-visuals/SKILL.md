---
name: pr-visuals
description: >-
  Capture a looping GIF + a still PNG of a visual/rendering change and embed them
  in its pull request. Uses Functor's headless frame-capture path (no screen
  recording, deterministic via --fixed-time), assembles a GIF with PIL, hosts the
  binaries in a gist (pushed via git, the only token-friendly host for binary
  images), and embeds them in the PR body. Use whenever a change adds or alters
  something visible (a new example/scene, a rendering/material/lighting/camera
  feature, a shader) and you're opening or updating a PR — see CLAUDE.md.
---

# pr-visuals — capture & embed PR screenshots / GIFs

Produce a short looping GIF **and** a still PNG of a visual change and embed them
in its PR. Everything runs headlessly (no OS screen-recording permission, no
manual clicking) and deterministically.

## Context discipline (important)

Verifying the look means **reading PNGs** (image tokens) and running many capture
commands — that bloats the main context. So **delegate the whole capture → assemble
→ upload → embed flow to a subagent** and let it return only the final URLs +
markdown block. Do this with the `Agent` tool (`general-purpose`):

> Spawn a `general-purpose` subagent. Tell it to **read this file**
> (`.claude/skills/pr-visuals/SKILL.md`) for the full technique and execute it for
> the given target, then return the gist raw URLs and the markdown block it
> embedded. Pass it: the example/scene dir (e.g. `examples/synthwave`), a one-line
> caption, the animation period if known, and the PR number (or "current branch").

The rest of this file is the technique the subagent (or you, if running inline)
follows.

## 1. Prerequisites

- Built CLI: if `target/debug/functor` / `functor-runner` are missing, run
  `npm run build:cli` first (slow; only needed once).
- `python3` with PIL (`Pillow`) for GIF assembly + verification.
- Run multi-step shell blocks under **`bash`** (heredoc), not the default zsh —
  zsh misparses hex gist IDs like `832ee3…` as math expressions and aborts.

## 2. Capture stills

The CLI forwards capture flags to `functor-runner`:

```sh
./target/debug/functor -d examples/<name> run native \
  --capture-frame /ABSOLUTE/path/shot.png --fixed-time 2.0 --capture-time 0.8
```

- **The capture path must be ABSOLUTE.** It's resolved against the *game's*
  working dir (`examples/<name>`), so a relative path lands in the wrong place / fails.
- `--fixed-time T` pins the game clock so the pose is deterministic (byte-identical
  PNGs) — use it for stills and every GIF frame.
- `--capture-time` is wall-clock seconds before the shot; `0.8` is enough for the
  window/GL to initialize.
- `--capture-frame` implies `--hidden`: the GL window is never shown and never
  steals focus or the cursor, so captures are safe to run while the user works.
- Pick 1–2 flattering times for the stills.

## 3. Capture a looping GIF

Only single-frame capture exists, so loop over `--fixed-time` and stitch.

- **Seamless loop:** span exactly one period of the dominant animation. For a
  scroll/wave driven by `sin(k * t * speed)`, one period is `T = (2π / k) / speed`.
  Capture N frames (≈24) over `[0, T)`. If you don't know the period, just capture
  a few seconds — a small loop seam is fine for a demo.
- Capture each frame to `/tmp/frames/frame_%02d.png` with `--capture-time 0.8`.
- Assemble + downscale with PIL (keeps the file small; ~600px wide is plenty):

```python
from PIL import Image
import glob
files = sorted(glob.glob('/tmp/frames/frame_*.png'))
W = 600
frames = []
for f in files:
    im = Image.open(f).convert('RGB')
    frames.append(im.resize((W, int(im.height * W / im.width)), Image.LANCZOS))
frames[0].save('/tmp/demo.gif', save_all=True, append_images=frames[1:],
               duration=70, loop=0, optimize=True, disposal=2)
```

## 4. Before/after (when the change MODIFIES an existing visual)

If the change alters an *existing* rendered scene (not a net-new feature), include
a **before/after** so reviewers see the delta — a one-sided "after" can't show what
improved. Capture both at the **same `--fixed-time`** so only the change differs.

- **Cheapest "before":** if the base ref already commits a golden at that fixed
  time (`examples/<name>/golden/<name>-t2.png`), just reuse it — no rebuild.
- **Otherwise build the base ref in a throwaway worktree** (doesn't disturb your
  branch):

  ```bash
  BASE=$(gh pr view --json baseRefName -q .baseRefName 2>/dev/null || echo main)
  git worktree add /tmp/before "$BASE"
  ( cd /tmp/before && ./target/debug/functor -d examples/<name> run native \
      --capture-frame /tmp/before.png --fixed-time 2.0 --capture-time 0.8 )
  git worktree remove /tmp/before --force
  ```

- Compose the two into one labeled side-by-side PNG (renders everywhere, one
  attachment):

  ```python
  from PIL import Image, ImageDraw, ImageFont
  b = Image.open('/tmp/before.png').convert('RGB'); a = Image.open('/tmp/after.png').convert('RGB')
  H = 440; fit = lambda im: im.resize((int(im.width*H/im.height), H), Image.LANCZOS)
  b, a = fit(b), fit(a); gap = 8
  c = Image.new('RGB', (b.width+gap+a.width, H+28), (18,8,28)); c.paste(b,(0,28)); c.paste(a,(b.width+gap,28))
  d = ImageDraw.Draw(c); f = ImageFont.truetype("/System/Library/Fonts/Supplemental/Arial Bold.ttf", 20)
  d.text((8,4), "BEFORE", fill=(255,255,255), font=f); d.text((b.width+gap+8,4), "AFTER", fill=(255,255,255), font=f)
  c.save('/tmp/beforeafter.png', optimize=True)
  ```

  Host it with the GIF (host step) and embed it under a "Before → after" heading
  (embed step). **Skip before/after for net-new features** — there's no prior state.

## 5. Verify

Read a couple of frames (and/or the GIF) back to confirm the look, and confirm the
animation actually moves (diff two frames with `PIL.ImageChops.difference(...).getbbox()`
— `None` means identical). Fix framing/timing and re-capture before uploading.

## 6. Host the media in a gist (binary-safe)

The browser drag-drop `user-attachments` host needs your web session cookies — not
reachable with a token. A **gist works** as a token-based equivalent, BUT you must
push binaries via **git** — `gh gist create` flat-out **rejects** a binary file
(`binary file not supported`), so create the gist with a small **text placeholder**
first, then git-push the images into it. Gist raw URLs serve the correct `image/*`
content-type, so they render inline in markdown.

```bash
bash <<'EOF'
set -e
# Create with a TEXT placeholder (gh gist create rejects binary), then git-push the media.
printf '# <repo> <feature> — media for PR #<N>\n' > /tmp/gist-readme.md
URL=$(gh gist create --desc "<repo> <feature> — media for PR #<N>" /tmp/gist-readme.md 2>&1 | tail -1)
GID=$(basename "$URL"); USER=$(gh api user -q .login); TOKEN=$(gh auth token)
rm -rf /tmp/gist && git clone "https://x-access-token:$TOKEN@gist.github.com/$GID.git" /tmp/gist
cp /tmp/demo.gif /tmp/shot.png /tmp/gist/
git -C /tmp/gist add -A
git -C /tmp/gist -c user.email="$(git config user.email)" -c user.name="$(git config user.name)" commit -qm "media"
git -C /tmp/gist push -q
for f in demo.gif shot.png; do
  RAW="https://gist.githubusercontent.com/$USER/$GID/raw/$f"
  echo "$RAW  [$(curl -sIL "$RAW" | grep -i '^content-type:' | tr -d '\r')]"
done
EOF
```

Confirm each raw URL reports `content-type: image/gif` / `image/png` (proof the
binary survived and markdown will render it).

## 7. Embed in the PR body

**Do NOT use `gh pr edit --body-file`** here — it hits a Projects-classic GraphQL
path that errors out and silently leaves the body unchanged. Use the REST API:

```bash
# fetch current body, insert the media markdown, write to /tmp/body.md, then:
gh api -X PATCH repos/<owner>/<repo>/pulls/<N> -F body=@/tmp/body.md -q .html_url
```

Markdown to embed (a caption reads nicely under the GIF):

```markdown
![<feature> demo](<gif raw url>)

<sub>Short caption. Rendered headlessly via `--capture-frame` / `--fixed-time`.</sub>

![still](<png raw url>)
```

## 8. Report

Return the gist URL, the raw image URLs, **and the local media paths** (keep the
captured files on disk, e.g. `/tmp/demo.gif` + the stills, don't delete them), plus
a confirmation that the PR body was updated (verify with
`gh pr view <N> --json body -q .body | grep gist`).

## 9. Hand the media to the review (visual verification)

The point of the media isn't just decoration — a reviewer should confirm the
render **actually exercises the feature and looks correct**. After embedding, run
`/xreview` and pass the local media paths so **both** image-capable reviewers
analyze them against the change's claims (e.g. "the GIF should show the grid
scrolling toward the camera with a glowing sun on the horizon"):

```
/xreview --media /tmp/demo.gif,/tmp/shot-t0.png,/tmp/shot-t2.png
```

Pass the GIF **plus two stills captured at different `--fixed-time` values** — a
single image is one frame, so motion (e.g. "scrolling") must be evidenced by
distinct stills. Both engines do the visual check (Claude via `Read`, Codex via
`-i`), so a visual issue both raise is high-confidence. Treat a reviewer that
can't see the claimed feature in the media as a finding to resolve.
