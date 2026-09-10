// `defineConfig` 从 vitest/config 引：它就是 vite 那个，只是多认识一个 `test`
// 字段（下面钉时区用），不然 svelte-check 会说 `test` 不是合法的配置项。
import { defineConfig } from "vitest/config";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Tauri 在 `beforeDevCommand` 里启动 vite，端口必须固定且不能自动漂移，
// 否则 `devUrl` 指向的地址会落空。
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      // src-tauri 的改动由 cargo 自己监听，vite 重启只会互相打断。
      ignored: ["**/src-tauri/**"],
    },
  },
  build: {
    target: "safari15",
    sourcemap: true,
  },
  test: {
    env: {
      // 时区钉死（M4 §2）：`lib/nextRun.ts` 算的是墙上时钟，夏令时那两条用例
      // 需要一个真的有夏令时的时区，而作者的机器（和多半的 CI）没有。选纽约
      // 是因为它的两次跳变时刻是公开的常识：3 月第二个周日 02:00 跳到 03:00，
      // 11 月第一个周日 02:00 退回 01:00。别的测试都不看本地时区。
      TZ: "America/New_York",
    },
  },
});
