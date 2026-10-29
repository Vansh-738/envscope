use clap::{Parser, ValueEnum};
use envscope::{Options, scan};
use std::collections::BTreeSet;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Text,
    Json,
    Template,
}

/// Check that environment variables used by Python code are documented.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Directory to scan; respects .gitignore and skips hidden paths
    #[arg(default_value = ".")]
    path: PathBuf,
    /// Example file, relative to the scan directory (or an absolute path)
    #[arg(long, default_value = ".env.example")]
    example: PathBuf,
    #[arg(long, value_enum, default_value = "text")]
    format: Format,
    /// Exclude paths matching a glob; may be repeated
    #[arg(long)]
    exclude: Vec<String>,
    /// Ignore an externally managed variable; may be repeated
    #[arg(long, value_name = "NAME")]
    ignore_key: Vec<String>,
    /// Also fail when a recognized lookup uses a dynamic key
    #[arg(long)]
    deny_dynamic: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let report = scan(&Options {
        root: cli.path,
        example: cli.example,
        excludes: cli.exclude,
        ignore_keys: cli.ignore_key.into_iter().collect(),
        template: matches!(cli.format, Format::Template),
    });
    let code = if !report.errors.is_empty() {
        2
    } else if (!matches!(cli.format, Format::Template) && !report.missing.is_empty())
        || (cli.deny_dynamic && !report.unresolved.is_empty())
    {
        1
    } else {
        0
    };
    let mut output = String::new();
    match cli.format {
        Format::Json => {
            output =
                serde_json::to_string_pretty(&report).expect("report contains serializable fields");
            output.push('\n');
        }
        Format::Template => {
            // Never emit a partial template when a source file could not be read.
            if report.errors.is_empty() {
                for name in report
                    .accesses
                    .iter()
                    .map(|a| &a.name)
                    .collect::<BTreeSet<_>>()
                {
                    output.push_str(&format!("{name}=\n"));
                }
            }
            for error in &report.errors {
                eprintln!("error: {error}");
            }
            for site in &report.unresolved {
                eprintln!("{}:{}: {}", site.file, site.line, site.reason);
            }
        }
        Format::Text => {
            for (name, accesses) in &report.missing {
                output.push_str(&format!("{name} is missing from the example file\n"));
                for access in accesses {
                    output.push_str(&format!(
                        "  {}:{}:{} ({})\n",
                        access.file, access.line, access.column, access.kind
                    ));
                }
            }
            for site in &report.unresolved {
                output.push_str(&format!("{}:{}: {}\n", site.file, site.line, site.reason));
            }
            for error in &report.errors {
                output.push_str(&format!("error: {error}\n"));
            }
            output.push_str(&format!("{} Python file(s), {} undocumented variable(s), {} dynamic lookup(s), {} error(s)\n",
                report.files_scanned, report.missing.len(), report.unresolved.len(), report.errors.len()));
        }
    }
    match io::stdout().lock().write_all(output.as_bytes()) {
        Ok(()) => ExitCode::from(code),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => ExitCode::from(code),
        Err(error) => {
            eprintln!("cannot write output: {error}");
            ExitCode::from(2)
        }
    }
}
