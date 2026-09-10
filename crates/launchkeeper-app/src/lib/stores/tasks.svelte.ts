// 全局任务状态：Svelte 5 runes。
//
// runes 不能作为普通导出的模块级绑定跨文件保持响应性，所以用一个类把
// $state 包起来，通过单例实例的 getter/方法对外暴露——这是本仓库里
// runes 状态目前唯一要接入的地方，采用 class 写法方便以后加派生字段。
import type { ExternalAgentView, TaskView } from "../bindings";
import { api, events } from "../api";
import { filterExternal } from "../external";
import { tagCounts } from "../counts";
import {
  DEFAULT_SORT,
  readListSort,
  sortMode,
  writeListSort,
  type SortMode,
} from "../sort";

class TaskStore {
  tasks = $state<TaskView[]>([]);
  loading = $state(true);
  selectedName = $state<string | null>(null);
  /**
   * 别人写的 LaunchAgents（M3 §3.4）。不在后台扫描的推送里：列一次要读一遍
   * ~/Library/LaunchAgents 再对每个 label 问一次 launchctl，那是文件系统的活，
   * 不该每 2 秒干一遍。只在初次加载、接管/撤销之后、点刷新时拉。
   */
  externalAgents = $state<ExternalAgentView[]>([]);
  externalLoading = $state(false);
  /** 选中的外部 agent 的 label；和 selectedName 互斥，右侧据此二选一。 */
  selectedExternal = $state<string | null>(null);
  /**
   * 正在确认接管的 label（null 表示没开框）。放在 store 而不是某个组件里：
   * 列表行的「接管」按钮和详情页头部的「接管」按钮开的是同一个框。
   */
  adoptTarget = $state<string | null>(null);
  search = $state("");
  selectedTags = $state<string[]>([]);
  /**
   * 管理组的排序方式（M4 §2）。
   *
   * 第一帧先用 localStorage 里那份，`App.svelte` 拿到 `get_settings` 之后再用
   * `AppSettings.list_sort` 覆盖（`applyListSort`）——列表在设置回来之前就已经
   * 画出来了，让它先按上次的顺序排，比先按名称排一下再跳一次好。
   */
  listSort = $state<SortMode>(readListSort() ?? DEFAULT_SORT);
  toastError = $state<string | null>(null);
  /**
   * 这条 toast 是报错还是报成功（M4 §4）。
   *
   * 同一个位置、同一个计时器，只是底色不同：一条「已复制到剪贴板」不该长得
   * 像出错，也不值得为它再造一套浮层。默认是 `error`，`showError` 每次都会
   * 把它设回去，所以先弹一条成功的再弹一条失败的不会串色。
   */
  toastKind = $state<"error" | "ok">("error");
  /**
   * 每次任务列表被后端数据覆盖就 +1（后台扫描事件、refresh、立即运行之后）。
   * 详情页那些不在 TaskView 里的东西——运行历史、日志——用它当"该重新拉一次
   * 了"的信号，这样运行历史不会停在几分钟前的快照上。
   */
  revision = $state(0);
  /** 这些任务正有一次启停请求在飞，界面据此禁用启动/停止/重启按钮。 */
  busyServices = $state<string[]>([]);
  /** 右侧正在显示"新建任务"表单。新建期间左侧不高亮任何任务。 */
  creating = $state(false);
  /**
   * 详情页的「配置」页正在编辑（M3 §3.4 追补）。
   *
   * false 时那一页是只读概览——点开一个任务不该直接掉进一堆输入框里。放在 store
   * 而不是 TaskDetail 里，是因为进入编辑的入口不止一个（右键菜单的「编辑」要
   * 连选中一起做），而离开编辑的路径（换任务、开始新建、接管、删除）全都在
   * 这里，忘一个就会出现"换了个任务却还在编辑态"。
   */
  editing = $state(false);
  /** 右侧表单（新建或编辑）有尚未保存的改动，由 TaskForm 上报。 */
  formDirty = $state(false);
  /**
   * 会丢掉未保存改动的动作（切换任务、开始新建、放弃新建）在 formDirty 时
   * 先存到这里，由 App 弹确认框；用户确认后再执行。null 表示没有待确认动作。
   */
  pendingAction = $state<(() => void) | null>(null);
  private selectedBeforeCreate: string | null = null;
  private toastTimer: ReturnType<typeof setTimeout> | null = null;

