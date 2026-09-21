use super::CmdResult;
use crate::{
    cmd::StringifyErr as _,
    core::traffic_usage::{GroupBy, TrafficUsageCollector, UsageFilter, UsageRange, UsageRow, UsageStatus},
};

/// 按时间范围与维度查询流量用量聚合结果
#[tauri::command]
pub async fn get_traffic_usage(
    range: UsageRange,
    group_by: GroupBy,
    filter: Option<UsageFilter>,
) -> CmdResult<Vec<UsageRow>> {
    TrafficUsageCollector::global()
        .query(range, group_by, filter.unwrap_or_default())
        .await
        .stringify_err()
}

/// 清空全部流量用量历史
#[tauri::command]
pub async fn clear_traffic_usage() -> CmdResult {
    TrafficUsageCollector::global().clear().await.stringify_err()
}

/// 查询流量用量采集状态
#[tauri::command]
pub async fn get_traffic_usage_status() -> CmdResult<UsageStatus> {
    TrafficUsageCollector::global().status().await.stringify_err()
}
