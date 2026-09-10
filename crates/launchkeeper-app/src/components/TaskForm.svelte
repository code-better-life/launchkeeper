<script lang="ts">
  import { onDestroy, untrack } from "svelte";
  import type { InterpreterView, TaskInput } from "../lib/bindings";
  import { api } from "../lib/api";
  import { taskStore } from "../lib/stores/tasks.svelte";
  import { locale, t, type Params, type TranslationKey } from "../lib/i18n";
  import {
    composeTaskCommand,
    detectFromTask,
    interpreterId,
    viewOf,
  } from "../lib/interpreter";
  import {
    defaultTriggerForm,
    formToTrigger,
    triggerFormError,
    triggerToForm,
  } from "../lib/trigger";
  import type { IntervalUnit, TriggerForm } from "../lib/trigger";
  import InterpreterPicker from "./InterpreterPicker.svelte";
  import TriggerEditor from "./TriggerEditor.svelte";

  let {
    mode,
    initial,
    onsaved,
    oncancel,
  }: {
    mode: "create" | "edit";
    initial: TaskInput | null;
    onsaved: (saved: TaskInput) => void;
    oncancel: () => void;
  } = $props();

  // 表单里的「脚本」和「运行方式」是分开的两格；数据库里存的是拼好的
  // script_path + args。进来时拆开（detectFromTask），保存时拼回去
  // （composeTaskCommand），两个函数都是 core::interpreters 的镜像。
  // 拆不开的命令（比如 script_path 本身就是个解释器、后面却没有脚本）原样
  // 落进「脚本」格，运行方式留空 = 直接执行，保存后还是原来那条命令。
  // svelte-ignore state_referenced_locally
  const initialSplit = initial ? detectFromTask(initial.script_path, initial.args) : null;

  function makeState(input: TaskInput | null) {
    return {
      name: input?.name ?? "",
      display_name: input?.display_name ?? "",
      description: input?.description ?? "",
      script_path: initialSplit?.script ?? input?.script_path ?? "",
      args: initialSplit ? [...initialSplit.args] : input ? [...input.args] : [],
      working_dir: input?.working_dir ?? "",
      env: input ? input.env.map((e) => ({ key: e.key, value: e.value })) : [],
      keep_alive: input?.keep_alive ?? false,
      timeoutEnabled: input?.timeout_secs != null,
      timeoutValue: input?.timeout_secs ?? 60,
      timeoutUnit: "seconds" as IntervalUnit,
      tags: input ? [...input.tags] : [],
      favorite: input?.favorite ?? false,
      // 新建任务默认开启失败通知：静默失败的定时任务是这个工具要解决的问题
      // 本身，默认关掉它等于默认把工具的价值关掉。
      notify_on_fail: input?.notify_on_fail ?? true,
    };
  }

  // 表单只在挂载时用 `initial` 播种一次；之后 initial 变化（例如后端刷新）
  // 不应打断用户正在编辑的内容，丢弃改动走的是 TaskDetail 用 {#key} 强制
  // 重新挂载这条路径，而不是让这里响应 initial 的变化。
  // svelte-ignore state_referenced_locally
  let f = $state(makeState(initial));
  // svelte-ignore state_referenced_locally
  let triggerForm = $state<TriggerForm>(
    // svelte-ignore state_referenced_locally
    initial ? triggerToForm(initial.trigger) : defaultTriggerForm(),
  );
  let newTag = $state("");
  let saving = $state(false);
  /** 后端原样返回的错误，不经过词典。 */
  let backendError = $state<string | null>(null);
  // 报错存的是词典键（加插值参数）而不是拼好的句子：切换语言时那条还挂在
  // 表单上的错误也要跟着变（M3 §5）。
  let formError = $state<{ key: TranslationKey; params?: Params } | null>(null);

  // ---- 运行方式 ---------------------------------------------------------
  // 候选来自后端 scan_interpreters；当前选中的那个可能不在候选里（编辑一个已有
  // 任务、或者用户手填了路径），所以单独存一份，并在渲染时并进候选列表。
  let interpreters = $state<InterpreterView[]>([]);
  let scanning = $state(false);
  // svelte-ignore state_referenced_locally
  let interpreter = $state<InterpreterView | null>(
    // svelte-ignore state_referenced_locally
    initialSplit ? viewOf(initialSplit.interpreter, locale.current) : null,
  );
  // 选择被「钉住」之后，重新扫描不再把它改回推荐项。两种情况会钉住：用户自己动过
  // 下拉框（改一下工作目录不该把人家选好的解释器冲掉），以及打开一个已有任务
  // （那条命令是它现在真正在跑的东西，开个表单不该悄悄换掉，哪怕那个解释器已经
  // 不在 PATH 里了）。
  //
  // 注意判据是 `initial`，不是 `initialSplit`：拆不开的命令（detectFromTask 返回
  // null，比如 `python3` 后面根本没有脚本）同样是一个已有任务，钉住它才不会在
  // 第一次扫描回来时把 script_path 换成推荐的解释器、把原来那条命令改掉。
  // svelte-ignore state_referenced_locally
  let pinned = $state(initial != null);
  // 解释器自己的参数（`python3 -u x.py` 里的 `-u`）。界面上没有编辑它的地方，
  // 但保存时要原样带回去，否则打开再保存一次就把 -u 弄丢了。只在运行方式还是
  // 打开任务时那个的情况下带：换了解释器，前一个的 flag 就没有意义了。
  const initialInterpId = initialSplit
    ? interpreterId(initialSplit.interpreter)
    : null;
  const interpArgs = $derived(
    initialSplit && interpreter?.id === initialInterpId
      ? initialSplit.interp_args
      : [],
  );

  const options = $derived(
    interpreter && !interpreters.some((o) => o.id === interpreter!.id)
      ? [interpreter, ...interpreters]
      : interpreters,
  );

  /**
   * 下拉框下面那行说明。只有当前选中的就是推荐项时才说“为什么推荐它”，
   * 否则说明的是别的东西，挂在这里会误导。
   */
  const reason = $derived(
    interpreter == null
      ? null
      : interpreter.recommended
        ? // 后端拼好的推荐理由，本期仍然只有中文（docs/M3-design.md §5）。
          interpreter.reason
        : interpreters.some((o) => o.recommended)
          ? t("form.run_with_manual")
          : null,
  );

  // 每次扫描领一个号；只有最新那次的结果算数。扫描要跑一遍 PATH 并对每个程序执行
  // 一次 --version，快慢差得很远，先发的请求完全可能后回来——不挡一下的话，改完
  // 脚本路径过一会儿会被上一个脚本的候选和推荐项盖掉。
  let seq = 0;

  async function rescan(script: string, projectDir: string) {
    const mine = ++seq;
    scanning = true;
    try {
      const found = await api.scanInterpreters(
        script.trim() || null,
        projectDir.trim() || null,
      );
      if (mine !== seq) return;
      interpreters = found;
      const same = found.find((o) => o.id === interpreter?.id);
      if (same) {
        // 同一个解释器，换成带版本号和出处的那份。
        interpreter = same;
      } else if (!pinned) {
        interpreter = found.find((o) => o.recommended) ?? interpreter;
      }
    } catch {
      if (mine !== seq) return;
      // 扫描失败不该拦住表单：下拉框保持上一次的候选，用户仍可手填路径。
      interpreters = [];
    } finally {
      if (mine === seq) scanning = false;
    }
  }

  // 脚本路径 / 工作目录变了就重扫，防抖 300 ms——真机上一次扫描要走一遍 PATH
  // 并对每个程序执行一次 --version，不能每敲一个字符来一次。
  $effect(() => {
    const script = f.script_path;
    const dir = f.working_dir;
    const timer = setTimeout(() => void rescan(script, dir), 300);
    return () => clearTimeout(timer);
  });

  // 改动检测：和挂载时的快照比。上报给 store，切换任务 / 新建时据此决定
  // 要不要先问一句。保存后 TaskDetail 会用 {#key} 重新挂载表单，基线随之刷新。
  function snapshot() {
    return JSON.stringify({
      f: $state.snapshot(f),
      t: $state.snapshot(triggerForm),
      // 只记 id：扫描回来会把同一个解释器换成带版本号的那份，那不算改动。
      i: interpreter?.id ?? null,
    });
  }
  const baseline = untrack(snapshot);
  $effect(() => {
    taskStore.formDirty = snapshot() !== baseline;
  });
  onDestroy(() => {
    taskStore.formDirty = false;
  });

  // keep_alive 只在 at_login 和 manual 下有效（Task::validate 拒绝其余组合）。
  // 这里不用 $effect 去改写 f.keep_alive：那会在用户临时切一下触发方式时把他
  // 原来的勾选永久抹掉。勾选框在其它触发方式下是 disabled 的，真正提交给后端
  // 的值在 buildInput 里再判一次。
  const keepAliveAllowed = $derived(
    triggerForm.kind === "at_login" || triggerForm.kind === "manual",
  );
  const keepAliveEffective = $derived(keepAliveAllowed && f.keep_alive);

  function addArg() {
    f.args = [...f.args, ""];
  }
  function removeArg(i: number) {
    f.args = f.args.filter((_, idx) => idx !== i);
  }
  function setArg(i: number, v: string) {
    const copy = f.args.slice();
    copy[i] = v;
    f.args = copy;
  }

  function addEnv() {
    f.env = [...f.env, { key: "", value: "" }];
  }
  function removeEnv(i: number) {
    f.env = f.env.filter((_, idx) => idx !== i);
  }

  function addTag() {
    const t = newTag.trim();
    if (t && !f.tags.includes(t)) f.tags = [...f.tags, t];
    newTag = "";
  }
  function removeTag(t: string) {
    f.tags = f.tags.filter((x) => x !== t);
  }

  /** 超时秒数；`value` 已由 save() 校验过是个大于 0 的有限数。 */
  function timeoutSeconds(value: number): number {
    const unit = f.timeoutUnit === "hours" ? 3600 : f.timeoutUnit === "minutes" ? 60 : 1;
    return Math.round(value * unit);
  }

  function buildInput(): TaskInput {
    // 「脚本 + 参数 + 运行方式」在这里拼成后端要的 script_path + args；规则与
    // core::interpreters::apply 一致（lib/interpreter.ts 有说明）。没选运行方式
    // 就是直接执行，等于 M2 时的行为。
    const command = composeTaskCommand(
      f.script_path.trim(),
      // 空参数行是"点了添加还没填"的产物，不是一个想传给脚本的空字符串参数：
      // 界面上没有别的办法表达后者，所以这里一律丢掉。
      f.args.filter((a) => a.length > 0),
      interpreter,
      interpArgs,
    );
    return {
      name: f.name.trim(),
      display_name: f.display_name.trim() || f.name.trim(),
      description: f.description.trim() ? f.description.trim() : null,
      script_path: command.script_path,
      args: command.args,
      working_dir: f.working_dir.trim() ? f.working_dir.trim() : null,
      env: f.env
        .filter((e) => e.key.trim().length > 0)
        .map((e) => ({ key: e.key.trim(), value: e.value })),
      trigger: formToTrigger(triggerForm),
      keep_alive: keepAliveEffective,
      timeout_secs: f.timeoutEnabled ? timeoutSeconds(f.timeoutValue) : null,
      tags: f.tags,
      favorite: f.favorite,
      notify_on_fail: f.notify_on_fail,
    };
  }

  async function save() {
    formError = null;
    backendError = null;
    if (!f.name.trim()) {
      formError = { key: "form.error.name_required" };
      return;
    }
    const script = f.script_path.trim();
    if (!script) {
      formError = { key: "form.error.script_required" };
      return;
    }
    // launchd 在任务的工作目录（没设就是 $HOME）里起进程，相对路径几乎一定不是
    // 用户想的那个；后端也会拒，但那要等到点了保存之后才说。
    if (!script.startsWith("/")) {
      formError = { key: "form.error.script_absolute" };
      return;
    }
    // 数字输入框清空后 bind:value 给的是 null；夹到 1 会静默造出一个用户没写
    // 过的超时/间隔，所以这里当作校验错误报出来（M4）。
    const timeout = f.timeoutValue as number | null | undefined;
    if (
      f.timeoutEnabled &&
      (timeout == null || !Number.isFinite(timeout) || timeout <= 0)
    ) {
      formError = { key: "form.error.timeout_number" };
      return;
    }
    const triggerErr = triggerFormError(triggerForm);
    if (triggerErr) {
      formError = { key: triggerErr };
      return;
    }
    saving = true;
    try {
      // 最后一道，问一次后端，别拿它猜：路径写对了、但那儿什么都没有，是最
      // 常见的一种"任务建好了却天天失败"；而没选运行方式就是交给 launchd 直接
      // exec 这个文件，它得可执行才行。
      const check = await api.checkScript(script);
      if (!check.exists) {
        formError = { key: "form.error.script_missing", params: { path: script } };
        return;
      }
      if (interpreter == null && !check.executable) {
        formError = { key: "form.error.not_executable" };
        return;
      }
      const input = buildInput();
      if (mode === "create") {
        await api.createTask(input);
      } else {
        await api.updateTask(initial!.name, input);
      }
      onsaved(input);
    } catch (e) {
      // 后端原话（AppError.message）本期仍然只有中文（docs/M3-design.md §5）。
      backendError = e instanceof Error ? e.message : String(e);
    } finally {
      saving = false;
    }
  }
