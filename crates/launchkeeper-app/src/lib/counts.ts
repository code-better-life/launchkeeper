// 分类数量统计（PRD M4 项 3，docs/M4-design.md §3）。
//
// 顶栏的标签 chip 上那个数字。纯函数，收一份任务列表还回一张表；"哪一份任务
// 列表"是这里最要紧的决定，见 `tagCounts` 的注释。

import type { TaskView } from "./bindings";

/**
 * 每个标签各有多少个任务。
 *
 * 传进来的应该是**只按搜索过滤过**的列表，不含标签筛选本身（`taskStore` 的
 * `searchFiltered`）：
 *
 * - 计数跟着搜索走，因为搜索是"我在找什么"，这时"带这个标签的还剩几个"正是
 *   用户要的答案；
 * - 计数**不**跟着标签筛选走，因为一旦跟着，点中「同步」之后所有别的标签都会
 *   变成 0，而那个 0 说的不是"没有同步以外的任务"，是"没有同时带两个标签的
 *   任务"——用户看到的却是一排失效的按钮，反而不敢点第二下。
 *
 * 返回的是 `Map`：键是用户自己起的标签名，用普通对象会和 `constructor`、
 * `__proto__` 这种名字撞上。
 */
export function tagCounts(tasks: TaskView[]): Map<string, number> {
  const counts = new Map<string, number>();
  for (const task of tasks) {
    // 同一个任务上重复的标签只算一次——数的是任务，不是标签出现的次数。
    for (const tag of new Set(task.tags)) {
      counts.set(tag, (counts.get(tag) ?? 0) + 1);
    }
  }
  return counts;
}
