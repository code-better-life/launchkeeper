// 把要塞进 app bundle 的两个辅助二进制复制成 Tauri `externalBin` 要求的名字，
// 即 `src-tauri/binaries/<name>-<target-triple>`：
//
//   launchkeeper-runner   plist 里的 ProgramArguments 指向它（必经之路）
//   launchkeeper          CLI；跟着 app 一起被同一个证书签名，Cask 用户
//                         `ln -s /Applications/Launchkeeper.app/Contents/MacOS/launchkeeper`
//                         就能用，不必再装一遍 Formula
//
// 两个都在 `Contents/MacOS/` 里并排放着，正好是 CLI 的
// `locate_runner_source` 和 app 的 `locate_runner` 都会找的"同目录"布局。
//
// 只在打包前跑（`pnpm bundle` / `scripts/release/build-dmg.sh` / CI），
// **不要**接到 `beforeDevCommand` 上：dev 模式下 app 会直接从
// `target/{debug,release}/` 找 runner（见 `src-tauri/src/state.rs` 的
// `runner_plan`），多复制一份只会让两边不同步。`externalBin` 也因此留在
// `tauri.bundle.conf.json` 这个叠加配置里，而不是主 `tauri.conf.json`：
// Tauri 在 dev 时也会校验 externalBin，主配置里写了就没法在没打包的情况下
// `pnpm tauri dev`。
//
// 用法：
//   node scripts/prepare-runner.mjs [--profile release|debug]

import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, existsSync, chmodSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const appDir = resolve(here, "..");
const workspace = resolve(appDir, "..", "..");

/** cargo 的 bin 名 -> 放进 bundle 后的名字（这里一样，列出来是为了明确）。 */
const BINARIES = ["launchkeeper-runner", "launchkeeper"];

const args = process.argv.slice(2);
const profileFlag = args.indexOf("--profile");
const profile = profileFlag === -1 ? "release" : args[profileFlag + 1];
if (profile !== "release" && profile !== "debug") {
  throw new Error(`--profile 只能是 release 或 debug，收到 ${profile}`);
}

// rustc 是唯一权威的 target triple 来源；从 process.arch 猜会在交叉编译时错。
const triple = execFileSync("rustc", ["-vV"], { encoding: "utf8" })
  .split("\n")
  .find((l) => l.startsWith("host:"))
  ?.slice("host:".length)
  .trim();
if (!triple) throw new Error("无法从 `rustc -vV` 解析 host target triple");

const destDir = join(appDir, "src-tauri", "binaries");
mkdirSync(destDir, { recursive: true });

for (const name of BINARIES) {
  const source = join(workspace, "target", profile, name);
  if (!existsSync(source)) {
    throw new Error(
      `找不到 ${source}\n先构建它：cargo build ${BINARIES.map((b) => `--bin ${b}`).join(" ")} --${profile === "release" ? "release" : "profile=dev"}`,
    );
  }
  const dest = join(destDir, `${name}-${triple}`);
  copyFileSync(source, dest);
  chmodSync(dest, 0o755);
  console.log(`${source} -> ${dest}`);
}
