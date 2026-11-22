//! `rig completions <shell>`.

use clap::{Args, CommandFactory};
use clap_complete::{Shell, generate};

use crate::Cli;

#[derive(Args)]
pub struct CompletionsArgs {
    /// Target shell to generate completions for.
    #[arg(value_enum)]
    pub shell: Shell,
}

pub fn run(args: CompletionsArgs) {
    let mut cmd = Cli::command();
    let name = cmd.get_name().to_string();
    generate(args.shell, &mut cmd, name, &mut std::io::stdout());
}
