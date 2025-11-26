pub fn timestamp_now() -> i64 {
    chrono::Local::now().timestamp()
}

/// 获取 repo_id 的最后一段字符串（用 : 分割）
pub fn get_repo_id_last_part(repo_id: &str) -> String {
    repo_id.split(':').next_back().unwrap_or(repo_id).to_string()
}

/// 获取 node_id 的最后一段字符串（用 : 分割）
pub fn get_node_id_last_part(node_id: &str) -> String {
    node_id.split(':').next_back().unwrap_or(node_id).to_string()
}
