use std::path::PathBuf;

use clap::{Parser, Subcommand};


#[derive(Parser, Debug)]
#[clap(about, version, author)]
pub struct Args {
    #[clap(subcommand)]
    pub subcommand: BiCommand,
    #[arg(short, long)]
    pub abbs_path: PathBuf,
}

#[derive(Subcommand, Debug)]
pub enum BiCommand {
    /// Open pull request
    OpenPR {
        #[arg(short, long)]
        title: String,
        #[arg(short, long)]
        git_ref: Option<String>,
        #[arg(short, long)]
        packages: Vec<String>,
    },
    /// Login to Github
    Login,
}


fn main() {
    println!("Hello, world!");
}
