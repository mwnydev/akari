# 0006 版本控制用 Jujutsu（与 Git 共存）

- 状态：已采纳
- 日期：2026-10-19

## 背景

开发方式是 1 名开发者 + 多个并行 AI 代理。Git 的暂存区、分支切换和 worktree 对这种方式不顺手：代理做到一半的改动没有记录，出错难以整体撤回，多个代理共用一个工作区时改动混在一起。

## 决定

- 用 Jujutsu（jj 0.46.0），**与 Git 共存**（`jj git init --colocate`）：仓库仍是普通 Git 仓库，GitHub、CI、Renovate、`gh` 不受影响。
- 工作区的每次改动由 jj 自动记录进当前 change，不需要 `git add`；`jj op log` / `jj undo` 可以撤回任何一步操作（包括代理的误操作）。
- 一个任务 = 一个 change：`jj new -m "feat: …"` 开始，完成后 `jj describe` 定稿；需要拆分用 `jj split`，合并用 `jj squash`。
- 并行代理各用一个 jj workspace（`jj workspace add ../akari-<任务>`），每个代理只在自己的 change 上工作，lead 审完后 `jj rebase` 到 main 上。冲突不阻塞操作，留在 change 里解决。
- 推送：`jj bookmark set main -r <change>` 后 `jj git push -b main`。
- 提交信息仍用约定式提交，末尾带 Co-Authored-By 行。

## 后果

- 需要在每台开发机安装 jj；CI 和服务器不需要。
- 不要混用 `git commit` 和 jj 修改同一段历史；只读的 git 命令（`git log`、`git diff`）随便用。
