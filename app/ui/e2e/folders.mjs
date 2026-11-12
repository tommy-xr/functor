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
  // One control: the picker names the project; its menu switches, reopens, opens, creates.
  const picker = explorer.getByRole("button", { name: /^Project: / });
  const menu = page.getByRole("menu", { name: "Projects" });
  const choose = async (name) => {
    await picker.click();
    await menu.getByRole("menuitem", { name }).click();
  };
  try {
    await page.goto(server.url);
    await expect(titlebar).toContainText("counter");
    await expect(picker).toHaveAccessibleName("Project: games/counter");

    // The menu by keyboard: the repo's projects first, arrows wrap, Escape closes back to the picker.
    await picker.focus();
    await page.keyboard.press("Enter");
    const projects = menu.getByRole("group", { name: "Projects in this repo" });
    await expect(projects).toContainText("2 projects in this repo");
    await expect(projects.getByRole("menuitem")).toHaveText(["games/counter", "games/second"]);
    await expect(projects.getByRole("menuitem", { name: "games/counter" })).toHaveAttribute("aria-current", "true");
    await expect(projects.getByRole("menuitem").first()).toBeFocused();
    await page.keyboard.press("ArrowUp");
    await expect(menu.getByRole("menuitem", { name: "New project…" })).toBeFocused();
    await shot(page, `folders-${tag}-menu`);
    await page.keyboard.press("Escape");
    await expect(menu).toBeHidden();
    await expect(picker).toBeFocused();

    // By path: the server suggests directories under what is typed.
    await choose("Open folder…");
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
    await expect(picker).toHaveAccessibleName("Project: mygame");
    await picker.click();
    await expect(menu.getByRole("group", { name: "Projects in this folder" })).toContainText("The only project in this folder");
    await page.keyboard.press("Escape");
    expect(app.state.tracks_changes, "a loose folder tracks no changes").toBe(false);
    await expect(field).toBeHidden();

    // Escape puts the path field away, back to the picker.
    await choose("Open folder…");
    await expect(field).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(field).toBeHidden();
    await expect(picker).toBeFocused();

    // A path that is not a folder says why, and cannot be opened.
    await choose("Open folder…");
    await field.fill(join(empty, "nope"));
    await expect(explorer.locator(".open-folder-form .reason")).toHaveText("it does not exist");
    await expect(explorer.getByRole("button", { name: "Open", exact: true })).toBeDisabled();

    // An empty folder: its own state, offering a new project or another folder.
    await field.fill(empty);
    await explorer.getByRole("button", { name: "Open", exact: true }).click();
    const none = page.locator(".no-project");
    await expect(none).toContainText("No Functor project here");
    await expect(none).toContainText(empty);
    await expect(picker).toHaveAccessibleName("Project: No projects in this folder");
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

    // Back through the picker's recent projects: the repo, both projects in the picker.
    await picker.click();
    await expect(menu.getByRole("group", { name: "Recent projects" }).getByRole("menuitem")).toHaveText([/^mygame/, /^counter/]);
    await shot(page, `folders-${tag}-recent`);
    await menu.getByRole("menuitem", { name: "Open counter" }).click();
    await expect(titlebar).toContainText("counter");
    await expect.poll(() => app.state.repo_root).toBe(repo);
    await expect(picker).toHaveAccessibleName("Project: games/counter");
    expect(app.state.tracks_changes, "the repo tracks changes").toBe(true);

    // Switching project through the picker.
    await choose("games/second");
    await expect(picker).toHaveAccessibleName("Project: games/second");
    await expect.poll(() => app.state.active).toBe("games/second");
    await choose("games/counter");
    await expect(picker).toHaveAccessibleName("Project: games/counter");

    // A reopened page shows the same workspace.
    await reopen(page, server.url);
    await expect(titlebar).toContainText("counter");
    await expect(picker).toHaveAccessibleName("Project: games/counter");

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
    await expect(picker).toHaveAccessibleName("Project: No folder open");
    // Open and new stay in reach from the sidebar too.
    await picker.click();
    await expect(menu.getByRole("menuitem")).toContainText(["Open folder…", "New project…"]);
    await page.keyboard.press("Escape");
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
