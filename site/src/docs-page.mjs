// Pure page template for a built docs page. Given the page title, the
// pre-rendered sidebar + content HTML, and the rootPrefix (the relative path
// back to the site root from this page's URL — "../" for /docs/, "../../" for
// /docs/<slug>/), it returns the full HTML document.
//
// The .site-header markup is copied from the built pages so the nav stays
// consistent; Docs is marked current here. The .docs-layout / .docs-nav /
// .docs-main classes are the existing docs styles (styles.css), reused as-is.

export const renderDocsPage = ({ title, sidebar, content, rootPrefix }) => `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>functor docs — ${title}</title>
    <meta
      name="description"
      content="Get started with functor and learn Functor Lang — the tiny live-editable game language. Every full program here runs in the sandbox with one click."
    />
    <link rel="preconnect" href="https://fonts.googleapis.com" />
    <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin />
    <link
      href="https://fonts.googleapis.com/css2?family=Orbitron:wght@600;800&family=JetBrains+Mono:ital,wght@0,400;0,600;1,400&display=swap"
      rel="stylesheet"
    />
    <link rel="stylesheet" href="${rootPrefix}styles.css" />
  </head>
  <body class="docs-page">
    <header class="site-header">
      <a class="wordmark" href="${rootPrefix}">FUNCTOR<span class="wordmark-accent">//DOCS</span></a>
      <nav class="site-nav">
        <a href="${rootPrefix}sandbox.html">Sandbox</a>
        <a href="${rootPrefix}docs/" aria-current="page">Docs</a>
        <a href="https://github.com/tommy-xr/functor">GitHub ↗</a>
      </nav>
    </header>

    <div class="docs-layout">
      <nav class="docs-nav">
${sidebar}
      </nav>

      <main class="docs-main" data-pagefind-body>
${content}
      </main>
    </div>
  </body>
</html>
`;
