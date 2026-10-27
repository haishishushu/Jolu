//! 每日签到的公共结果；不包含凭证、设备 ID 或服务端原始响应。
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum State {
    Unknown,
    Ready,
    Claimed,
    Inactive,
    Unsupported,
    Unauthorized,
    Forbidden,
    RateLimited,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Outcome {
    pub state: State,
    pub message: String,
    /// 仅服务端提供语义明确的数字才有值；未知不补零。
    pub reward: Option<f64>,
    pub unit: String,
    /// 服务端轮次或有官方依据的日期边界；未知为 None，不能按本机日期跳过请求。
    pub period_key: Option<String>,
    pub reset_at: Option<i64>,
    /// 官方响应 Date（Unix 秒），用于抵消本机时钟偏差；旧记录缺省为空。
    #[serde(default)]
    pub server_time: Option<i64>,
    /// 官方可查汇总及其范围；缺字段不从当前奖励或连续天数补造历史。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub official_statistics: Option<OfficialCheckinStatistics>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct OfficialCheckinStatistics {
    pub unit: String,
    pub scope: String,
    pub week: Option<OfficialCheckinTotals>,
    pub month: Option<OfficialCheckinTotals>,
    pub all: Option<OfficialCheckinTotals>,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct OfficialCheckinTotals {
    pub days: Option<u32>,
    pub reward: Option<f64>,
}

impl Outcome {
    pub(crate) fn new(state: State, message: impl Into<String>) -> Self {
        Self {
            state,
            message: message.into(),
            reward: None,
            unit: "积分".into(),
            period_key: None,
            reset_at: None,
            server_time: None,
            official_statistics: None,
        }
    }
}
