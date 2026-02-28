//! Proactive agent prompt builder.

/// Build the user prompt for a proactive review cycle.
///
/// This is used by the proactive worker to construct the input for the
/// kernel-spawned agent process.
pub fn build_user_prompt(activity_summary: &str) -> String {
    format!(
        "以下是最近24小时的用户活动摘要：\n\n{}\n\n根据你的行为策略，\
         决定是否需要主动联系用户。\n你可以使用工具查询更多信息、发送通知、或安排后续任务。\\
         n如果没有值得做的事情，直接回复 DONE。",
        activity_summary
    )
}
