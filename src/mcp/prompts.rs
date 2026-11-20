use crate::mcp::types::{
    GetPromptResult, JsonRpcError, PromptArgument, PromptDefinition, PromptListResult,
    PromptMessage, PromptMessageContent,
};

/// Retrieve all supported MCP Prompt definitions.
pub fn get_prompt_definitions() -> Vec<PromptDefinition> {
    vec![
        PromptDefinition {
            name: "inspect_screen".to_string(),
            description: "Capture the current desktop screen and analyze its layout, visible windows, and interactive elements.".to_string(),
            arguments: vec![
                PromptArgument {
                    name: "focus".to_string(),
                    description: "Target window title, application name, or UI region of interest.".to_string(),
                    required: false,
                },
            ],
        },
        PromptDefinition {
            name: "verify_ui_action".to_string(),
            description: "Verify that an automation step produced expected UI state changes by comparing before-and-after screen captures.".to_string(),
            arguments: vec![
                PromptArgument {
                    name: "action".to_string(),
                    description: "Description of the action just executed (e.g. 'Clicked login submit button').".to_string(),
                    required: true,
                },
            ],
        },
        PromptDefinition {
            name: "diagnose_system".to_string(),
            description: "Inspect screen capture subsystem, connected displays, clipboard, and active windows.".to_string(),
            arguments: vec![],
        },
    ]
}

/// Handle `prompts/list` request.
pub fn handle_prompts_list() -> PromptListResult {
    PromptListResult {
        prompts: get_prompt_definitions(),
    }
}

/// Handle `prompts/get` request.
pub fn handle_prompts_get(params: &serde_json::Value) -> Result<GetPromptResult, JsonRpcError> {
    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Missing required parameter 'name'".to_string(),
            data: None,
        })?;

    let arguments = params.get("arguments").and_then(|v| v.as_object());

    match name {
        "inspect_screen" => {
            let focus = arguments
                .and_then(|args| args.get("focus"))
                .and_then(|v| v.as_str())
                .unwrap_or("entire desktop");
            Ok(GetPromptResult {
                description: Some("Inspect desktop layout and UI components".to_string()),
                messages: vec![PromptMessage {
                    role: "user".to_string(),
                    content: PromptMessageContent::Text {
                        text: format!(
                            "Please capture the screen using the 'capture_screen' tool focusing on '{focus}'. \
                             Analyze the visible windows, text fields, buttons, and coordinate layout, \
                             then provide recommended interaction coordinates."
                        ),
                    },
                }],
            })
        }
        "verify_ui_action" => {
            let action = arguments
                .and_then(|args| args.get("action"))
                .and_then(|v| v.as_str())
                .unwrap_or("UI action");
            Ok(GetPromptResult {
                description: Some("Verify visual state transition".to_string()),
                messages: vec![PromptMessage {
                    role: "user".to_string(),
                    content: PromptMessageContent::Text {
                        text: format!(
                            "Action executed: '{action}'. \
                             Please capture the latest screen state using 'capture_screen' (optionally with 'diffWith' or 'waitChange'). \
                             Verify whether the expected changes occurred and report any visual errors or unexpected dialogs."
                        ),
                    },
                }],
            })
        }
        "diagnose_system" => Ok(GetPromptResult {
            description: Some("System diagnostics inspection".to_string()),
            messages: vec![PromptMessage {
                role: "user".to_string(),
                content: PromptMessageContent::Text {
                    text: "Please run the 'doctor' tool to check display capture health, active monitors, and window enumeration.".to_string(),
                },
            }],
        }),
        unknown => Err(JsonRpcError {
            code: -32602,
            message: format!("Prompt '{unknown}' not found"),
            data: None,
        }),
    }
}
