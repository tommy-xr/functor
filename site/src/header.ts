// The shared site header, rendered at BUILD time (see build.mjs) into every
// page that carries an `<!--@header-->` block. Build-only — no client bundle
// imports this, so the header stays plain markup in the shipped HTML: no
// runtime cost and no pop-in on the landing page.
//
// Pages differ along three axes, which are exactly this module's inputs:
//   prefix   — "" at the site root, "../" from docs/ and manual/
//   suffix   — the pink wordmark tag ("SANDBOX", "IDE", "DOCS", "MANUAL")
//   active   — which nav entry gets aria-current
// plus a `controls` slot for the sandbox/IDE toolbars, which the page owns
// (it is the block's inner markup — see injectHeader).
//
// The burger markup is emitted on every page; CSS decides where it shows
// (styles.css: `.landing-page` opts in at <=620px, `.docs-page` wraps the nav
// into a scrollable row at <=720px instead).

// The Functor mark — the same art as the favicon and the VS Code file icon
// (docs/media/functor-icon.svg).
const MARK =
  '<svg class="wordmark-mark" viewBox="366 163 385 690" fill="currentColor" aria-hidden="true">' +
  '<path d="M382 341 545 179H735L615 300H548L382 466Z"/>' +
  '<path d="M382 493 497 380V748L382 837Z"/>' +
  '<path d="M518 426H693L597 522H518Z"/></svg>';

const GITHUB_MARK =
  '<svg viewBox="0 0 16 16" fill="currentColor" aria-hidden="true"><path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0 0 16 8c0-4.42-3.58-8-8-8Z"/></svg>';

const BURGER = `<input type="checkbox" id="nav-toggle" class="nav-toggle" />
      <label class="nav-burger" for="nav-toggle" aria-label="Toggle menu">
        <svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true">
          <line x1="4" y1="7" x2="20" y2="7" /><line x1="4" y1="12" x2="20" y2="12" /><line x1="4" y1="17" x2="20" y2="17" />
        </svg>
      </label>`;

// The one nav model for the whole site. The IDE is deliberately absent: it is
// still in development, so it ships unlinked and is reached by typing its URL.
// The landing footer (index.html) mirrors this list — keep them in step.
const NAV = [
  { id: "sandbox", href: "sandbox.html", label: "Sandbox" },
  { id: "manual", href: "manual/", label: "Manual" },
  { id: "docs", href: "docs/", label: "API Reference" },
];

const GITHUB = "https://github.com/tommy-xr/functor";
export const ALPHA_BADGE_TITLE =
  "Functor is alpha software — everything may change between releases";

/** The attributes a page may set on its `<!--@header-->` marker. */
type HeaderKey = "prefix" | "suffix" | "active";

const KEYS: ReadonlySet<string> = new Set<HeaderKey>(["prefix", "suffix", "active"]);
const isHeaderKey = (key: string): key is HeaderKey => KEYS.has(key);
const NAV_IDS = new Set(NAV.map(({ id }) => id));

type HeaderOptions = Partial<Record<HeaderKey, string>> & { controls?: string };

// The badge ships as the literal "alpha"; build.mjs stamps the release tag or
// PR head SHA over it afterwards (one regex — which is why a page may carry
// only ONE header block; injectHeader enforces that).
const renderHeader = ({
  prefix = "",
  suffix = "",
  active = "",
  controls = "",
}: HeaderOptions): string => {
  const nav = [
    ...NAV.map(
      ({ id, href, label }) =>
        `<a href="${prefix}${href}"${id === active ? ' aria-current="page"' : ""}>${label}</a>`
    ),
    `<a class="nav-github" href="${GITHUB}" aria-label="Functor on GitHub" title="GitHub">${GITHUB_MARK}</a>`,
  ];

  const children = [
    `<a class="wordmark" href="${prefix || "./"}">${MARK}FUNCTOR${
      suffix ? `<span class="wordmark-accent">//${suffix}</span>` : ""
    }</a>`,
    `<span class="version-badge" title="${ALPHA_BADGE_TITLE}">alpha</span>`,
    BURGER,
    ...(controls ? [controls] : []),
    `<nav class="site-nav">\n        ${nav.join("\n        ")}\n      </nav>`,
  ];

  return `<header class="site-header">\n      ${children.join("\n      ")}\n    </header>`;
};

// A page's header block: `<!--@header attr="…"--> …controls… <!--/@header-->`.
// The leading indentation is deliberately NOT consumed, so the emitted
// `<header>` keeps the page's own indentation (and matches the closing tag
// this module renders).
const BLOCK = /<!--@header([^>]*?)-->([\s\S]*?)<!--\/@header-->/g;
const ATTR = /([a-z]+)="([^"]*)"/g;

// Replace each block with the rendered header; the block's inner markup
// becomes the controls slot, so page-specific toolbars stay in the page they
// belong to. `page` only names the file in error messages.
//
// Every failure mode throws rather than passing the page through unchanged: a
// mistyped marker would otherwise ship a page with NO header, and a mistyped
// attribute (`prefx="../"`) would ship silently-broken relative links — both
// with a green build.
export const injectHeader = (html: string, page = "page"): string => {
  let blocks = 0;

  const out = html.replace(BLOCK, (_match: string, rawAttrs: string, controls: string) => {
    blocks += 1;
    const options: Partial<Record<HeaderKey, string>> = {};
    // Consume the recognised attributes; anything left over is a typo (an
    // unknown key, or a form this parser does not accept such as single
    // quotes) and must fail the build rather than be ignored.
    const rest = rawAttrs
      .replace(ATTR, (_attr: string, key: string, value: string) => {
        if (!isHeaderKey(key)) {
          throw new Error(`${page}: unknown @header attribute "${key}"`);
        }
        options[key] = value;
        return "";
      })
      .trim();
    if (rest) throw new Error(`${page}: unparsed @header attributes: ${rest}`);
    if (options.active && !NAV_IDS.has(options.active)) {
      throw new Error(`${page}: @header active="${options.active}" is not a nav entry`);
    }
    return renderHeader({ ...options, controls: controls.trim() });
  });

  // One block per page: each rendered header carries `id="nav-toggle"` and a
  // version-badge span, and build.mjs stamps only the first badge.
  if (blocks > 1) throw new Error(`${page}: ${blocks} @header blocks — expected at most one`);
  // A marker surviving the replace means the block is malformed (unclosed, or
  // a `>` inside the attributes), which would ship a headerless page.
  if (/<!--\/?@header/.test(out)) throw new Error(`${page}: malformed @header block`);

  return out;
};
