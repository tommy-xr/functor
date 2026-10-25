# Built-in fonts

Subsets of SIL Open Font License fonts, renamed because subsetting makes a
Modified Version and both licenses reserve their original names. The copyright
and license strings stay in each file's `name` table (IDs 0, 13, 14); the full
licenses are beside them.

| File | Derived from | License |
| --- | --- | --- |
| `FunctorScript-Regular.ttf` | Courgette Regular (Sorkin Type Co) | `OFL-Courgette.txt` |
| `FunctorSans-Regular.ttf` | Lato Regular (Łukasz Dziedzic) | `OFL-Lato.txt` |

Regenerate (fonttools; sources from github.com/google/fonts `ofl/courgette`, `ofl/lato`):

```sh
pyftsubset Courgette-Regular.ttf \
  --unicodes="U+0020-007E,U+00A0-00FF,U+2013-2014,U+2018-201D,U+2022,U+2026" \
  --layout-features='kern,liga' --name-IDs='*' --name-languages='*' \
  --output-file=sub.ttf
python - sub.ttf FunctorScript-Regular.ttf "Functor Script" FunctorScript-Regular <<'PY'
import sys
from fontTools.ttLib import TTFont
src, dst, family, ps = sys.argv[1:]
font = TTFont(src)
for rec in font["name"].names:
    if rec.nameID in (1, 16): rec.string = family
    elif rec.nameID in (3, 4): rec.string = f"{family} Regular"
    elif rec.nameID == 6: rec.string = ps
    elif rec.nameID in (2, 17): rec.string = "Regular"
font.save(dst)
PY
```

(Same for Lato → `FunctorSans-Regular.ttf`, "Functor Sans".)