  allTags = $derived.by(() => {
    const set = new Set<string>();
    for (const t of this.tasks) for (const tag of t.tags) set.add(tag);
    return Array.from(set).sort();
  });

  /**
   * 只按搜索框过滤，不含标签筛选。
   *
   * 标签 chip 上的计数（M4 §3）用的是这一份：数字跟着搜索变，但不跟着标签
   * 筛选变，理由见 `lib/counts.ts`。
   */
  searchFiltered = $derived.by(() => {
    const q = this.search.trim().toLowerCase();
    if (!q) return this.tasks;
    return this.tasks.filter((t) =>
      `${t.display_name} ${t.name}`.toLowerCase().includes(q),
    );
  });

  filtered = $derived.by(() => {
    if (this.selectedTags.length === 0) return this.searchFiltered;
    return this.searchFiltered.filter((t) =>
      this.selectedTags.every((tag) => t.tags.includes(tag)),
    );
  });

  /** 每个标签有多少个任务（M4 §3）。 */
  tagCounts = $derived.by(() => tagCounts(this.searchFiltered));

  /** 搜索或标签筛选正在起作用——分组标题据此显示「匹配 x / 共 y」。 */
  filtering = $derived(
    this.search.trim().length > 0 || this.selectedTags.length > 0,
  );

  selected = $derived.by(() =>
    this.tasks.find((t) => t.name === this.selectedName) ?? null,
  );

  /** 搜索/标签对外部 agent 的过滤，规则见 lib/external.ts。 */
  filteredExternal = $derived.by(() =>
    filterExternal(this.externalAgents, this.search, this.selectedTags),
  );

  selectedAgent = $derived.by(
    () =>
      this.externalAgents.find((a) => a.label === this.selectedExternal) ?? null,
  );

  private unlisten: (() => void) | null = null;

  async init() {
    await Promise.all([this.refresh(), this.loadExternal()]);
    this.unlisten = await events.tasksChanged.listen((e) => {
      this.applyList(e.payload);
    });
  }

  dispose() {
    this.unlisten?.();
    this.unlisten = null;
  }

  private applyList(list: TaskView[]) {
    this.tasks = list;
    this.revision += 1;
    // 保持选中项稳定：按 name 匹配；若已选中的任务消失了则清空选中。
    if (this.selectedName && !list.some((t) => t.name === this.selectedName)) {
      this.selectedName = null;
    }
  }

  async refresh() {
    this.loading = true;
    try {
      const list = await api.listTasks();
      this.applyList(list);
    } catch (e) {
      this.showError(e);
    } finally {
      this.loading = false;
    }
  }

  /**
   * 拉一次「其他 LaunchAgents」。
   *
   * 失败只记 toast，不清空已有列表：这一组是只读的旁支，读不出来时把上一次
   * 的结果留在屏幕上，比让半个列表突然消失更不吓人。
   */
  async loadExternal() {
    this.externalLoading = true;
    try {
      const list = await api.listExternalAgents();
      this.externalAgents = list;
      if (
        this.selectedExternal &&
        !list.some((a) => a.label === this.selectedExternal)
      ) {
        this.selectedExternal = null;
      }
    } catch (e) {
      this.showError(e);
    } finally {
      this.externalLoading = false;
    }
  }

  // ---- M4 §2：管理组的排序 ----

  /**
   * 用 `settings.json` 里那份覆盖当前排序（`App.svelte` 启动时读一次）。
   *
   * 字段缺失（M4 之前写的 settings.json）或者是个不认识的值时，保留 localStorage
   * 里那份——它至少是这台机器上的人选过的，比无条件退回「按名称」强。
   */
  applyListSort(raw: string | null | undefined) {
    if (raw == null) return;
    this.listSort = sortMode(raw);
  }

  /**
   * 换一种排序：界面立刻变，然后写回设置。
   *
   * 写的是整份 `AppSettings`（后端的 `set_settings` 是整体替换），所以先读一次
   * 再改这一个字段，不然会把别处刚存的语言/通知开关抹掉。
   *
   * localStorage 里那份**每次都写**，不只在失败时写：它是下次启动时第一帧的
   * 依据，也是设置写不进去（数据目录只读、磁盘满）时唯一还记得住这个选择的
   * 地方。写失败不回滚界面——排序纯粹是看的方式，让用户刚点的那一下自己弹
   * 回去，比一个没存住的排序更让人摸不着头脑；toast 已经把原因说了。
   */
  async setListSort(mode: SortMode) {
    if (this.listSort === mode) return;
    this.listSort = mode;
    writeListSort(mode);
    try {
      const current = await api.getSettings();
      await api.setSettings({ ...current, list_sort: mode });
    } catch (e) {
      this.showError(e);
    }
  }

