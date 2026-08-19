use std::env;
use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const HELPER_NAMES: &[&str] = &["smoosh-shell", "argv", "fds", "getenv", "readdir"];

pub(crate) fn status_if_invoked() -> Option<i32> {
    let invoked = env::args_os().next()?;
    let name = Path::new(&invoked).file_name()?;
    let helper = HELPER_NAMES
        .iter()
        .copied()
        .find(|candidate| name == OsStr::new(candidate))?;
    let arguments: Vec<OsString> = env::args_os().skip(1).collect();
    Some(match run(helper, &invoked, &arguments) {
        Ok(status) => status,
        Err(error) => {
            eprintln!("{helper}: {error}");
            1
        }
    })
}

fn run(name: &str, invoked: &OsStr, arguments: &[OsString]) -> Result<i32> {
    match name {
        "smoosh-shell" => shell(arguments),
        "argv" => argv(invoked, arguments),
        "fds" => fds(arguments),
        "getenv" => getenv(arguments),
        "readdir" => readdir(arguments),
        _ => Err(format!("unknown native Smoosh helper {name}").into()),
    }
}

fn shell(arguments: &[OsString]) -> Result<i32> {
    let shell =
        env::var_os("NSH_SURVEY_SMOOSH_SHELL").ok_or("NSH_SURVEY_SMOOSH_SHELL is not set")?;
    let encoded = env::var("NSH_SURVEY_SMOOSH_FLAGS_JSON")
        .map_err(|_| "NSH_SURVEY_SMOOSH_FLAGS_JSON is not set")?;
    let flags: Vec<String> = serde_json::from_str(&encoded)?;
    // The imported byte oracles were authored for the `smoosh` executable.
    // Preserve that invocation persona while process-replacing this wrapper.
    let error = Command::new(shell)
        .arg0("smoosh")
        .args(flags)
        .args(arguments)
        .exec();
    Err(error.into())
}

fn argv(invoked: &OsStr, arguments: &[OsString]) -> Result<i32> {
    let mut output = std::io::stdout().lock();
    for (index, argument) in std::iter::once(invoked)
        .chain(arguments.iter().map(OsString::as_os_str))
        .enumerate()
    {
        write!(output, "argv[{index}] = \"")?;
        output.write_all(argument.as_bytes())?;
        output.write_all(b"\";\n")?;
    }
    Ok(0)
}

fn getenv(arguments: &[OsString]) -> Result<i32> {
    let mut output = std::io::stdout().lock();
    for name in arguments {
        output.write_all(name.as_os_str().as_bytes())?;
        match env::var_os(name) {
            Some(value) => {
                output.write_all(b"='")?;
                output.write_all(value.as_os_str().as_bytes())?;
                output.write_all(b"'\n")?;
            }
            None => output.write_all(b" is unset\n")?,
        }
    }
    Ok(0)
}

fn fds(arguments: &[OsString]) -> Result<i32> {
    if arguments.len() > 2 {
        eprintln!("usage: fds [start_fd] [end_fd]");
        return Ok(1);
    }
    let start = parse_descriptor(arguments.first(), 0, "start fd")?;
    let stop = parse_descriptor(arguments.get(1), 9, "end fd")?;
    let mut output = std::io::stdout().lock();
    for descriptor in start..=stop {
        let path = Path::new("/proc/self/fd").join(descriptor.to_string());
        match fs::read_link(path) {
            Ok(_) => writeln!(output, "{descriptor} open")?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                writeln!(output, "{descriptor} closed")?
            }
            Err(error) => writeln!(output, "{descriptor} error: {error}")?,
        }
    }
    Ok(0)
}

fn parse_descriptor(value: Option<&OsString>, default: i32, description: &str) -> Result<i32> {
    let Some(value) = value else {
        return Ok(default);
    };
    let value = value
        .to_str()
        .ok_or_else(|| format!("{description} is not UTF-8"))?;
    let descriptor: i32 = value
        .parse()
        .map_err(|_| format!("couldn't parse {value:?} as a number for {description}"))?;
    if !(0..=1_000_000).contains(&descriptor) {
        return Err(format!("{description} {descriptor} is outside 0..=1000000").into());
    }
    Ok(descriptor)
}

fn readdir(arguments: &[OsString]) -> Result<i32> {
    let directory = match arguments {
        [] => PathBuf::from("."),
        [path] => PathBuf::from(path),
        _ => {
            eprintln!("usage: readdir [directory]");
            return Ok(2);
        }
    };
    let mut output = std::io::stdout().lock();
    output.write_all(b".\n..\n")?;
    for entry in fs::read_dir(&directory)
        .map_err(|error| format!("couldn't open directory {}: {error}", directory.display()))?
    {
        let name = entry?.file_name();
        output.write_all(name.as_os_str().as_bytes())?;
        output.write_all(b"\n")?;
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_arguments_are_bounded() {
        assert_eq!(parse_descriptor(None, 9, "end").unwrap(), 9);
        assert_eq!(
            parse_descriptor(Some(&OsString::from("20")), 9, "end").unwrap(),
            20
        );
        assert!(parse_descriptor(Some(&OsString::from("-1")), 9, "end").is_err());
    }
}
