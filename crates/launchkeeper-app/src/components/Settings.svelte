<script lang="ts">
  // 设置面板（PRD §4.8 / M3 项 4「失败通知」和项 5「界面中英文切换」，
  // docs/M3-design.md §4、§5）。
  //
  // 现在「语言」和「失败通知」都是真的；菜单栏登录项、运行历史保留条数仍然是
  // 设计稿里画出来、后端还没有对应逻辑的占位——禁用 + 「即将支持」，免得看起来
  // 能点却什么都不做。数据目录是只读展示，不在这个表单里编辑（改目录要挪数据库
  // 和日志，不是切一个开关）。
  import { onMount } from "svelte";
  import { api } from "../lib/api";
  import { aiPrompt } from "../lib/aiPrompt";
  import { LINKS, coffeeReady } from "../lib/links";
  import { taskStore } from "../lib/stores/tasks.svelte";
  import type { AiConfig, AiProvider, AppSettings } from "../lib/bindings";
  import {
    LANGUAGE_SETTINGS,
    languageSetting,
    locale,
    setLocale,
    t,
    type LanguageSetting,
  } from "../lib/i18n";
  import { THEME_SETTINGS, setTheme, themeSetting, type ThemeSetting } from "../lib/theme";

  let { onclose }: { onclose: () => void } = $props();

  let version = $state("");
  onMount(() => {
    void api.appInfo().then((i) => (version = i.version)).catch(() => {});
  });
  function openLink(url: string) {
    void api.openUrl(url).catch((e) => {
      taskStore.showError(t("settings.about.open_failed", { error: String(e) }));
    });
  }

  // 后端字段在 IPC 上是 `#[serde(default)]`，类型里因此都是可选的；界面上
  // 永远按「已加载的具体值」显示，未加载完之前不猜。
  let settings = $state<AppSettings | null>(null);
  let dataDir = $state<string | null>(null);
  let loading = $state(true);
  // 每个开关各自的保存状态：一个正在保存不该锁住另一个。
  let savingTaskFailure = $state(false);
  let savingServiceCrash = $state(false);
  let savingLanguage = $state(false);
  let savingTheme = $state(false);

  // ---- M4 §4：外观 ----
  /** 当前选中的外观。`settings.theme` 的镜像，分段控件读它。 */
  let theme = $state<ThemeSetting>("system");

  // ---- M4 §1 / §4：AI ----
  // `ai.json` 不在 `settings.json` 里（M4 §1.3：CLI 也要能改它），所以这一段
  // 有自己的加载和保存路径，跟上面那些开关互不相干。
  /** 后端最后一次确认的那份，用来判断三个输入框是不是真的改了。 */
  let loadedAi = $state<AiConfig | null>(null);
  let provider = $state<AiProvider>("anthropic");
  let baseUrlDraft = $state("");
  let modelDraft = $state("");
  let savingAi = $state(false);
  /**
   * Key 的输入框是**只写**的：后端从不把 Key 交回来（M4 §1.3），所以这里存的
   * 永远是用户刚敲进去、还没保存的那一串，保存成功立刻清空。
   */
  let keyDraft = $state("");
  let hasKey = $state(false);
  let savingKey = $state(false);
  let testing = $state(false);
  /** 「测试连接」的结论，就地显示在按钮旁边（成功/失败都留在面板上）。 */
  let testResult = $state<{ ok: boolean; text: string } | null>(null);

  /** 两个 provider 的默认 base，只用来做输入框的 placeholder。 */
  const DEFAULT_BASE: Record<AiProvider, string> = {
    anthropic: "https://api.anthropic.com",
    openai_compatible: "https://api.openai.com/v1",
  };

  /** 分段控件里三个格子的文案，顺序就是 `LANGUAGE_SETTINGS`。 */
  const LANGUAGE_LABEL = {
    system: "settings.language.system",
    "zh-CN": "settings.language.zh",
    en: "settings.language.en",
  } as const;

  /** 同上，顺序就是 `THEME_SETTINGS`。 */
  const THEME_LABEL = {
    system: "settings.theme.system",
    light: "settings.theme.light",
    dark: "settings.theme.dark",
  } as const;

  /** 复制走的那段提示词，跟着界面语言走（M4 §4）。 */
  const promptText = $derived(aiPrompt(locale.current));

  onMount(() => {
    void load();
  });

  async function load() {
    loading = true;
    try {
      const [s, d] = await Promise.all([api.getSettings(), api.dataDir()]);
      settings = s;
      dataDir = d;
      // 面板打开时对一次表：别的窗口/别的进程改过 settings.json 的话，这里
      // 显示的应该是文件里的那个值，而不是本次会话开始时读到的那个。
      setLocale(languageSetting(s.language));
      theme = themeSetting(s.theme);
      setTheme(theme);
    } catch (e) {
      taskStore.showError(e);
    } finally {
      loading = false;
    }
    // AI 那半边单独加载：`ai.json` 读不出来（数据目录不在了）不该把语言和
    // 通知开关一起挡在加载态里。
    try {
      applyAiConfig(await api.getAiConfig());
      hasKey = await api.hasAiApiKey();
    } catch (e) {
      taskStore.showError(e);
    }
  }

  /** 把后端确认过的那份写进三个输入框，并记成「已保存的状态」。 */
  function applyAiConfig(c: AiConfig) {
    loadedAi = c;
    provider = c.provider ?? "anthropic";
    baseUrlDraft = c.base_url ?? "";
    modelDraft = c.model ?? "";
  }

  /** 输入框跟后端那份不一样了才值得写一次 `ai.json`。 */
  function aiDirty(): boolean {
    const base = baseUrlDraft.trim() ? baseUrlDraft.trim() : null;
    return (
      provider !== (loadedAi?.provider ?? "anthropic") ||
      base !== (loadedAi?.base_url ?? null) ||
      modelDraft.trim() !== (loadedAi?.model ?? "")
    );
  }

  /**
   * 存 `ai.json`。输入框 blur / 下拉框 change 时调，没改就直接返回。
   *
   * 配置一变，上一次「测试连接」的结论就不作数了，跟着清掉——一个绿勾指着
   * 另一个 base URL 比没有结论更误导人。
   */
  async function saveAiConfig() {
    if (savingAi || !aiDirty()) return;
    savingAi = true;
    try {
      applyAiConfig(
        await api.setAiConfig({
          provider,
          base_url: baseUrlDraft.trim() ? baseUrlDraft.trim() : null,
          model: modelDraft.trim(),
        }),
      );
      testResult = null;
    } catch (e) {
      // 写不进去就把输入框退回后端那份，免得界面显示的是一个没存住的值。
      if (loadedAi) applyAiConfig(loadedAi);
      taskStore.showError(e);
    } finally {
      savingAi = false;
    }
  }

  async function saveKey() {
    const key = keyDraft.trim();
    if (!key || savingKey) return;
    savingKey = true;
    try {
      await api.setAiApiKey(key);
      // 存进钥匙串了就别在输入框里留着——它是这个界面上唯一还能读到 Key 的
      // 地方，而后端从这一刻起就再也不会把它交回来了。
      keyDraft = "";
      hasKey = await api.hasAiApiKey();
      testResult = null;
    } catch (e) {
      taskStore.showError(e);
    } finally {
      savingKey = false;
    }
  }

  async function clearKey() {
    if (savingKey) return;
    savingKey = true;
    try {
      await api.clearAiApiKey();
      // 环境变量 `LAUNCHKEEPER_AI_API_KEY` 删不掉（M4 §1.3），所以「还有没有
      // Key」要重新问一次，不能想当然地置 false。
      hasKey = await api.hasAiApiKey();
      testResult = null;
    } catch (e) {
      taskStore.showError(e);
    } finally {
      savingKey = false;
    }
  }

  /** 真的发一次请求出去，可能等几秒；结论就地显示，不弹 toast。 */
  async function testConnection() {
    if (testing) return;
    testing = true;
    testResult = null;
    try {
      const reply = await api.testAiConnection();
      testResult = {
        ok: true,
        text: t("settings.ai.test_ok", { reply: reply.trim().slice(0, 120) }),
      };
    } catch (e) {
      testResult = { ok: false, text: e instanceof Error ? e.message : String(e) };
    } finally {
      testing = false;
    }
  }

  async function copyPrompt() {
    try {
      await navigator.clipboard.writeText(promptText);
      taskStore.showOk(t("settings.prompt.copied"));
    } catch (e) {
      taskStore.showError(
        new Error(
          t("settings.prompt.copy_failed", {
            message: e instanceof Error ? e.message : String(e),
          }),
        ),
      );
    }
  }

  /**
   * 把当前 `settings`（已经乐观地改过了）整份存回后端。
   *
   * 后端是整体替换，所以这里必须把这个面板**不编辑**的字段也原样带上：
   * `list_sort`（M4 §2）是列表自己存的，用户在这儿点一下通知开关不该把它的
   * 排序抹回默认。`theme`（M4 §4）虽然这个面板自己就在编辑，同样要每次带上
   * ——它跟语言、通知开关在同一份 JSON 里，漏一次就被抹回「跟随系统」。
   */
  async function persist(): Promise<AppSettings> {
    const s = settings!;
    return await api.setSettings({
      notify_on_task_failure: s.notify_on_task_failure ?? true,
      notify_on_service_crash: s.notify_on_service_crash ?? true,
      language: s.language ?? null,
      list_sort: s.list_sort ?? null,
      theme: s.theme ?? null,
    });
  }

  async function toggle(
    field: "notify_on_task_failure" | "notify_on_service_crash",
    value: boolean,
  ) {
    if (!settings) return;
    const saving = field === "notify_on_task_failure" ? "task" : "service";
    if (saving === "task") savingTaskFailure = true;
    else savingServiceCrash = true;
    // 乐观更新：开关立刻翻过去，失败了再弹 toast 并改回来——两个请求飞在
    // 路上时界面不该看起来卡住。
    const before = settings;
    settings = { ...settings, [field]: value };
    try {
      settings = await persist();
    } catch (e) {
      settings = before;
      taskStore.showError(e);
    } finally {
      if (saving === "task") savingTaskFailure = false;
      else savingServiceCrash = false;
    }
  }

  /**
   * 切换界面语言。
   *
   * 界面先换（`setLocale`），再去写 settings.json：这一步不需要重启，也不需要
   * 等后端确认——语言纯粹是前端的事，后端那份只是"下次打开时用哪个"以及托盘
   * 菜单读的那份。写失败就把界面和设置一起退回去，并弹 toast。
   */
  async function chooseLanguage(next: LanguageSetting) {
    if (!settings || savingLanguage || locale.setting === next) return;
    const before = settings;
    const beforeSetting = locale.setting;
    savingLanguage = true;
    setLocale(next);
    settings = { ...settings, language: next };
    try {
      settings = await persist();
    } catch (e) {
      settings = before;
      setLocale(beforeSetting);
      taskStore.showError(e);
    } finally {
      savingLanguage = false;
    }
  }

  /**
   * 换外观。和语言同一个套路：界面立刻变（`setTheme` 盖 `<html>` 上的
   * `data-theme` 并让 Tauri 窗口的标题栏跟上），再去写 settings.json；写失败
   * 就连界面一起退回去。
   */
  async function chooseTheme(next: ThemeSetting) {
    if (!settings || savingTheme || theme === next) return;
    const before = settings;
    const beforeTheme = theme;
    savingTheme = true;
    theme = next;
    setTheme(next);
    settings = { ...settings, theme: next };
    try {
      settings = await persist();
    } catch (e) {
      settings = before;
      theme = beforeTheme;
      setTheme(beforeTheme);
      taskStore.showError(e);
    } finally {
      savingTheme = false;
    }
  }

  function onWindowKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") onclose();
  }