  /** 顶栏「刷新」：两组一起重来。 */
  async refreshAll() {
    await Promise.all([this.refresh(), this.loadExternal()]);
  }

  /** 会丢改动的动作统一走这里：没改动直接做，有改动先问。 */
  private guarded(action: () => void) {
    if (this.formDirty) {
      this.pendingAction = action;
    } else {
      action();
    }
  }

  confirmPending() {
    const action = this.pendingAction;
    this.pendingAction = null;
    this.formDirty = false;
    action?.();
  }

  cancelPending() {
    this.pendingAction = null;
  }

  select(name: string | null) {
    if (!this.creating && !this.selectedExternal && name === this.selectedName)
      return;
    this.guarded(() => {
      this.creating = false;
      this.formDirty = false;
      this.editing = false;
      this.selectedExternal = null;
      this.selectedName = name;
    });
  }

  /**
   * 选中一个任务并直接进入编辑（列表右键菜单的「编辑」）。
   *
   * 不能走 `select()`：那一条在"点的就是已选中的任务"时直接返回，而这里正是
   * 用户在已经选中的任务上要求编辑的常见情形。
   */
  selectForEdit(name: string) {
    this.guarded(() => {
      this.creating = false;
      this.formDirty = false;
      this.selectedExternal = null;
      this.selectedName = name;
      this.editing = true;
    });
  }

  /** 进入/离开编辑（详情页的「编辑」按钮、保存、取消）。 */
  setEditing(on: boolean) {
    this.editing = on;
    if (!on) this.formDirty = false;
  }

  /** 选中一个外部 agent（右侧换成只读的 ExternalDetail）。 */
  selectExternal(label: string | null) {
    if (!this.creating && label === this.selectedExternal) return;
    this.guarded(() => {
      this.creating = false;
      this.formDirty = false;
      this.editing = false;
      this.selectedName = null;
      this.selectedExternal = label;
    });
  }

  startCreate() {
    if (this.creating) return;
    this.guarded(() => {
      this.selectedBeforeCreate = this.selectedName;
      this.selectedName = null;
      this.selectedExternal = null;
      this.formDirty = false;
      this.editing = false;
      this.creating = true;
    });
  }

  /** 放弃新建（取消或 Esc）：回到新建前选中的任务。 */
  stopCreate() {
    if (!this.creating) return;
    this.guarded(() => {
      this.creating = false;
      this.formDirty = false;
      this.editing = false;
      this.selectedName = this.selectedBeforeCreate;
    });
  }

  /** 新建已保存：直接切到新任务，不需要确认。 */
  finishCreate(name: string) {
    this.creating = false;
    this.formDirty = false;
    this.editing = false;
    this.selectedName = name;
  }

  showError(e: unknown) {
    const msg = e instanceof Error ? e.message : String(e);
    this.toastKind = "error";
    this.toastError = msg;
    if (this.toastTimer) clearTimeout(this.toastTimer);
    this.toastTimer = setTimeout(() => {
      this.toastError = null;
    }, 6000);
  }

  /** 同一条 toast，绿色的那种——「已复制到剪贴板」这类干完就走的确认。 */
  showOk(message: string) {
    this.toastKind = "ok";
    this.toastError = message;
    if (this.toastTimer) clearTimeout(this.toastTimer);
    // 成功的消息不用读第二遍，比报错收得快一些。
    this.toastTimer = setTimeout(() => {
      this.toastError = null;
    }, 3000);
  }

  dismissToast() {
    this.toastError = null;
    if (this.toastTimer) clearTimeout(this.toastTimer);
  }

  /**
   * 启用开关：乐观更新，失败回滚。
   *
   * 回滚按 name 定位，不按下标：命令还没返回时后台扫描（§3.4）随时可能整
   * 个换掉 this.tasks，排序也可能变，旧下标写回去就会把回滚写到别的任务上。
   */
  async setEnabled(name: string, enabled: boolean) {
    const prev = this.tasks.find((t) => t.name === name) ?? null;
    this.patch(name, { enabled });
    try {
      await api.setEnabled(name, enabled);
    } catch (e) {
      if (prev) this.patch(name, { enabled: prev.enabled });
      this.showError(e);
      return;
    }
    await this.refresh();
  }

