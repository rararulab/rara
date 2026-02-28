// Copyright 2025 Crrow
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Proactive agent prompt builder.

/// Build the user prompt for a proactive review cycle.
///
/// This is used by the proactive worker to construct the input for the
/// kernel-spawned agent process.
pub fn build_user_prompt(activity_summary: &str) -> String {
    format!(
        "以下是最近24小时的用户活动摘要：\n\n{}\n\n根据你的行为策略，决定是否需要主动联系用户。\\
         n你可以使用工具查询更多信息、发送通知、或安排后续任务。\\
         n如果没有值得做的事情，直接回复 DONE。",
        activity_summary
    )
}
