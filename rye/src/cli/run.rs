use std::env;
use std::ffi::{CString, OsString};
#[cfg(not(target_os = "windows"))]
use std::os::unix::prelude::OsStrExt;
#[cfg(target_os = "windows")]
use std::os::windows::prelude::OsStrExt;

use anyhow::{bail, Context, Error};
use clap::Parser;
use console::style;

use crate::pyproject::{PyProject, Script};
use crate::sync::{sync, SyncOptions};

/// Runs a command installed into this package.
#[derive(Parser, Debug)]
#[command(arg_required_else_help(false))]
pub struct Args {
    /// List all commands
    #[arg(short, long)]
    list: bool,
    /// The command to run
    #[command(subcommand)]
    cmd: Option<Command>,
}

#[derive(Parser, Debug)]
enum Command {
    #[command(external_subcommand)]
    External(Vec<OsString>),
}

pub fn execute(cmd: Args) -> Result<(), Error> {
    let pyproject = PyProject::discover()?;

    // make sure we have the minimal virtualenv.
    sync(SyncOptions::python_only()).context("failed to sync ahead of run")?;
    let venv_bin = pyproject.venv_bin_path();

    if cmd.list || cmd.cmd.is_none() {
        return list_scripts(&pyproject);
    }
    let mut args = match cmd.cmd {
        Some(Command::External(args)) => args,
        None => unreachable!(),
    };

    let short_name = args[0].to_string_lossy().to_string();

    // do we have a custom script to invoke?
    match pyproject.get_script_cmd(&args[0].to_string_lossy()) {
        Some(Script::Cmd(script_args)) if !script_args.is_empty() => {
            let script_target = venv_bin.join(&script_args[0]);
            if script_target.is_file() {
                args = Some(script_target.as_os_str().to_owned())
                    .into_iter()
                    .chain(script_args.into_iter().map(OsString::from).skip(1))
                    .chain(args.into_iter().skip(1))
                    .collect();
            } else {
                args = script_args
                    .into_iter()
                    .map(OsString::from)
                    .chain(args.into_iter().skip(1))
                    .collect();
            }
        }
        Some(Script::External(_)) => {
            args[0] = venv_bin.join(&args[0]).into();
        }
        _ => {}
    }

    let args = args
        .iter()
        .filter_map(|x| CString::new(x.as_bytes()).ok())
        .collect::<Vec<_>>();
    let path = CString::new(args[0].as_bytes())?;

    // when we spawn into a script, we implicitly activate the virtualenv to make
    // the life of tools easier that expect to be in one.
    env::set_var("VIRTUAL_ENV", &*pyproject.venv_path());
    if let Some(path) = env::var_os("PATH") {
        let mut new_path = venv_bin.as_os_str().to_owned();
        new_path.push(":");
        new_path.push(path);
        env::set_var("PATH", new_path);
    } else {
        env::set_var("PATH", &*venv_bin);
    }
    env::remove_var("PYTHONHOME");

    #[cfg(not(target_os = "windows"))]
    if let Err(err) = nix::unistd::execv(&path, &args) {
        if err == nix::Error::ENOENT {
            bail!("No script with name '{}' found in virtualenv", short_name);
        }
        return Err(err.into());
    }

    Ok(())
}

fn list_scripts(pyproject: &PyProject) -> Result<(), Error> {
    let mut scripts: Vec<_> = pyproject
        .list_scripts()
        .into_iter()
        .filter_map(|name| {
            let script = pyproject.get_script_cmd(&name)?;
            Some((name, script))
        })
        .collect();
    scripts.sort_by(|a, b| a.0.to_ascii_lowercase().cmp(&b.0.to_ascii_lowercase()));
    for (name, script) in scripts {
        if matches!(script, Script::External(_)) {
            println!("{}", name);
        } else {
            println!("{} ({})", name, style(script).dim());
        }
    }
    Ok(())
}