  /** 就地改写某个任务的若干字段；任务已不在列表里则什么也不做。 */
  private patch(name: string, fields: Partial<TaskView>) {
    const idx = this.tasks.findIndex((t) => t.name === name);
    if (idx < 0) return;
    const copy = this.tasks.slice();
    copy[idx] = { ...copy[idx], ...fields };
    this.tasks = copy;
  }

  async runNow(name: string) {
    try {
      await api.runNow(name);
    } catch (e) {
      this.showError(e);
      return;
    }
    await this.refresh();
  }

  /**
   * 服务型任务的启动 / 停止 / 重启（M2.5 §7）。
   *
   * 名字记在 `busyServices` 里，界面据此把这三个按钮禁掉：停止最多要等后端
   * 8 秒（STOP_GRACE），期间不能让用户连点出好几次 launchctl 调用。无论成功
   * 失败都刷新一次，让 pid / 运行时长 立刻跟上，而不是等下一轮后台扫描。
   */
  async start(name: string) {
    await this.lifecycle(name, () => api.startService(name));
  }

  async stop(name: string) {
    await this.lifecycle(name, () => api.stopService(name));
  }

  async restart(name: string) {
    await this.lifecycle(name, () => api.restartService(name));
  }

  isBusy(name: string): boolean {
    return this.busyServices.includes(name);
  }

  private async lifecycle(name: string, call: () => Promise<TaskView>) {
    if (this.isBusy(name)) return;
    this.busyServices = [...this.busyServices, name];
    try {
      const view = await call();
      // 命令返回的就是操作之后的视图，先就地写进去，界面不用等 refresh。
      this.patch(name, view);
    } catch (e) {
      this.showError(e);
    } finally {
      this.busyServices = this.busyServices.filter((n) => n !== name);
    }
    await this.refresh();
  }

  async deleteTask(name: string) {
    try {
      await api.deleteTask(name);
    } catch (e) {
      this.showError(e);
      return;
    }
    if (this.selectedName === name) this.selectedName = null;
    this.editing = false;
    await this.refresh();
  }

  // ---- M3 §3.4：其他 LaunchAgents ----

  /** 打开接管确认框。 */
  startAdopt(label: string) {
    this.adoptTarget = label;
  }

  /** 关掉接管确认框（取消、Esc、接管成功之后）。 */
  cancelAdopt() {
    this.adoptTarget = null;
  }

  /**
   * 接管一个外部 agent，成功后选中它变成的那个任务。
   *
   * 两组都要重拉：那一行从「其他 LaunchAgents」挪到了「Launchkeeper 管理」，
   * 只刷一边会让它同时出现在两处或者一处都不在。
   * 抛出去（而不是只弹 toast）让确认框知道自己该不该关。
   */
  async adopt(label: string, reload: boolean): Promise<string> {
    const view = await api.adoptAgent(label, reload);
    this.adoptTarget = null;
    await this.refreshAll();
    this.selectedExternal = null;
    this.creating = false;
    this.formDirty = false;
    this.editing = false;
    this.selectedName = view.name;
    return view.name;
  }

  /** 撤销接管：plist 按字节还原，任务行消失，右侧回到空白。 */
  async unadopt(name: string) {
    try {
      await api.unadoptTask(name);
    } catch (e) {
      this.showError(e);
      return;
    }
    if (this.selectedName === name) this.selectedName = null;
    this.formDirty = false;
    this.editing = false;
    await this.refreshAll();
  }

  /**
   * 未接管 agent 的启用/禁用：只走 launchctl，plist 一个字节都不动。
   *
   * 命令返回的就是操作之后的那一行，先就地写进去，界面不用等 loadExternal。
   */
  async externalSetEnabled(label: string, enabled: boolean) {
    try {
      const view = await api.externalSetEnabled(label, enabled);
      this.patchExternal(label, view);
    } catch (e) {
      this.showError(e);
    }
    await this.loadExternal();
  }

  /** 未接管 agent 的「立即运行」：kickstart，没有运行历史可记。 */
  async externalRun(label: string) {
    try {
      await api.externalRun(label);
    } catch (e) {
      this.showError(e);
      return;
    }
    await this.loadExternal();
  }

  private patchExternal(label: string, view: ExternalAgentView) {
    const idx = this.externalAgents.findIndex((a) => a.label === label);
    if (idx < 0) return;
    const copy = this.externalAgents.slice();
    copy[idx] = view;
    this.externalAgents = copy;
  }
}

export const taskStore = new TaskStore();
