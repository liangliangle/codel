use codel_config::ToolFeature;
use codel_tool_runtime::Tool;

use super::{codel_build, opencode};

#[test]
fn tool_features_name_the_tools_they_remove() {
    let cases = [
        (
            ToolFeature::AskUserQuestion,
            vec![codel_build::AskUserQuestionTool.id()],
        ),
        (
            ToolFeature::ImageEdit,
            vec![codel_build::ImageEditTool.id()],
        ),
        (ToolFeature::ImageGen, vec![codel_build::ImageGenTool.id()]),
        (ToolFeature::LspTools, vec![codel_build::LspTool.id()]),
        (
            ToolFeature::VideoGen,
            vec![
                codel_build::ImageToVideoTool.id(),
                codel_build::ReferenceToVideoTool.id(),
            ],
        ),
        (ToolFeature::WebFetch, vec![codel_build::WebFetchTool.id()]),
        (
            ToolFeature::WriteFile,
            vec![opencode::OpenCodeWriteTool.id()],
        ),
    ];
    for (feature, tools) in cases {
        let names: Vec<&str> = tools.iter().map(|id| id.as_str()).collect();
        assert_eq!(names, feature.tool_names(), "{feature:?}");
    }
}
