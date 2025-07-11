use std::error::Error;
use vergen_gitcl::{Emitter, GitclBuilder};

fn main() -> Result<(), Box<dyn Error>> {
    // Emit the instructions
    Emitter::default()
        .add_instructions(&GitclBuilder::all_git()?)?
        .emit()?;
    Ok(())
}