</script>

<svelte:window onkeydown={onWindowKeydown} />

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="backdrop" onmousedown={onclose}>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="sheet"
    onmousedown={(e) => e.stopPropagation()}
    role="dialog"
    aria-modal="true"
    aria-labelledby="settings-title"
    tabindex="-1"
  >
    <header>
      <h2 id="settings-title">{t("settings.title")}</h2>
      <button class="close" onclick={onclose} aria-label={t("common.close")}>✕</button>
    </header>

    {#if loading}
      <p class="muted loading">{t("common.loading")}</p>
    {:else if settings}
      <div class="body">
        <section class="field">
          <span class="label" id="language-label">{t("settings.language")}</span>
          <!-- 三格分段控件（设计稿「设置」画板）：跟随系统 / 中文 / English。
               `<select>` 撑得住，但这是个只有三项、每项都短的选择，摆出来比
               藏在下拉框里少一次点击。 -->
          <div class="segmented" role="radiogroup" aria-labelledby="language-label">
            {#each LANGUAGE_SETTINGS as option (option)}
              <button
                type="button"
                class="segment"
                class:active={locale.setting === option}
                role="radio"
                aria-checked={locale.setting === option}
                disabled={savingLanguage}
                onclick={() => void chooseLanguage(option)}
              >
                {t(LANGUAGE_LABEL[option])}
              </button>
            {/each}
          </div>
        </section>

        <section class="field">
          <span class="label" id="theme-label">{t("settings.theme")}</span>
          <!-- 外观（M4 §4）：跟随系统 / 浅色 / 深色，和语言同一个分段控件。 -->
          <div class="segmented" role="radiogroup" aria-labelledby="theme-label">
            {#each THEME_SETTINGS as option (option)}
              <button
                type="button"
                class="segment"
                class:active={theme === option}
                role="radio"
                aria-checked={theme === option}
                disabled={savingTheme}
                onclick={() => void chooseTheme(option)}
              >
                {t(THEME_LABEL[option])}
              </button>
            {/each}
          </div>
        </section>

        <section class="field">
          <span class="label">{t("settings.data_dir")}</span>
          <div class="readonly-path" title={dataDir ?? ""}>
            {dataDir ?? t("common.placeholder")}
          </div>
        </section>

        <section class="field">
          <span class="label">{t("settings.notify")}</span>
          <label class="check">
            <input
              type="checkbox"
              checked={settings.notify_on_task_failure ?? true}
              disabled={savingTaskFailure}
              onchange={(e) => toggle("notify_on_task_failure", e.currentTarget.checked)}
            />
            {t("settings.notify.task")}
          </label>
          <label class="check">
            <input
              type="checkbox"
              checked={settings.notify_on_service_crash ?? true}
              disabled={savingServiceCrash}
              onchange={(e) => toggle("notify_on_service_crash", e.currentTarget.checked)}
            />
            {t("settings.notify.service")}
          </label>
          <p class="hint">{t("settings.notify.hint")}</p>
        </section>

        <!-- ---- AI（M4 §1 / §4）----
             这一格存的是 `<data_dir>/ai.json` + 钥匙串，不是 settings.json：
             CLI 的 `launchkeeper ai config` 改的是同一份，两边不会对"用哪个
             模型"有不同看法。 -->
        <section class="field">
          <span class="label">{t("settings.ai")}</span>

          <label class="sub" for="ai-provider">{t("settings.ai.provider")}</label>
          <select
            id="ai-provider"
            class="control"
            value={provider}
            disabled={savingAi}
            onchange={(e) => {
              provider = e.currentTarget.value as AiProvider;
              void saveAiConfig();
            }}
          >
            <option value="anthropic">{t("settings.ai.provider.anthropic")}</option>
            <option value="openai_compatible">{t("settings.ai.provider.openai")}</option>
          </select>

          <label class="sub" for="ai-base-url">{t("settings.ai.base_url")}</label>
          <input
            id="ai-base-url"
            class="control"
            type="text"
            spellcheck="false"
            autocomplete="off"
            bind:value={baseUrlDraft}
            placeholder={DEFAULT_BASE[provider]}
            disabled={savingAi}
            onblur={() => void saveAiConfig()}
          />
          <p class="hint">{t("settings.ai.base_url.hint")}</p>

          <label class="sub" for="ai-model">{t("settings.ai.model")}</label>
          <input
            id="ai-model"
            class="control"
            type="text"
            spellcheck="false"
            autocomplete="off"
            bind:value={modelDraft}
            disabled={savingAi}
            onblur={() => void saveAiConfig()}
          />

          <label class="sub" for="ai-key">{t("settings.ai.key")}</label>
          <!-- 只写：后端从不把 Key 交回来，所以这个框永远是空的，旁边那行
               「已保存 / 未配置」才是当前状态（M4 §1.3）。 -->
          <div class="row">
            <input
              id="ai-key"
              class="control grow"
              type="password"
              spellcheck="false"
              autocomplete="off"
              bind:value={keyDraft}
              placeholder={t("settings.ai.key.placeholder")}
              disabled={savingKey}
              onkeydown={(e) => {
                if (e.key === "Enter") void saveKey();
              }}
            />
            <button
              class="btn"
              disabled={savingKey || keyDraft.trim() === ""}
              onclick={() => void saveKey()}
            >
              {t("settings.ai.key.save")}
            </button>
          </div>
          <div class="row">
            <span class="state" class:on={hasKey}>
              {hasKey ? t("settings.ai.key.saved") : t("settings.ai.key.missing")}
            </span>
            {#if hasKey}
              <button class="link" disabled={savingKey} onclick={() => void clearKey()}>
                {t("settings.ai.key.clear")}
              </button>
            {/if}
          </div>
          <p class="hint">{t("settings.ai.key.hint")}</p>

          <div class="row">
            <button class="btn" disabled={testing} onclick={() => void testConnection()}>
              {testing ? t("settings.ai.testing") : t("settings.ai.test")}
            </button>
            {#if testResult}
              <span class="test" class:bad={!testResult.ok}>{testResult.text}</span>
            {/if}
          </div>
          <p class="hint">{t("settings.ai.hint")}</p>
        </section>

        <!-- ---- 给 AI 的提示词（M4 §4）----
             不是设置，是一段拿去粘贴的文本：让终端里的助手知道这台机器上有
             launchkeeper，以及先读 `launchkeeper docs`。 -->
        <section class="field">
          <span class="label" id="ai-prompt-label">{t("settings.prompt")}</span>
          <textarea
            class="control prompt"
            readonly
            rows="8"
            aria-labelledby="ai-prompt-label"
            value={promptText}
          ></textarea>
          <div class="row">
            <button class="btn" onclick={() => void copyPrompt()}>
              {t("settings.prompt.copy")}
            </button>
          </div>
          <p class="hint">{t("settings.prompt.hint")}</p>
        </section>

        <section class="field">
          <span class="label">{t("settings.menubar")}</span>
          <label class="check disabled">
            <input type="checkbox" disabled />
            {t("settings.menubar.login")}
          </label>
          <p class="hint">{t("settings.coming_soon")}</p>
        </section>

        <section class="field">
          <span class="label">{t("settings.history_limit")}</span>
          <input type="number" class="num" value={50} disabled />
          <p class="hint">{t("settings.coming_soon")}</p>
        </section>

        <section class="field about">
          <span class="label">{t("settings.about")}</span>
          <p class="about-name">
            <strong>Launchkeeper</strong>
            <span class="muted">{t("settings.about.version", { version })}</span>
          </p>
          <p class="hint">{t("settings.about.tagline")}</p>
          <div class="links">
            <button class="link" onclick={() => openLink(LINKS.repo)}>{t("settings.about.repo")}</button>
            <span class="sep">·</span>
            <button class="link" onclick={() => openLink(LINKS.issues)}>{t("settings.about.issues")}</button>
            {#if coffeeReady()}
              <span class="sep">·</span>
              <button class="link coffee" onclick={() => openLink(LINKS.coffee)}>{t("settings.about.coffee")}</button>
            {/if}
          </div>
        </section>
      </div>
    {:else}
      <p class="muted loading">{t("settings.load_failed")}</p>
    {/if}
  </div>
</div>

<style>
  .about-name {
    margin: 0 0 4px;
    display: flex;
    gap: 10px;
    align-items: baseline;
  }
  .links {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: center;
    margin-top: 6px;
  }
  .links .link {
    background: none;
    border: none;
    padding: 0;
    color: var(--accent);
    cursor: pointer;
    font: inherit;
  }
  .links .link:hover {
    text-decoration: underline;
  }
  .links .sep {
    color: var(--muted);
  }
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.25);
    z-index: 1500;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .sheet {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 10px;
    width: 420px;
    max-width: calc(100vw - 32px);
    max-height: calc(100vh - 64px);
    overflow-y: auto;
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.3);
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 14px 16px;
    border-bottom: 1px solid var(--border);
    position: sticky;
    top: 0;
    background: var(--bg);
  }
  h2 {
    margin: 0;
    font-size: 14px;
  }
  .close {
    all: unset;
    cursor: default;
    color: var(--muted);
    padding: 2px 4px;
  }
  .close:hover {
    color: var(--fg);
  }
  .loading {
    padding: 24px 16px;
  }
  .body {
    display: flex;
    flex-direction: column;
    gap: 18px;
    padding: 16px;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .label {
    font-size: 12px;
    font-weight: 600;
    color: var(--muted);
  }
  /* 分段控件：三格等宽，选中的那格填 accent。 */
  .segmented {
    display: inline-flex;
    width: fit-content;
    border: 1px solid var(--border);
    border-radius: 6px;
    overflow: hidden;
    background: var(--surface);
  }
  .segment {
    all: unset;
    box-sizing: border-box;
    padding: 5px 14px;
    font-size: 13px;
    color: var(--fg);
    cursor: default;
    text-align: center;
    white-space: nowrap;
  }
  .segment + .segment {
    border-left: 1px solid var(--border);
  }
  .segment:hover:not(.active):not(:disabled) {
    background: var(--surface-hover);
  }
  .segment.active {
    background: var(--accent);
    color: #fff;
  }
  .segment:disabled {
    opacity: 0.6;
  }
  /* ---- AI 那一格（M4 §4）---- */
  /* 一格里有四个输入框，每个都要自己的小标题；`.label` 是整格的标题。 */
  .sub {
    font-size: 12px;
    color: var(--muted);
    margin-top: 4px;
  }
  .control {
    padding: 5px 8px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--fg);
    font-size: 13px;
    width: 100%;
    font-family: inherit;
  }
  .control:disabled {
    opacity: 0.6;
  }
  .prompt {
    /* 提示词里有反引号和短横线对齐的列表，等宽字体读起来才是它本来的样子。 */
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 12px;
    line-height: 1.5;
    resize: vertical;
    white-space: pre-wrap;
    color: var(--muted);
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .btn {
    padding: 5px 12px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--fg);
    font-size: 13px;
    cursor: default;
    flex-shrink: 0;
  }
  .btn:hover:not(:disabled) {
    background: var(--surface-hover);
  }
  .btn:disabled {
    opacity: 0.5;
  }
  .link {
    all: unset;
    color: var(--accent);
    cursor: default;
    font-size: 12px;
  }
  .link:disabled {
    opacity: 0.5;
  }
  .state {
    font-size: 12px;
    color: var(--muted);
  }
  .state.on {
    color: var(--status-ok);
  }
  .test {
    font-size: 12px;
    color: var(--status-ok);
    overflow-wrap: anywhere;
  }
  .test.bad {
    color: var(--status-failed);
  }
  .num {
    padding: 5px 8px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--fg);
    font-size: 13px;
    width: fit-content;
    min-width: 160px;
  }
  .num:disabled {
    opacity: 0.6;
  }
  .readonly-path {
    padding: 6px 8px;
    border-radius: 6px;
    background: var(--surface);
    border: 1px solid var(--border);
    color: var(--muted);
    font-size: 12px;
    font-family:
      ui-monospace, SFMono-Regular, Menlo, monospace;
    overflow-x: auto;
    white-space: nowrap;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
  }
  .check.disabled {
    color: var(--muted);
  }
  .hint {
    margin: 0;
    font-size: 12px;
    color: var(--muted);
  }
</style>
