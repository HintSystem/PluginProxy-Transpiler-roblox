use log::info;
use rfd::FileDialog;
use std::{
    fs,
    io::{self, Write},
    path::PathBuf,
    sync::{Arc, RwLock},
};

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

fn routine(log_file: LogFile) -> Result<(), Problem> {
    info!("PluginProxy Transpiler {}", env!("CARGO_PKG_VERSION"));

    info!("Select a Roblox binary/xml file containing a plugin");
    let file_path = match std::env::args().nth(1) {
        Some(text) => PathBuf::from(text),
        None => FileDialog::new()
            .set_title("Select a Roblox binary/xml file containing a plugin")
            .add_filter("Roblox", &["rbxm", "rbxl", "rbxmx", "rbxlx"])
            .pick_file()
            .ok_or(Problem::RFDCancel)?,
    };

    let file_dir = file_path.parent().ok_or(Problem::InvalidPath)?;

    let log_file_name = "PluginProxy-Transpiler.log";
    log_file
        .write()
        .unwrap()
        .replace(fs::File::create(file_dir.join(log_file_name)).map_err(|error| Problem::IOError("create a log file", error))?);

    let out_file = match std::env::args().nth(2) {
        Some(text) => {
            let path = PathBuf::from(text);
            RbxFileType::from_path(&path)?;
            path
        }
        None => file_dir.join("out.rbxm"),
    };

    pluginproxy_transpiler::from_file(&file_path)?
        .transpile_tree()?
        .save_to_file(&out_file)?;

    info!("Done! Check {log_file_name} for a full log");
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
