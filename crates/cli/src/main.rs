use std::path::{Path, PathBuf};
use std::process::ExitCode;

use appimage_integrator_core::{Change, Dirs, Error, inspect, integrate, list, remove, sync, watch};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Use this directory instead of appimage_dir and watch_dirs from the config file
    #[arg(long, global = true)]
    dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show the metadata embedded in an AppImage
    Info { file: PathBuf },
    /// Move an AppImage into the applications directory and add it to the launcher
    Integrate { file: PathBuf },
    /// List integrated AppImages
    List,
    /// Integrate new AppImages in the watched directories and drop entries for deleted ones
    Sync,
    /// Run sync, then keep reacting to AppImages added to or deleted from the watched directories
    Watch,
    /// Remove an integrated AppImage, its launcher entry and its icon
    Remove {
        /// Id as shown by the list command
        id: String,
        /// Leave the AppImage file in place
        #[arg(long)]
        keep_file: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let mut dirs = match Dirs::load() {
        Ok(dirs) => dirs,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    if let Some(dir) = cli.dir {
        dirs.watched = vec![dir.clone()];
        dirs.appimages = dir;
    }
    let (subject, result) = match cli.command {
        Command::Info { file } => (file.display().to_string(), info(&file)),
        Command::Integrate { file } => (file.display().to_string(), integrate_command(&file, &dirs)),
        Command::List => ("list".to_string(), list_command(&dirs)),
        Command::Sync => ("sync".to_string(), sync(&dirs, &mut print_change)),
        Command::Watch => ("watch".to_string(), watch(&dirs, &mut print_change)),
        Command::Remove { id, keep_file } => {
            let result = remove_command(&id, &dirs, keep_file);
            (id, result)
        }
    };
    if let Err(error) = result {
        eprintln!("{subject}: {error}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn info(file: &Path) -> Result<(), Error> {
    let info = inspect(file)?;
    println!("Name:     {}", info.name().unwrap_or("-"));
    println!("Version:  {}", info.version().unwrap_or("-"));
    println!("Desktop:  {}", info.desktop_file);
    match &info.icon {
        Some(icon) => println!(
            "Icon:     {} ({}, {} bytes)",
            icon.path.display(),
            icon.extension,
            icon.data.len()
        ),
        None => println!("Icon:     -"),
    }
    Ok(())
}

fn integrate_command(file: &Path, dirs: &Dirs) -> Result<(), Error> {
    let integrated = integrate(file, dirs)?;
    println!("Integrated {} as {}", integrated.name.as_deref().unwrap_or("-"), integrated.id);
    println!("AppImage:  {}", integrated.appimage.display());
    println!("Launcher:  {}", integrated.desktop_file.display());
    Ok(())
}

fn list_command(dirs: &Dirs) -> Result<(), Error> {
    let rows: Vec<[String; 4]> = list(dirs)?
        .into_iter()
        .map(|item| {
            let missing = if item.appimage.exists() { "" } else { " (missing)" };
            [
                item.id,
                item.name.unwrap_or_else(|| "-".to_string()),
                item.version.unwrap_or_else(|| "-".to_string()),
                format!("{}{missing}", item.appimage.display()),
            ]
        })
        .collect();
    let width = |column: usize, header: &str| {
        rows.iter().map(|row| row[column].chars().count()).max().unwrap_or(0).max(header.len())
    };
    let (id, name, version) = (width(0, "ID"), width(1, "NAME"), width(2, "VERSION"));
    println!("{:id$}  {:name$}  {:version$}  PATH", "ID", "NAME", "VERSION");
    for row in &rows {
        println!("{:id$}  {:name$}  {:version$}  {}", row[0], row[1], row[2], row[3]);
    }
    Ok(())
}

fn print_change(change: Change) {
    match change {
        Change::Integrated(item) => println!("integrated {}: {}", item.id, item.appimage.display()),
        Change::Removed(item) => println!("removed {}: {}", item.id, item.appimage.display()),
        Change::Failed(file, error) => eprintln!("{}: {error}", file.display()),
    }
}

fn remove_command(id: &str, dirs: &Dirs, keep_file: bool) -> Result<(), Error> {
    let removed = remove(id, dirs, keep_file)?;
    println!("Removed {} ({})", removed.name.as_deref().unwrap_or("-"), removed.id);
    if keep_file {
        println!("Kept {}", removed.appimage.display());
    }
    Ok(())
}
