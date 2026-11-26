// Editor flows against `vite preview` with the in-memory mock backend (no Tauri). Desktop sizes only.
import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

test.skip(({ viewport }) => (viewport?.width ?? 0) < 1000, "the editor is a desktop app");
test.use({ viewport: { width: 1280, height: 720 } });

const clips = (page: Page) => page.locator("#tl-clips li");

async function start(page: Page, theme: "dark" | "light" = "dark") {
  await page.addInitScript((t) => localStorage.setItem("theme", t), theme);
  await page.goto("/");
  await page.getByRole("button", { name: "Start editing" }).click();
  await expect(page.locator("#editor")).toBeVisible();
}

async function importAndAppend(page: Page) {
  await page.locator(".bin .panel-head").getByRole("button", { name: "Import" }).click();
  await expect(page.locator("#bin-list > li")).toHaveCount(4);
  await page.getByRole("button", { name: "Add Beach day 4K.mp4 to the timeline" }).click();
  await expect(clips(page)).toHaveCount(1);
}

test("welcome → editor: format sets the canvas, coming-from sets the keymap", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "What are you making?" })).toBeVisible();
  await page.getByText("Short, Reel or TikTok").click();
  await page.getByText("Premiere Pro").click();
  await page.locator("#project-name-new").fill("My short");
  await page.getByRole("button", { name: "Start editing" }).click();
  await expect(page.locator("#welcome")).toBeHidden();
  await expect(page.getByRole("textbox", { name: "Project name" })).toHaveValue("My short");
  await expect(page.locator("#preview")).toHaveAttribute("data-aspect", "9:16");
  await expect(page.locator("#bin-empty")).toContainText("Drop clips, music or photos here");
  await page.keyboard.press("Shift+Slash");
  const sheet = page.getByRole("dialog", { name: "Keyboard shortcuts" });
  await expect(sheet).toBeVisible();
  await sheet.getByRole("searchbox").fill("split");
  await expect(sheet.locator("tbody tr")).toHaveCount(1);
  await expect(sheet.locator("tbody")).toContainText("Ctrl+K");
  await page.keyboard.press("Escape"); // first Esc clears the search box (native), the second closes
  await page.keyboard.press("Escape");
  await expect(sheet).toBeHidden();
});

test("import → append → split → undo, plus drag from the bin", async ({ page }) => {
  await start(page);
  await importAndAppend(page);
  await expect(page.locator("#bin-list")).toContainText("4K");
  await expect(page.locator("#bin-list")).toContainText("VFR");
  await expect(page.locator("#bin-list")).toContainText("Proxy");

  for (let i = 0; i < 45; i++) await page.keyboard.press("ArrowRight");
  await expect(page.locator("#timecode")).toHaveText("00:00:01;15");
  await page.keyboard.press("s");
  await expect(clips(page)).toHaveCount(2);
  await expect(page.getByRole("button", { name: "Undo" })).toHaveAttribute("title", "Undo Split clip");
  await page.getByRole("button", { name: "Undo" }).click();
  await expect(clips(page)).toHaveCount(1);
  await page.keyboard.press("Control+Shift+Z");
  await expect(clips(page)).toHaveCount(2);

  await page.locator('[data-media]', { hasText: "Interview.mp4" }).dragTo(page.locator("#tl-scroll"), { targetPosition: { x: 5, y: 100 } });
  await expect(clips(page)).toHaveCount(3);
  await expect(clips(page).first()).toContainText("Interview.mp4");
});

test("titles and music show in the inspector", async ({ page }) => {
  await start(page);
  await importAndAppend(page);
  await page.getByRole("button", { name: "Add title", exact: true }).click();
  await expect(page.getByLabel("Text")).toHaveValue("Your title");
  await page.getByLabel("Text").fill("Hello there");
  await page.getByLabel("Font").selectOption("Inter");
  await expect(clips(page).filter({ hasText: "Title: Hello there" })).toHaveCount(1);
  await page.getByRole("button", { name: "Add Theme song.mp3 to the timeline" }).click();
  await expect(clips(page).filter({ hasText: "Music: Theme song.mp3" })).toHaveCount(1);
});

test("export dialog: smart default, progress, show in folder", async ({ page }) => {
  await start(page);
  await importAndAppend(page);
  await page.getByRole("button", { name: "Export" }).first().click();
  const dlg = page.getByRole("dialog", { name: "Export" });
  await expect(dlg).toBeVisible();
  await expect(dlg.getByRole("radio", { name: /YouTube 1080p/ })).toBeChecked();
  await expect(dlg.getByLabel("File name")).toHaveValue("Untitled project.mp4");
  await expect(dlg).toContainText("Encoder: Auto");
  await dlg.getByRole("button", { name: "Export", exact: true }).click();
  await expect(dlg.locator("#export-status")).toContainText("Exported to Videos\\Untitled project.mp4", { timeout: 10_000 });
  await expect(dlg.getByRole("button", { name: "Show in folder" })).toBeVisible();
});

for (const size of [
  { width: 1280, height: 720 },
  { width: 1920, height: 1200 },
]) {
  test(`editor fits ${size.width}x${size.height} with no overflow`, async ({ page }) => {
    await page.setViewportSize(size);
    await start(page);
    await importAndAppend(page);
    const bad = await page.evaluate(() => {
      const out: string[] = [];
      const de = document.documentElement;
      if (de.scrollWidth > innerWidth || de.scrollHeight > innerHeight) out.push(`page ${de.scrollWidth}x${de.scrollHeight}`);
      for (const el of document.querySelectorAll<HTMLElement>(".bar, .bin, .viewer, .inspector, .timeline, .tl-bar, .transport, #preview")) {
        const r = el.getBoundingClientRect();
        if (r.right > innerWidth + 0.5 || r.bottom > innerHeight + 0.5 || r.width < 1 || r.height < 1) out.push(`${el.className || el.id} ${JSON.stringify(r)}`);
      }
      return out;
    });
    expect(bad).toEqual([]);
    const preview = (await page.locator("#preview").boundingBox())!;
    expect(preview.width).toBeGreaterThan(400);
  });
}

for (const theme of ["dark", "light"] as const) {
  test(`axe: welcome and editor (${theme})`, async ({ page }) => {
    await page.addInitScript((t) => localStorage.setItem("theme", t), theme);
    await page.goto("/");
    await page.waitForTimeout(800); // let entrance animations settle before axe
    const serious = async () =>
      (await new AxeBuilder({ page }).analyze()).violations
        .filter((v) => ["serious", "critical"].includes(v.impact ?? ""))
        .map((v) => `${v.id}: ${v.nodes.map((n) => n.target).join(" ")}`);
    expect(await serious()).toEqual([]);
    await page.getByRole("button", { name: "Start editing" }).click();
    await importAndAppend(page);
    await page.locator("#tl-clips li").first().waitFor({ state: "attached" });
    await page.getByRole("button", { name: "Add title", exact: true }).click();
    expect(await serious()).toEqual([]);
    await page.getByRole("button", { name: "Export" }).first().click();
    expect(await serious()).toEqual([]);
  });
}
