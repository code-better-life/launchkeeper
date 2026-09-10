// README 截图脚本 —— 这是仓库里 assets/screenshots/{zh,en}/*.png 的唯一来源。
// 换了界面之后重跑它，README 的图就更新了；不要手动截图往里塞。
//
// 用法：
//   1) 先起一个带 mock 数据的前端 dev server（不需要 Tauri 后端）：
//        VITE_MOCK=1 pnpm -C crates/launchkeeper-app dev --port 5183
//   2) 另开一个终端：
//        pnpm -C crates/launchkeeper-app screenshots
//      可用环境变量：
//        SHOT_URL      dev server 地址，默认 http://localhost:5183
//        SHOT_OUT      输出根目录，默认 <repo>/assets/screenshots
//        SHOT_LOCALES  要拍的语言，逗号分隔，默认 zh,en
//
// 为什么是 playwright-core 而不是 playwright：core 不下载自带浏览器（本机
// 内置盘紧张，也没必要多一份 Chromium），直接驱动系统里已装的 Google Chrome
// —— 先试 channel: "chrome"，不行再退回固定路径。
//
// 选择器一律用可见文本 / role，不用 CSS class：class 是样式的实现细节，改了
// 样式不该让截图脚本静默拍出错的画面。唯一的例外是 `.row`（任务行没有更好的
// 语义锚点），所以每次点行都用 hasText 限定到具体任务。

import { chromium } from "playwright-core";
import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";

const APP_DIR = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const REPO_ROOT = path.resolve(APP_DIR, "..", "..");

const URL = process.env.SHOT_URL ?? "http://localhost:5183";
const OUT_ROOT = process.env.SHOT_OUT ?? path.join(REPO_ROOT, "assets", "screenshots");
const LOCALES = (process.env.SHOT_LOCALES ?? "zh,en").split(",").map((s) => s.trim()).filter(Boolean);

const CHROME_FALLBACK = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";

/** mock 数据里的任务显示名（lib/mock.ts 的 EN_TEXT 决定英文那一列）。 */
const L = {
  zh: {
    langSegment: "中文",
    scheduledTask: "夜间报告同步",
    serviceTask: "审批 bridge",
    tabHistory: "运行历史",
    tabInsight: "AI 解读",
    settings: "设置",
    themeDark: "深色",
    themeSystem: "跟随系统",
    expandOthers: "个无法接管",
  },
  en: {
    langSegment: "English",
    scheduledTask: "Nightly report sync",
    serviceTask: "Approval bridge",
    tabHistory: "Run history",
    tabInsight: "AI insight",
    settings: "Settings",
    themeDark: "Dark",
    themeSystem: "System",
    expandOthers: "cannot be adopted",
  },
};

/** 界面是动画+异步加载的，等一小会儿再拍，避免拍到过渡中的一帧。 */
const settle = (page) => page.waitForTimeout(450);

async function shot(page, dir, name) {
  const file = path.join(dir, `${name}.png`);
  await settle(page);
  await page.screenshot({ path: file });
  console.log(`  ${path.relative(REPO_ROOT, file)}`);
}

/**
 * 打开设置面板。刻意不吃传进来的语言：进函数时界面可能还是另一种语言（第一次
 * 打开就是为了去切语言），所以两个 aria-label 都试一遍。
 */
async function openSettings(page) {
  for (const name of [L.zh.settings, L.en.settings]) {
    const btn = page.getByRole("button", { name, exact: true }).first();
    if (await btn.count()) {
      await btn.click();
      await settle(page);
      return;
    }
  }
  throw new Error("找不到设置按钮");
}

async function closeSettings(page) {
  await page.keyboard.press("Escape");
  await settle(page);
}

/** 分段控件用 aria-labelledby 锚定（Settings.svelte 里的 radiogroup）。 */
function segment(page, group, label) {
  return page.locator(`[aria-labelledby="${group}-label"] button`, { hasText: label });
}

async function selectTask(page, name) {
  await page.locator(".row", { hasText: name }).first().click();
  await settle(page);
}

async function captureLocale(page, lang) {
  const s = L[lang];
  const dir = path.join(OUT_ROOT, lang);
  await mkdir(dir, { recursive: true });
  console.log(`\n[${lang}]`);

  // 语言：不依赖"跟随系统"的结果，每次都显式钉死，否则换台机器就拍错语言。
  await openSettings(page);
  await segment(page, "language", s.langSegment).click();
  await settle(page);
  await closeSettings(page);

  // 详情是按"选中的任务名变了"才重新拉的（stores/tasks.svelte.ts 的 select()
  // 对同名任务直接 return）。换语言不会改任务名，所以先点一个别的任务，把上一
  // 轮语言的详情挤掉，否则英文那一遍右侧会留着中文的字段。
  await selectTask(page, s.serviceTask);

  // 1. 主界面 + 选中一个定时任务（配置标签页，只读概览）
  await selectTask(page, s.scheduledTask);
  await shot(page, dir, "main");

  // 2. 运行历史
  await page.getByRole("button", { name: s.tabHistory, exact: true }).click();
  await shot(page, dir, "runs");

  // 3. AI 解读（mock 里 report-sync 有一条存好的解读，直接显示，不会发请求）
  await page.getByRole("button", { name: s.tabInsight, exact: true }).click();
  await shot(page, dir, "ai-insight");

  // 4. 服务型任务详情（启动 / 停止 / 重启、keep-alive）
  await selectTask(page, s.serviceTask);
  await shot(page, dir, "service");

  // 5. 外部 agent 列表，把"无法接管"那一组展开
  const expand = page.locator("button", { hasText: s.expandOthers }).first();
  if (await expand.count()) {
    await expand.click();
    await settle(page);
  }
  await shot(page, dir, "agents");

  // 6. 设置面板
  await openSettings(page);
  await shot(page, dir, "settings");

  // 7. 深色模式：切到深色、关掉设置拍主界面，再切回跟随系统
  await segment(page, "theme", s.themeDark).click();
  await closeSettings(page);
  await selectTask(page, s.scheduledTask);
  await shot(page, dir, "dark");
  await openSettings(page);
  await segment(page, "theme", s.themeSystem).click();
  await closeSettings(page);
}

async function main() {
  let browser;
  try {
    browser = await chromium.launch({ channel: "chrome", headless: true });
  } catch {
    browser = await chromium.launch({ executablePath: CHROME_FALLBACK, headless: true });
  }
  const page = await browser.newPage({ viewport: { width: 1280, height: 820 } });
  try {
    await page.goto(URL, { waitUntil: "networkidle" });
    await page.locator(".row").first().waitFor({ timeout: 15_000 });
    for (const lang of LOCALES) await captureLocale(page, lang);
  } finally {
    await browser.close();
  }
}

await main();
