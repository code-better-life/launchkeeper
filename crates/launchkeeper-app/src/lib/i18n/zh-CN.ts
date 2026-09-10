// 简体中文词典（PRD M3 项 5「界面中英文切换」，docs/M3-design.md §5）。
//
// 这一份是**基准**：`TranslationKey` 就是它的键集合，`en.ts` 被声明成
// `Record<TranslationKey, string>`，所以少一个键、多一个键都是编译错误，不用
// 等运行时才发现某个界面上是空的。`src/lib/i18n/i18n.test.ts` 再从运行时确认
// 一遍，并且拦住 .svelte 里新写死的中文。
//
// 键是扁平的、按界面区域分命名空间（`list.group.managed`）；插值写成
// `{name}`，由 `translate()` 做字符串替换，没有复数/性数规则引擎——英文里
// 需要单复数的地方（"1 second ago" / "3 seconds ago"）老老实实各给一个键
// （`*_one` / `*_other`），中文两个键写一样的字。

export const zhCN = {
  // ---- 通用 ----------------------------------------------------------
  "common.loading": "加载中…",
  "common.close": "关闭",
  "common.cancel": "取消",
  "common.ok": "确定",
  "common.save": "保存",
  "common.saving": "保存中…",
  "common.refresh": "刷新",
  "common.remove": "删除",
  "common.placeholder": "—",

  // ---- 顶栏 ----------------------------------------------------------
  "topbar.search_placeholder": "搜索任务…",
  "topbar.refresh_title": "重新加载任务列表和其他 LaunchAgents",
  "topbar.new_task": "新建任务",
  "topbar.settings": "设置",

  // ---- 任务列表 ------------------------------------------------------
  "list.empty.title": "还没有任务",
  "list.empty.hint": "点击右上角“新建任务”开始",
  "list.empty.no_match": "没有匹配的任务",
  "list.group.managed": "Launchkeeper 管理",
  "list.group.external": "其他 LaunchAgents",
  "list.group.external_note": "· 只读，可接管",
  "list.group.rescan_title": "重新扫描 ~/Library/LaunchAgents",
  "list.group.scanning": "扫描中…",
  "list.group.more_collapsed": "还有 {n} 个不支持接管",
  "list.group.more_expand": "展开",
  "list.group.more_collapse": "收起",
  "list.group.more_help":
    "这些 plist 用了 Launchkeeper 还表达不了的触发方式或键（WatchPaths、Sockets 之类），或者程序装在 App 包、/Library、系统目录里。它们只读展示：可以启用、禁用、立即运行，但不能接管。",
  "list.group.count": "共 {n}",
  "list.group.count_filtered": "匹配 {n} / 共 {total}",

  // ---- 列表排序（M4 §2）----------------------------------------------
  "list.sort.label": "排序方式",
  "list.sort.title": "选择这一组的排序方式",
  "list.sort.name": "按名称",
  "list.sort.last_run": "按最近运行",
  "list.sort.status": "按状态",
  "list.sort.next_run": "按下次触发",

  // ---- 状态点的 tooltip ----------------------------------------------
  "status.ok": "上次运行成功",
  "status.failed": "上次运行失败",
  "status.running": "运行中",
  "status.never": "从未运行",
  "status.disabled": "未启用",
  "status.stopped": "已停止",

  // ---- 任务行 --------------------------------------------------------
  "row.uptime": "运行 {duration}",
  "row.pid": "pid {pid}",
  "row.never_run": "从未运行",
  "row.start_service": "启动服务",
  "row.stop_service": "停止服务",
  "row.disabled_badge": "已停用",
  "row.toggle_on": "已启用，点击停用",
  "row.toggle_off": "已停用，点击启用",

  // ---- 右键菜单 ------------------------------------------------------
  "menu.run_now": "立即运行",
  "menu.start": "启动",
  "menu.stop": "停止",
  "menu.restart": "重启",
  "menu.edit": "编辑",
  "menu.enable": "启用",
  "menu.disable": "禁用",
  "menu.reveal_script": "在 Finder 中显示脚本",
  "menu.reveal_plist": "在 Finder 中显示 plist",
  "menu.adopt": "接管",
  "menu.delete": "删除",

  // ---- 确认框 --------------------------------------------------------
  "confirm.unsaved.title": "有未保存的改动",
  "confirm.unsaved.create": "新建任务还没有保存，放弃这次新建？",
  "confirm.unsaved.edit": "当前任务的改动还没有保存，放弃这些改动？",
  "confirm.unsaved.discard": "放弃",
  "confirm.delete.title": "删除任务",
  "confirm.delete.message":
    "确定要删除「{name}」吗？这会同时移除它的 launchd job、数据库记录和日志，且无法撤销。",
  "confirm.unadopt.title": "撤销接管",
  "confirm.unadopt.message":
    "会把 {path} 按字节还原成接管前的样子，并删除 Launchkeeper 里的这个任务（运行历史一并删除，日志文件保留）。接管之后在 Launchkeeper 里对它做过的改动——触发方式、环境变量、超时——都会随之丢弃，这个 LaunchAgent 会继续按它原来的定义运行。",
  "confirm.unadopt.confirm": "撤销接管",
  "confirm.unadopt.fallback_path": "原 plist",

  // ---- 任务详情 ------------------------------------------------------
  "detail.create_title": "新建任务",
  "detail.empty": "选择左侧的任务查看详情",
  "detail.adopted_from": "接管自",
  "detail.unknown_path": "（未知路径）",
  "detail.undo_adopt": "撤销接管",
  "detail.tab.config": "配置",
  "detail.tab.history": "运行历史",
  "detail.edit": "编辑",
  "detail.edit_title": "编辑这个任务的配置",

  // ---- 配置页的只读概览（M3 §3.4 追补）------------------------------
  "summary.none": "无",
  "summary.working_dir_default": "$HOME（未设置）",
  "summary.log_dir": "日志目录",
  "summary.start_at_login": "登录时自动启动",
  "summary.start_at_login_note": "= 已注册到 launchd",
  "summary.on": "已开启",
  "summary.off": "已关闭",
  "summary.keep_alive_suffix": " · 保持常驻",

  // ---- 运行历史 ------------------------------------------------------
  "history.empty": "还没有运行记录",
  "history.col.id": "#",
  "history.col.started": "开始时间",
  "history.col.duration": "时长",
  "history.col.result": "结果",
  "history.col.stop": "结束方式",
  "history.col.source": "来源",
  "history.source.manual": "手动",
  "history.source.scheduled": "计划",
  "history.stdout": "标准输出",
  "history.stderr": "标准错误",

  // ---- 日志 ----------------------------------------------------------
  "log.truncated": "仅显示末尾 {tail} 字节（文件共 {total} 字节，已截断）",
  "log.empty": "（空）",

  // ---- 任务表单 ------------------------------------------------------
  "form.name": "任务名（唯一标识）",
  "form.name_placeholder": "例如 report-sync",
  "form.display_name": "显示名称",
  "form.display_name_placeholder": "给人看的名字",
  "form.description": "描述",
  "form.script": "脚本",
  "form.script_placeholder": "/absolute/path/to/script.py",
  "form.run_with": "运行方式",
  "form.run_with_manual": "手动选择的运行方式",
  "form.interp_args": "解释器参数: {args}",
  "form.args": "参数",
  "form.args_add": "+ 添加参数",
  "form.working_dir": "工作目录",
  "form.working_dir_placeholder": "留空则为 $HOME",
  "form.env": "环境变量",
  "form.env_key_placeholder": "KEY",
  "form.env_value_placeholder": "value",
  "form.env_add": "+ 添加环境变量",
  "form.trigger": "触发规则",
  "form.keep_alive": "保持常驻（KeepAlive，仅“登录时启动”和“手动启停”可用）",
  "form.keep_alive_hint":
    "崩溃后自动重启，手动停止不会重启；注意：勾选后「启用」即会启动。",
  "form.timeout": "超时限制",
  "form.timeout_check": "限制单次运行时长",
  "form.timeout_none": "无限制",
  "form.tags": "标签",
  "form.tag_placeholder": "输入后回车添加",
  "form.favorite": "收藏（显示在菜单栏）",
  "form.notify_on_fail": "失败时通知",
  "form.error.name_required": "任务名不能为空",
  "form.error.script_required": "脚本路径不能为空",
  "form.error.script_absolute": "脚本必须是绝对路径（以 / 开头）",
  "form.error.timeout_number": "超时时长必须填一个大于 0 的数字",
  "form.error.script_missing": "脚本不存在：{path}",
  "form.error.not_executable":
    "请选择运行方式：没选就是交给 launchd 直接运行这个文件，而它没有可执行权限",
  "form.error.interval_number": "间隔必须填一个大于 0 的数字",
  "form.error.hour_range": "小时必须是 0–23 之间的整数",
  "form.error.minute_range": "分钟必须是 0–59 之间的整数",
  "form.error.day_range": "日期必须是 1–31 之间的整数，或留空",

  // ---- 时间单位（超时下拉框、间隔下拉框共用）--------------------------
  "unit.seconds": "秒",
  "unit.minutes": "分钟",
  "unit.hours": "小时",

  // ---- 触发规则编辑器 ------------------------------------------------
  "trigger.at_login": "登录时启动",
  "trigger.interval": "按间隔重复",
  "trigger.calendar": "按日历时间点",
  "trigger.manual": "手动启停（常驻服务）",
  "trigger.manual_hint":
    "不写任何自动触发：任务只在你按「启动」时运行，按「停止」时结束，适合 bridge、监听器这类需要一直跑着的进程。",
  "trigger.every": "每",
  "trigger.run_once": "运行一次",
  "trigger.hour_suffix": "时",
  "trigger.minute_suffix": "分",
  "trigger.weekday_hint": "不选表示每天",
  "trigger.monthly_prefix": "每月",
  "trigger.monthly_suffix": "号（留空表示不限日期）",
  "trigger.day_placeholder": "不限",
  "trigger.add_entry": "+ 添加时间点",
  "weekday.short.0": "日",
  "weekday.short.1": "一",
  "weekday.short.2": "二",
  "weekday.short.3": "三",
  "weekday.short.4": "四",
  "weekday.short.5": "五",
  "weekday.short.6": "六",

  // ---- 运行方式选择器 ------------------------------------------------
  "interp.scanning": "扫描解释器…",
  "interp.choose": "选择运行方式",
  "interp.group.recommended": "推荐",
  "interp.group.project": "项目内",
  "interp.group.project_empty": "项目内 · 此项目没有 .venv 或 uv.lock",
  "interp.group.path": "PATH 里的其他解释器",
  "interp.custom": "自定义路径…",
  "interp.custom_placeholder": "/absolute/path/to/interpreter",
  "interp.custom_confirm": "确定",
  "interp.badge_default": "默认",
  "interp.direct": "直接执行",
  "interp.script_itself": "脚本本身",

  // ---- 设置 ----------------------------------------------------------
  "settings.title": "设置",
  "settings.language": "语言",
  "settings.language.system": "跟随系统",
  "settings.language.zh": "中文",
  "settings.language.en": "English",
  "settings.data_dir": "数据目录",
  "settings.notify": "失败通知",
  "settings.notify.task": "定时任务运行失败时通知",
  "settings.notify.service": "常驻服务崩溃并自动重启时通知",
  "settings.notify.hint":
    "仍然只对勾选了「失败时通知」的任务生效；这里是总开关，任务详情里的开关是每个任务自己的开关。",
  "settings.menubar": "菜单栏",
  "settings.menubar.login": "登录时启动 Launchkeeper",
  "settings.history_limit": "运行历史保留条数",
  "settings.coming_soon": "即将支持",
  "settings.load_failed": "加载设置失败",

  // M4 AI / theme -------------------------------------------------------
  // ---- 外观（M4 §4）--------------------------------------------------
  "settings.theme": "外观",
  "settings.theme.system": "跟随系统",
  "settings.theme.light": "浅色",
  "settings.theme.dark": "深色",
  // ---- AI（M4 §1 / §4）-----------------------------------------------
  "settings.ai": "AI",
  "settings.ai.hint": "「AI 解读」用这里配置的模型。任务的环境变量只送变量名，值不出本机。",
  "settings.ai.provider": "服务商",
  "settings.ai.provider.anthropic": "Anthropic",
  "settings.ai.provider.openai": "OpenAI 兼容",
  "settings.ai.base_url": "Base URL",
  "settings.ai.base_url.hint": "留空使用默认地址",
  "settings.ai.model": "模型",
  "settings.ai.key": "API Key",
  "settings.ai.key.placeholder": "粘贴 Key，保存后不再显示",
  "settings.ai.key.saved": "已保存",
  "settings.ai.key.missing": "未配置",
  "settings.ai.key.save": "保存",
  "settings.ai.key.clear": "清除",
  "settings.ai.key.hint": "存在数据目录的 ai-key 文件里（仅本用户可读），不会出现在任何命令输出、日志或 plist 里。",
  "settings.ai.test": "测试连接",
  "settings.ai.testing": "连接中…",
  "settings.ai.test_ok": "连接成功：{reply}",
  // ---- 给 AI 的提示词（M4 §4）----------------------------------------
  "settings.prompt": "给 AI 的提示词",
  "settings.prompt.hint": "复制给终端里的 AI 助手，它就知道这台机器上有 launchkeeper 以及该怎么用。",
  "settings.prompt.copy": "复制",
  "settings.prompt.copied": "已复制到剪贴板",
  "settings.prompt.copy_failed": "复制失败：{message}",
  // ---- AI 解读标签页（M4 §1 / §4）------------------------------------
  "detail.tab.insight": "AI 解读",
  "insight.explain": "解读",
  "insight.refresh": "重新解读",
  "insight.running": "解读中…",
  "insight.meta": "{time} · {model}",
  "insight.empty.title": "还没有解读过这个任务",
  "insight.empty.body":
    "点「解读」把任务定义、最近 10 次运行和最新一次日志的尾部发给模型，让它说明这个任务在做什么、最近健不健康、失败可能是什么原因。",
  "insight.empty.kept": "结果会存下来，下次打开这一页直接显示，直到你再点一次「重新解读」。",
  "insight.empty.privacy": "环境变量只送变量名，值不出本机；日志最多送末尾 16 KiB。",
  "insight.open_settings": "打开设置",

  // ---- 接管确认框 ----------------------------------------------------
  "adopt.title": "接管 {label}",
  "adopt.lead_before": "名字和文件位置保持不变。将做以下改动，原文件先备份为",
  "adopt.lead_after": "，随时可以撤销。",
  "adopt.loading": "正在读取 plist…",
  "adopt.reload_check": "接管后立即重新加载（bootout + bootstrap）",
  "adopt.no_reload_hint":
    "不重新加载：文件已经改了，但 launchd 里跑的还是旧定义，下次登录才生效。",
  "adopt.busy": "接管中…",
  "adopt.confirm_reload": "接管并重新加载",
  "adopt.confirm": "接管",

  // ---- 其他 LaunchAgents --------------------------------------------
  "external.subtitle.managed": "已由 Launchkeeper 管理",
  "external.subtitle.not_adoptable": "{reason} · 仅展示",
  "external.subtitle.adoptable": "手写 plist · 无运行历史",
  "external.reason.fallback": "不可接管",
  "external.why_not_adoptable": "为什么不能接管",
  "external.status.unloaded": "未加载到 launchd",
  "external.status.running": "运行中 · pid {pid}",
  "external.status.loaded": "已加载，等待触发",
  "external.headline.managed": "Launchkeeper 管理",
  "external.headline.handwritten": "手写 plist",
  "external.headline.loaded": "已加载",
  "external.headline.unloaded": "未加载",
  "external.log.merged": "{path}（合并）",
  "external.log.split": "{out} · 错误 {err}",
  "external.log.err_only": "错误 {err}",
  "external.log.none": "未设置（输出被 launchd 丢弃）",
  "external.adopt.managed_tip": "这个 plist 已经由 Launchkeeper 管理",
  "external.adopt.tip":
    "把它纳入 Launchkeeper 管理（名字和位置不变，原文件备份为 .bak）",
  "external.diff.kept": "{keys} 保持不变",
  "external.diff.join": "、",
  "external.run_disabled_tip": "没有加载到 launchd，先启用再运行",
  "external.enable_tip": "只对 launchd 生效，plist 文件不会被改动",
  "external.detail.run_tip": "launchctl kickstart，不会记运行历史",
  "external.detail.run_disabled_tip": "没有加载到 launchd，先启用",
  "external.detail.lead":
    "这个 LaunchAgent 不是 Launchkeeper 创建的。你可以直接启用、禁用或运行它，但没有运行历史和日志。接管后这些都会有，名字和位置不变。",
  "external.detail.program": "程序",
  "external.detail.trigger": "触发",
  "external.detail.working_dir": "工作目录",
  "external.detail.log": "日志",
  "external.detail.plist": "plist",
  "external.detail.launchd": "launchd 状态",
  "external.detail.working_dir_unset": "未设置（$HOME）",
  "external.detail.last_exit": "上次退出码 {code}",

  // ---- 时间/时长/退出码的格式化（lib/format.ts）----------------------
  "fmt.rel.now": "刚刚",
  "fmt.rel.second_one": "{n} 秒前",
  "fmt.rel.second_other": "{n} 秒前",
  "fmt.rel.minute_one": "{n} 分钟前",
  "fmt.rel.minute_other": "{n} 分钟前",
  "fmt.rel.hour_one": "{n} 小时前",
  "fmt.rel.hour_other": "{n} 小时前",
  "fmt.rel.day_one": "{n} 天前",
  "fmt.rel.day_other": "{n} 天前",
  "fmt.rel.month_one": "{n} 个月前",
  "fmt.rel.month_other": "{n} 个月前",
  "fmt.rel.year_one": "{n} 年前",
  "fmt.rel.year_other": "{n} 年前",
  "fmt.dur.ms": "{n} 毫秒",
  "fmt.dur.sec": "{n} 秒",
  "fmt.dur.min_sec": "{m} 分 {s} 秒",
  "fmt.dur.hour_min": "{h} 时 {m} 分",
  "fmt.up.sec": "{s} 秒",
  "fmt.up.min_sec": "{m} 分 {s} 秒",
  "fmt.up.hour_min": "{h} 小时 {m} 分",
  "fmt.up.day_hour": "{d} 天 {h} 小时",
  "fmt.stop.stopped": "手动停止",
  "fmt.stop.timeout": "超时",
  "fmt.stop.timeout_killed": "超时，已被终止",
  "fmt.exit.running": "运行中…",
  "fmt.exit.signal": "被外部信号终止（非 Launchkeeper 操作，如注销、kill）",
  "fmt.exit.success": "成功",
  "fmt.exit.failed": "失败（退出码 {code}）",
  // M4 CLI docs / external disable
  "confirm.external_disable.title": "禁用别的软件的 LaunchAgent？",
  "confirm.external_disable.message":
    "{label} 是别的软件自己装的 LaunchAgent（自动更新、云同步、后台助手一类），Launchkeeper 并没有接管它。不可接管的原因：{reason}。\n禁用它很可能让那个软件不再自动更新 / 不再同步，甚至直接不能用。\nplist 文件不会被改动，只是从 launchd 卸载；随时可以再启用回来。",
  "confirm.external_disable.confirm": "仍然禁用",
  // M3 关于
  "settings.about": "关于",
  "settings.about.version": "版本 {version}",
  "settings.about.tagline": "用 launchd 管理 macOS 上的定时任务与常驻服务。",
  "settings.about.repo": "GitHub 项目主页",
  "settings.about.issues": "反馈问题",
  "settings.about.coffee": "请我喝杯咖啡 ☕",
  "settings.about.open_failed": "打开链接失败：{error}",
  // 解释器来源
  "interp.origin.system": "系统",
  "interp.origin.homebrew": "Homebrew",
  "interp.origin.project_venv": "项目 .venv",
  "interp.origin.uv": "uv 安装",
  "interp.origin.user_local": "用户安装",
  "interp.origin.path": "PATH",
  "interp.origin.custom": "自定义",
  "external.trigger_unsupported": "不支持的触发方式",
} as const;

/** 词典的键集合。`en.ts` 必须一个不多一个不少地实现它。 */
export type TranslationKey = keyof typeof zhCN;
