<system-reminder>
<goal-state>
目标：{objective}
状态：推进中 (Active)
Tokens：{tokens} | 已用时长：{elapsed}
</goal-state>

{bail_preface}{plan_pointer}{verifier_gaps}{strategist_note}{reverify_block}目标尚未完成——请继续推进行动。下一步骤：
{next_step}

请时刻维持您的 {todo_tool} 清单处于最新状态（保证存在至少 1 个 `in_progress` 步骤与描述明确的 `activeForm`）。在每次代码变更后务必运行针对性测试，而非仅在最后才运行。测试必须在真实路径上驱动已交付的代码——严禁硬编码返回值、严禁跳过被测对象、严禁重构被测逻辑。请仅使用您的暂存目录 {scratch_dir} {scratch_status} 保存截获的测试输出、临时脚本与随用随弃的产物，绝不要使用全局共享的 `/tmp/...`。使用用户、系统或项目既有的默认设置作为执行依赖与环境状态。严禁将 `HOME`、`CARGO_HOME`、`RUSTUP_HOME`、包管理器主目录、虚拟环境、缓存或配置目录指向暂存区，亦不可保留对暂存区的持久化引用（暂存区将在目标结束时被销毁）。
规划中的 `{SCRATCH}` 占位符将解析至该路径。审定者会审查您提交的测试与保存的凭据而非代为重建——请务必留下真实可信的凭证，否则必定会被驳回。
请自行运行规划中 `## Verification plan` 章节指定的步骤，并确认其列出的观测结果完全符合。测试框架将在本轮操作后自动评估完成情况，并在适当时对相同步骤进行对抗性复核，将任何尚未解决的审定缺口行内内嵌于上。
</system-reminder>
