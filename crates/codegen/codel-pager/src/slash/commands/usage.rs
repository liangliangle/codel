//! `/usage` shows this session's token and cost totals.

use crate::app::actions::Action;
use crate::slash::command::{
    AppCtx, ArgItem, CommandExecCtx, CommandResult, SlashCommand, slash_meta,
};

pub struct UsageCommand;

impl SlashCommand for UsageCommand {
    slash_meta! {
        name: "usage",
        aliases: ["cost"],
        description: "View usage",
        usage: "/usage",
        takes_args: false,
    }

    fn visible(&self, _ctx: &AppCtx) -> bool {
        true
    }

    fn suggest_args(&self, _ctx: &AppCtx, _args_query: &str) -> Option<Vec<ArgItem>> {
        None
    }

    fn run(&self, _ctx: &mut CommandExecCtx, args: &str) -> CommandResult {
        let arg = args.trim();
        if arg.is_empty() {
            CommandResult::Action(Action::ShowUsage)
        } else {
            CommandResult::Error(format!("Unknown argument: {arg}. Use /usage"))
        }
    }
}