</script>

<form class="task-form" onsubmit={(e) => e.preventDefault()}>
  <div class="grid">
    <label class="field">
      <span>{t("form.name")}</span>
      <input
        type="text"
        bind:value={f.name}
        disabled={mode === "edit"}
        placeholder={t("form.name_placeholder")}
      />
    </label>
    <label class="field">
      <span>{t("form.display_name")}</span>
      <input
        type="text"
        bind:value={f.display_name}
        placeholder={t("form.display_name_placeholder")}
      />
    </label>
  </div>

  <label class="field">
    <span>{t("form.description")}</span>
    <textarea rows="2" bind:value={f.description}></textarea>
  </label>

  <label class="field">
    <span>{t("form.script")}</span>
    <input
      type="text"
      bind:value={f.script_path}
      placeholder={t("form.script_placeholder")}
    />
  </label>

  <div class="field">
    <span id="run-with-label">{t("form.run_with")}</span>
    <div class="run-with">
      <InterpreterPicker
        bind:value={interpreter}
        options={options}
        loading={scanning}
        labelledby="run-with-label"
        onpicked={() => (pinned = true)}
      />
      {#if reason}
        <span class="hint">{reason}</span>
      {/if}
    </div>
    {#if interpArgs.length > 0}
      <!-- 这条命令里解释器自己带着参数。表单没有编辑它的地方（要改就改成
           另一个运行方式），但保存时会原样带回去，所以这里如实说一声。 -->
      <span class="hint">{t("form.interp_args", { args: interpArgs.join(" ") })}</span>
    {/if}
  </div>

  <div class="field">
    <span>{t("form.args")}</span>
    <div class="list-editor">
      {#each f.args as arg, i (i)}
        <div class="list-row">
          <input type="text" value={arg} oninput={(e) => setArg(i, (e.target as HTMLInputElement).value)} />
          <button type="button" class="remove" onclick={() => removeArg(i)}>
            {t("common.remove")}
          </button>
        </div>
      {/each}
      <button type="button" class="add" onclick={addArg}>{t("form.args_add")}</button>
    </div>
  </div>

  <label class="field">
    <span>{t("form.working_dir")}</span>
    <input
      type="text"
      bind:value={f.working_dir}
      placeholder={t("form.working_dir_placeholder")}
    />
  </label>

  <div class="field">
    <span>{t("form.env")}</span>
    <div class="list-editor">
      {#each f.env as pair, i (i)}
        <div class="list-row">
          <input
            type="text"
            bind:value={pair.key}
            placeholder={t("form.env_key_placeholder")}
            class="env-key"
          />
          <input
            type="text"
            bind:value={pair.value}
            placeholder={t("form.env_value_placeholder")}
          />
          <button type="button" class="remove" onclick={() => removeEnv(i)}>
            {t("common.remove")}
          </button>
        </div>
      {/each}
      <button type="button" class="add" onclick={addEnv}>{t("form.env_add")}</button>
    </div>
  </div>

  <div class="field">
    <span>{t("form.trigger")}</span>
    <TriggerEditor bind:form={triggerForm} />
  </div>

  <div class="field">
    <label class="check" class:disabled={!keepAliveAllowed}>
      <input type="checkbox" bind:checked={f.keep_alive} disabled={!keepAliveAllowed} />
      {t("form.keep_alive")}
    </label>
    {#if triggerForm.kind === "manual"}
      <p class="hint">{t("form.keep_alive_hint")}</p>
    {/if}
  </div>

  <div class="field">
    <span>{t("form.timeout")}</span>
    <div class="timeout-row">
      <label>
        <input type="checkbox" bind:checked={f.timeoutEnabled} />
        {t("form.timeout_check")}
      </label>
      {#if f.timeoutEnabled}
        <input type="number" min="1" class="num" bind:value={f.timeoutValue} />
        <select bind:value={f.timeoutUnit}>
          <option value="seconds">{t("unit.seconds")}</option>
          <option value="minutes">{t("unit.minutes")}</option>
          <option value="hours">{t("unit.hours")}</option>
        </select>
      {:else}
        <span class="muted">{t("form.timeout_none")}</span>
      {/if}
    </div>
  </div>

  <div class="field">
    <span>{t("form.tags")}</span>
    <div class="tags-row">
      {#each f.tags as tag (tag)}
        <span class="tag">
          {tag}
          <button type="button" onclick={() => removeTag(tag)}>✕</button>
        </span>
      {/each}
      <input
        type="text"
        class="tag-input"
        placeholder={t("form.tag_placeholder")}
        bind:value={newTag}
        onkeydown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            addTag();
          }
        }}
      />
    </div>
  </div>

  <div class="field row-check">
    <label>
      <input type="checkbox" bind:checked={f.favorite} />
      {t("form.favorite")}
    </label>
    <label>
      <input type="checkbox" bind:checked={f.notify_on_fail} />
      {t("form.notify_on_fail")}
    </label>
  </div>

  {#if formError}
    <p class="form-error">{t(formError.key, formError.params)}</p>
  {:else if backendError}
    <p class="form-error">{backendError}</p>
  {/if}

  <div class="actions">
    <button type="button" class="btn" onclick={oncancel} disabled={saving}>
      {t("common.cancel")}
    </button>
    <button type="button" class="btn primary" onclick={save} disabled={saving}>
      {saving ? t("common.saving") : t("common.save")}
    </button>
  </div>
</form>

<style>
  .task-form {
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 4px 2px 20px;
  }
  .grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .field > span {
    font-size: 12px;
    color: var(--muted);
  }
  input[type="text"],
  input[type="number"],
  textarea,
  select {
    padding: 5px 8px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
    color: var(--fg);
    font-size: 13px;
    font-family: inherit;
  }
  input:disabled {
    opacity: 0.55;
  }
  .num {
    width: 80px;
  }
  .list-editor {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .list-row {
    display: flex;
    gap: 6px;
  }
  .list-row input {
    flex: 1;
  }
  .env-key {
    max-width: 160px;
  }
  .remove,
  .add {
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--fg);
    border-radius: 5px;
    padding: 3px 10px;
    font-size: 12px;
    cursor: default;
    flex-shrink: 0;
  }
  .add {
    align-self: flex-start;
  }
  .remove:hover,
  .add:hover {
    background: var(--surface-hover);
  }
  .row-check {
    flex-direction: row;
    gap: 20px;
    align-items: center;
  }
  .row-check label {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 13px;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 13px;
  }
  .check.disabled {
    color: var(--muted);
  }
  .hint {
    margin: 0;
    font-size: 12px;
    line-height: 1.5;
    color: var(--muted);
  }
  .run-with {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    flex-wrap: wrap;
  }
  .run-with .hint {
    padding-top: 7px;
  }
  .timeout-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .timeout-row label {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .muted {
    color: var(--muted);
  }
  .tags-row {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
  }
  .tag {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 2px 4px 2px 10px;
    font-size: 12px;
  }
  .tag button {
    all: unset;
    cursor: default;
    padding: 0 6px;
    color: var(--muted);
  }
  .tag-input {
    min-width: 140px;
  }
  .form-error {
    color: #d7443e;
    background: rgba(215, 68, 62, 0.1);
    border: 1px solid rgba(215, 68, 62, 0.4);
    border-radius: 6px;
    padding: 8px 10px;
    font-size: 12.5px;
    white-space: pre-wrap;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    position: sticky;
    bottom: 0;
    background: var(--bg);
    padding-top: 8px;
  }
  .btn {
    padding: 6px 16px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--fg);
    font-size: 13px;
    cursor: default;
  }
  .btn.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }
  .btn:disabled {
    opacity: 0.6;
  }
</style>
