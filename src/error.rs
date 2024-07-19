use std::io;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Problem {
    #[error("Could not find the parent directory for file")]
    InvalidPath,
    #[error("User did not choose a file")]
    RFDCancel,
    #[error("While attempting to {0}, {1}")]
    IOError(&'static str, io::Error),
    #[error("While attempting to decode the place file, at {0} rbx_binary didn't know what to do")]
    BinaryDecodeError(rbx_binary::DecodeError),
    #[error("While attempting to decode the place file, at {0} rbx_xml didn't know what to do")]
    XMLDecodeError(rbx_xml::DecodeError),
    #[error("While attempting to decode the place file, at {0} rbx_binary didn't know what to do")]
    BinaryEncodeError(rbx_binary::EncodeError),
    #[error("While attempting to decode the place file, at {0} rbx_xml didn't know what to do")]
    XMLEncodeError(rbx_xml::EncodeError),
    #[error("File {} does not have the correct rbx file extension", .0.file_name().and_then(|name| name.to_str()).unwrap_or("None"))]
    InvalidExtension(PathBuf),
    #[error("While searching through file, no source script was found")]
    NoMainSource,
    #[error("Couldn't find source for script '{0}'")]
    NoScriptSource(String),
    #[error("While transpiling, {0:?}")]
    TranspilerError(Vec<full_moon::Error>),
}
