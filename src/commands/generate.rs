//! `generate` subcommand.
use anyhow::{Context, Result};
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

/// Utility function to generate all Man pages for the main argument structure and all dependent
/// recursive subcommand pages to the output directory if specified, else the current directory.
///
/// # Errors
///
/// Returns a [`Result`] with an [`anyhow::Error`] if the output directory could not be created.
pub fn generate_man_pages(
    cmd: &clap::Command,
    output_dir: Option<PathBuf>,
) -> Result<()> {
    let output_dir: PathBuf = output_dir.unwrap_or(
        std::env::current_dir().context("Opening current directory")?,
    );

    std::fs::create_dir_all(&output_dir)
        .context("create output Man directories")?;

    let main_man = clap_mangen::Man::new(cmd.clone());
    let main_man_path = output_dir.join(format!("{}.1", cmd.get_name()));
    let mut man_man_file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&main_man_path)
        .with_context(|| {
            format!("failed to create {}", main_man_path.display())
        })?;
    main_man.render(&mut man_man_file)?;
    println!("Generated: {}", main_man_path.display());

    // Recurse over subcommands
    generate_subcommand_man_pages(cmd, &output_dir, cmd.get_name())?;

    Ok(())
}

/// Generate subcommand Man pages recursively.
fn generate_subcommand_man_pages(
    cmd: &clap::Command,
    output_dir: &Path,
    prefix: &str,
) -> Result<()> {
    for subcmd in cmd.get_subcommands() {
        let prefixed_name = format!("{}-{}", prefix, subcmd.get_name());

        // Rename the Command so clap_mangen uses the prefixed name in
        // NAME, SYNOPSIS, and SEE ALSO sections. The leaked &'static str
        // is fine here since man page generation is a one-shot operation.
        let leaked_name: &'static str =
            Box::leak(prefixed_name.clone().into_boxed_str());
        let renamed = subcmd
            .clone()
            .name(leaked_name)
            .disable_help_subcommand(true);
        let subcmd_man = clap_mangen::Man::new(renamed);

        let file_name = format!("{prefixed_name}.1");
        let man_path = output_dir.join(&file_name);
        let mut subcmd_file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&man_path)
            .with_context(|| {
                format!("failed to create {}", man_path.display())
            })?;

        subcmd_man.render(&mut subcmd_file)?;
        println!("Generated: {}", man_path.display());
        if subcmd.has_subcommands() {
            generate_subcommand_man_pages(
                subcmd,
                output_dir,
                &prefixed_name,
            )?;
        }
    }

    Ok(())
}
