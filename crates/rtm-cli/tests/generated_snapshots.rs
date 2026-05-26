#![allow(clippy::expect_used, clippy::unwrap_used)]

#[test]
fn generated_mcp_tool_list_is_stable() {
    let tools: serde_json::Value =
        serde_json::from_str(rtm_cli::generated::mcp_tools::TOOL_LIST_JSON).expect("tools json");
    insta::assert_json_snapshot!(tools);
}

#[test]
fn generated_mcp_tool_names_are_stable() {
    insta::assert_debug_snapshot!(rtm_cli::generated::mcp_tools::TOOL_NAMES);
}

#[test]
fn generated_cli_help_is_stable() {
    insta::assert_snapshot!(
        "cli_help",
        [
            rtm_cli::generated::cli_help::MCP_ABOUT,
            rtm_cli::generated::cli_help::KILL_ABOUT,
            rtm_cli::generated::cli_help::STATUS_ABOUT,
            rtm_cli::generated::cli_help::VERSION_ABOUT,
            rtm_cli::generated::cli_help::WATCHERS_ABOUT,
        ]
        .join("\n")
    );
}

#[test]
fn generated_admin_skill_doc_is_stable() {
    insta::assert_snapshot!(include_str!("../templates/SKILL.md"));
}
