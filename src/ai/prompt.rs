use crate::model::CueRegistry;

pub fn generate_system_prompt(prefix: &str, cues: &CueRegistry) -> String {
    let expr_cues = cues
        .expression_cues()
        .into_iter()
        .map(|c| format!("[{c}]"))
        .collect::<Vec<_>>()
        .join(", ");

    let motion_cues = cues
        .motion_cues()
        .into_iter()
        .map(|c| format!("[{c}]"))
        .collect::<Vec<_>>()
        .join(", ");

    let rules = format!(
        "【Live2D 实时动作与表情嵌入规则】\n\
        在流式回复时，你可以在合适的位置直接嵌入动作与表情标签，格式为：`[标签名]`。\n\
        桌宠引擎会实时捕获该标签并触发对应的表情与肢体动作。请自然地将标签嵌入在句首或情感变化处，切勿附加多余的解释。\n\
        当前模型支持的可用标签：\n\
        - 表情标签：{expr_cues}\n\
        - 动作标签：{motion_cues}"
    );

    if prefix.trim().is_empty() {
        rules
    } else {
        format!("{prefix}\n\n{rules}")
    }
}
