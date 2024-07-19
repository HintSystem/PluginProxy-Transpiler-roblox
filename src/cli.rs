use std::{
    fs,
    io::{self, Write},
    path::PathBuf,
    sync::{Arc, RwLock},
};

use clap::{Args, Parser, Subcommand};
use log::info;
use rfd::FileDialog;

use pluginproxy_transpiler::{error::Problem, RbxFileType};

type LogFile = Arc<RwLock<Option<fs::File>>>;
struct WrappedLogger {
    log: env_logger::Logger,
    log_file: LogFile,
}

impl log::Log for WrappedLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        self.log.enabled(metadata)
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            self.log.log(record);

            if let Some(ref mut log_file) = &mut *self.log_file.write().unwrap() {
                log_file.write_all(format!("{}\r\n", record.args()).as_bytes()).ok();
            }
        }
    }

    fn flush(&self) {}
}

#[derive(Parser)]
#[clap(author, version, about)]
struct TranspilerCliArgs {
    #[arg(short = 'i')]
    input: Option<PathBuf>,

    #[arg(value_name = "OUTPUT")]
    output: Option<PathBuf>,

    #[arg(long, visible_alias = "libs", action = clap::ArgAction::SetTrue)]
    #[arg(help = r"Include all libraries, even non-plugin ones like React or Fusion.
    Use this if the plugin depends on a module with the same name as a standard library
    and requires plugin-specific methods.")]
    include_libs: bool,

    /// Disable saving logs to file
    #[arg(long, action = clap::ArgAction::SetTrue)]
    no_logs: bool,
}

fn routine(log_file: LogFile) -> Result<(), Problem> {
    let cli = TranspilerCliArgs::parse();

    info!("PluginProxy Transpiler {}", env!("CARGO_PKG_VERSION"));
    let in_file = match cli.input {
        Some(path) => {
            RbxFileType::from_path(&path)?;
            path
        }
        None => {
            let msg = "Select a Roblox binary/xml file containing a plugin";
            info!("{msg}");
            FileDialog::new()
                .set_title(msg)
                .add_filter("Roblox", &["rbxm", "rbxl", "rbxmx", "rbxlx"])
                .pick_file()
                .ok_or(Problem::RFDCancel)?
        }
    };
    let input_dir = in_file.parent().ok_or(Problem::InvalidPath)?;

    let out_file = match cli.output {
        Some(path) => {
            RbxFileType::from_path(&path)?;
            path
        }
        None => input_dir.join("out.rbxm"),
    };
    let output_dir = out_file.parent().ok_or(Problem::InvalidPath)?;

    let log_file_name = "PluginProxy-Transpiler.log";
    if !cli.no_logs {
        log_file.write().unwrap().replace(
            fs::File::create(output_dir.join(log_file_name)).map_err(|error| Problem::IOError("create a log file", error))?,
        );
    }

    pluginproxy_transpiler::from_file(&in_file)?
        .exclude_libs(!cli.include_libs)
        .transpile_tree()?
        .save_to_file(&out_file)?;

    let end_message = if !cli.no_logs {
        format!(" Check {log_file_name} for a full log")
    } else {
        String::new()
    };

    info!("Done!{end_message}");
    Ok(())
}

fn main() {
    let env_logger = env_logger::Builder::new()
        .format(|buf, record| {
            let timestamp = buf.timestamp();
            let level = record.level().as_str();

            let style = buf.default_level_style(record.level());
            let args = if matches!(record.level(), log::Level::Error | log::Level::Warn) {
                format!("{style}{}{style:#}", record.args())
            } else {
                record.args().to_string()
            };

            writeln!(buf, "[{timestamp} {style}{level:5}{style:#}] {}", args)
        })
        .filter_level(log::LevelFilter::Info)
        .build();

    let log_file = Arc::new(RwLock::new(None));
    let logger = WrappedLogger {
        log: env_logger,
        log_file: Arc::clone(&log_file),
    };

    log::set_boxed_logger(Box::new(logger)).unwrap();
    log::set_max_level(log::LevelFilter::Info);

    if let Err(error) = routine(log_file) {
        log::error!("Error occurred with PluginProxy Transpiler.");
        log::error!("{}", error);
    }

    if std::env::args().nth(1).is_none() {
        println!("Press Enter to exit...");
        io::stdin().read_line(&mut String::new()).unwrap();
    }
}
