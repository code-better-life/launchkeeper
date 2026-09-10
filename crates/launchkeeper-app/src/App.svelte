<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { taskStore } from "./lib/stores/tasks.svelte";
  import { api } from "./lib/api";
  import { languageSetting, setLocale, t } from "./lib/i18n";
  import { setTheme, themeSetting } from "./lib/theme";
  import TopBar from "./components/TopBar.svelte";
  import TaskList from "./components/TaskList.svelte";
  import TaskDetail from "./components/TaskDetail.svelte";
  import ExternalDetail from "./components/ExternalDetail.svelte";
  import AdoptDialog from "./components/AdoptDialog.svelte";
  import Toast from "./components/Toast.svelte";
  import ConfirmDialog from "./components/ConfirmDialog.svelte";
  import Settings from "./components/Settings.svelte";

  // 纯本地状态：设置面板不影响任务列表/详情，也不需要跨组件共享，不值得
  // 放进 taskStore（docs/M3-design.md §4）。
  let settingsOpen = $state(false);

  onMount(() => {
    void taskStore.init();
    // 语言在别的什么东西画出来之前就定下来（M3 §5）：设置面板存的是
    // `AppSettings.language`，但它是整个界面的事，不能等到用户第一次打开
    // 设置面板才生效。读失败就保持"跟随系统"——一份读不出来的
    // settings.json 不该让界面停在半路上（错误已经由别处的 toast 报过）。
    // 列表排序（M4 §2）搭同一趟车：它已经用 localStorage 里那份画过第一帧，
    // 这里拿设置里那份对一次（别的窗口/CLI 改过的话以文件为准）。
    void api
      .getSettings()
      .then((s) => {
        setLocale(languageSetting(s.language));
        // 外观搭同一趟车（M4 §4）：默认（不盖 data-theme）已经是「跟随系统」，
        // 所以设置回来之前那一帧不会闪——只有显式选了浅/深色的人才会看到一次
        // 切换，而那一次也发生在第一帧画完之前。
        setTheme(themeSetting(s.theme));
        taskStore.applyListSort(s.list_sort);
      })
      .catch(() => {
        setLocale("system");
        setTheme("system");
      });
  });
  onDestroy(() => {
    taskStore.dispose();
  });

  function onWindowKeydown(e: KeyboardEvent) {
    // 确认框打开时 Esc 归它处理，这里不再叠一层。
    if (e.key === "Escape" && taskStore.creating && !taskStore.pendingAction) {
      taskStore.stopCreate();
    }
  }
</script>

<svelte:window onkeydown={onWindowKeydown} />

<main>
  <TopBar oncreate={() => taskStore.startCreate()} onsettings={() => (settingsOpen = true)} />
  <div class="layout">
    <div class="left">
      <!-- 右键菜单的「编辑」= 选中它并直接进编辑态；点行只是选中（右侧是
           只读概览）。 -->
      <TaskList onedit={(name) => taskStore.selectForEdit(name)} />
    </div>
    <div class="right">
      <!-- 选中的是别人写的 LaunchAgent 时，右侧换成只读的那一版（M3 §3.4）：
           它没有表单、没有运行历史，能做的只有运行/启用/接管。 -->
      {#if taskStore.selectedAgent && !taskStore.creating}
        <ExternalDetail agent={taskStore.selectedAgent} />
      {:else}
        <TaskDetail
          mode={taskStore.creating ? "create" : "view"}
          taskName={taskStore.creating ? null : taskStore.selectedName}
          onclose={() => taskStore.stopCreate()}
          oncreated={(name) => taskStore.finishCreate(name)}
          onsettings={() => (settingsOpen = true)}
        />
      {/if}
    </div>
  </div>
</main>

{#if taskStore.pendingAction}
  <ConfirmDialog
    title={t("confirm.unsaved.title")}
    message={taskStore.creating
      ? t("confirm.unsaved.create")
      : t("confirm.unsaved.edit")}
    confirmLabel={t("confirm.unsaved.discard")}
    danger={true}
    onconfirm={() => taskStore.confirmPending()}
    oncancel={() => taskStore.cancelPending()}
  />
{/if}

{#if taskStore.adoptTarget}
  <AdoptDialog label={taskStore.adoptTarget} />
{/if}

{#if settingsOpen}
  <Settings onclose={() => (settingsOpen = false)} />
{/if}

{#if taskStore.toastError}
  <Toast
    message={taskStore.toastError}
    kind={taskStore.toastKind}
    onclose={() => taskStore.dismissToast()}
  />
{/if}

<style>
  main {
    display: flex;
    flex-direction: column;
    height: 100%;
  }
  .layout {
    flex: 1;
    display: flex;
    min-height: 0;
  }
  .left {
    width: 320px;
    flex-shrink: 0;
    border-right: 1px solid var(--border);
    overflow: hidden;
  }
  .right {
    flex: 1;
    min-width: 0;
    overflow: hidden;
  }
</style>
