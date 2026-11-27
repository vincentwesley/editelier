// Generic house checks. Set PATHS to the project's pages. Needs @playwright/test and @axe-core/playwright.
// Preview servers do not apply _headers/_redirects: those are unit-tested in tests/redirects.test.ts.
import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

const PATHS = ["/"]; // add each new page

for (const path of PATHS) {
  for (const theme of ["dark", "light"] as const) {
    test(`${path}: axe, same-origin only, no overflow (${theme})`, async ({ page, baseURL }) => {
      const off: string[] = [];
      page.on("request", (r) => {
        if (r.url().startsWith("http") && !r.url().startsWith(baseURL!)) off.push(r.url());
      });
      await page.setViewportSize({ width: 375, height: 812 });
      await page.addInitScript((t) => localStorage.setItem("theme", t), theme);
      await page.goto(path);
      if (theme === "light") await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      await page.waitForTimeout(800); // let entrance animations settle before axe
      const bad = (await new AxeBuilder({ page }).analyze()).violations.filter((v) =>
        ["serious", "critical"].includes(v.impact ?? ""),
      );
      expect(bad).toEqual([]);
      expect(off).toEqual([]);
    });
  }

  test(`${path}: reduced motion runs no animations`, async ({ page }) => {
    await page.emulateMedia({ reducedMotion: "reduce" });
    await page.goto(path);
    await page.mouse.wheel(0, 2000);
    await page.locator("a:not(.hs-skip), button").first().hover();
    expect(await page.evaluate(() => document.getAnimations().length)).toBe(0);
  });
}

// About shares the Welcome footer's credits card (moved in while open, D13).
const openAbout = async (page: import("@playwright/test").Page) => {
  await page.goto("/");
  await page.locator("header.top").getByRole("button", { name: "About" }).click();
  return page.locator("#about-dialog");
};

for (const theme of ["dark", "light"] as const) {
  test(`About carries the credits card and draggable logo, axe clean (${theme})`, async ({ page }) => {
    await page.addInitScript((t) => localStorage.setItem("theme", t), theme);
    const about = await openAbout(page);
    await expect(about.locator(".creator .creator__art img")).toBeVisible();
    await expect(about.locator(".creator__fx")).toBeAttached();
    await expect(about.getByRole("link", { name: /@portus_workstation/ })).toBeVisible();
    await expect(about.getByRole("link", { name: /@wason25_pz/ })).toBeVisible();
    await expect(about.getByText(/LGPLv2\.1/)).toBeVisible();
    await about.locator('input[name="logo-form"][value="long"]').check();
    await expect(page.locator("html")).toHaveAttribute("data-logo", "long");
    await page.waitForTimeout(800);
    const bad = (await new AxeBuilder({ page }).include("#about-dialog").analyze()).violations.filter((v) => ["serious", "critical"].includes(v.impact ?? ""));
    expect(bad).toEqual([]);
    await page.keyboard.press("Escape");
    await expect(page.locator(".welcome__footer .creator")).toHaveCount(1);
    await expect(about.locator(".creator")).toHaveCount(0);
  });
}

test("About with reduced motion runs no animations", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  const about = await openAbout(page);
  await expect(about.locator(".creator")).toBeVisible();
  expect(await page.evaluate(() => document.getAnimations().length)).toBe(0);
});