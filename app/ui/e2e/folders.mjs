// Opening any folder (D23): by path with the server's suggestions, a loose
// project outside git, an empty folder's state, the recent projects menu,
// and the workspace restored on reopen and after a restart (no -d: the most
// recent project). Runs at 1440×900 and 700×900 in temp folders with a temp
// home.
//
//   npm run test:app-ui
//
// APP_UI_SHOTS=<dir> saves stills at both widths.
import { cpSync, mkdirSync, mkdtempSync, realpathSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { chromium, expect } from "@playwright/test";
import { git, reopen, requireBuilt, ROOT, shot, startServer, stopServer, tempHome, watchErrors, watchExternal, watchState } from "./app.mjs";

requireBuilt();

const temps = [];
process.on("exit", () => temps.forEach((dir) => rmSync(dir, { recursive: true, force: true })));

/** A fresh temp dir (canonical: the app shows folders resolved), removed at exit. */
function temp(prefix) {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), prefix)));
  temps.push(dir);
  return dir;
}

/** A copy of examples/counter at `dir`. */
function counter(dir) {
  mkdirSync(dir, { recursive: true });
  cpSync(join(ROOT, "examples/counter"), dir, { recursive: true });
  return dir;
}

async function folders(browser, width, height) {
  const tag = `${width}x${height}`;
  // A repo with two projects, a loose project outside git, an empty folder.
  const repo = temp("functor-app-e2e-repo-");
  counter(join(repo, "games/counter"));
  counter(join(repo, "games/second"));
  git(repo, "init", "-q", "-b", "main");
  git(repo, "add", "-A");
  git(repo, "commit", "-q", "-m", "first");
  const loose = counter(join(temp("functor-app-e2e-loose-"), "mygame"));
  const empty = temp("functor-app-e2e-empty-");
  const home = tempHome();

  let server = await startServer(0, { dir: join(repo, "games/counter"), home });
  const page = await browser.newPage({ viewport: { width, height } });
  const app = watchState(page);
  const errors = watchErrors(page);
  const external = watchExternal(page);
  const titlebar = page.locator(".titlebar");
  const explorer = page.getByRole("complementary", { name: "Explorer" });
  const field = explorer.getByRole("textbox", { name: "Folder path" });
  try {
    await page.goto(server.url);
    await expect(titlebar).toContainText("counter");
    await expect(explorer).toContainText("One of 2 projects in this repo");

    // By path: the server suggests directories under what is typed.
    await explorer.getByRole("button", { name: "Open folder…" }).click();
    await expect(field).toBeFocused();
    await field.fill(`${loose.slice(0, -"mygame".length)}my`);
    const suggestions = explorer.getByRole("list", { name: "Folders" });
    await expect(suggestions.getByRole("button")).toHaveText(["mygame/"]);
    await shot(page, `folders-${tag}-suggestions`);
    await suggestions.getByRole("button", { name: loose }).click();
    await expect(field).toHaveValue(`${loose}/`);
    await explorer.getByRole("button", { name: "Open", exact: true }).click();
    await expect(titlebar).toContainText("mygame");
    await expect(page).toHaveTitle("Functor — mygame");
    await expect(explorer).toContainText("The only project in this folder");
    expect(app.state.tracks_changes, "a loose folder tracks no changes").toBe(false);
    await expect(field).toBeHidden();

    // A path that is not a folder says why, and cannot be opened.
    await explorer.getByRole("button", { name: "Open folder…" }).click();
    await field.fill(join(empty, "nope"));
    await expect(explorer.locator(".open-folder-form .reason")).toHaveText("it does not exist");
    await expect(explorer.getByRole("button", { name: "Open", exact: true })).toBeDisabled();

    // An empty folder: its own state, offering a new project or another folder.
    await field.fill(empty);
    await explorer.getByRole("button", { name: "Open", exact: true }).click();
    const none = page.locator(".no-project");
    await expect(none).toContainText("No Functor project here");
    await expect(none).toContainText(empty);
    await expect(explorer).toContainText("No projects in this folder");
    await expect(page).toHaveTitle(`Functor — ${empty.split("/").pop()}`);
    // "New project here" fills the new-project modal with this folder.
    await none.getByRole("button", { name: "New project here" }).click();
    const dialog = page.getByRole("dialog", { name: "New project" });
    await expect(dialog.getByRole("textbox", { name: "Name" })).toHaveValue(empty.split("/").pop());
    await expect(dialog.getByRole("textbox", { name: "Parent folder" })).toHaveValue(empty.slice(0, empty.lastIndexOf("/")));
    await dialog.getByRole("button", { name: "Cancel" }).click();
    await expect(dialog).toBeHidden();
    await expect(none.getByRole("region", { name: "Recent projects" }).getByRole("button")).toHaveText([
      /^mygame/,
      /^counter/,
    ]);
    await shot(page, `folders-${tag}-empty`);

    // Back through the Recent menu: the repo, both projects in the switcher.
    await explorer.getByRole("button", { name: "Recent" }).click();
    const menu = page.getByRole("menu", { name: "Recent projects" });
    await expect(menu.getByRole("menuitem")).toHaveText([/^mygame/, /^counter/]);
    await shot(page, `folders-${tag}-recent`);
    await menu.getByRole("menuitem", { name: "Open counter" }).click();
    await expect(titlebar).toContainText("counter");
    await expect.poll(() => app.state.repo_root).toBe(repo);
    await expect(explorer.getByRole("combobox")).toHaveValue("games/counter");
    expect(app.state.tracks_changes, "the repo tracks changes").toBe(true);

    // A reopened page shows the same workspace.
    await reopen(page, server.url);
    await expect(titlebar).toContainText("counter");
    await expect(explorer).toContainText("One of 2 projects in this repo");

    // Restarted with no -d where there is no project: the most recent opens.
    await stopServer(server);
    server = await startServer(0, { dir: null, cwd: empty, home, token: server.token });
    await reopen(page, server.url);
    await expect(titlebar).toContainText("counter");
    await expect.poll(() => app.state.folder).toBe(join(repo, "games/counter"));

    // With nothing to open at all: no folder.
    await stopServer(server);
    server = await startServer(0, { dir: null, cwd: empty, token: server.token });
    await reopen(page, server.url);
    await expect(page.locator(".no-project")).toContainText("No folder open");
    await expect(explorer).toContainText("No folder open");
    await expect(page.locator(".no-project").getByRole("textbox", { name: "Folder path" })).toBeVisible();
    await shot(page, `folders-${tag}-no-folder`);

    expect(await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth), "horizontal overflow (px)").toBeLessThanOrEqual(0);
    expect(errors, "console errors").toEqual([]);
    expect(external, "requests to non-loopback hosts").toEqual([]);
    console.log(`ok: folders ${tag}`);
  } finally {
    await page.close();
    await stopServer(server);
  }
}

const browser = await chromium.launch();
try {
  await folders(browser, 1440, 900);
  await folders(browser, 700, 900);
} catch (e) {
  console.error(`FAIL: ${e.message}`);
  process.exitCode = 1;
} finally {
  await browser.close();
}
