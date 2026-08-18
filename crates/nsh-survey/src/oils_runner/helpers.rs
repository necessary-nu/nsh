use std::env;
use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::symlink;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::Command;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const NATIVE_HELPERS: &[&str] = &[
    "argv.py",
    "printenv.py",
    "read_from_fd.py",
    "show_fd_table.py",
    "stdout_stderr.py",
    "python2",
];

pub(super) struct FixtureView {
    pub(super) root: PathBuf,
    pub(super) bin: PathBuf,
}

pub(super) fn install(scratch: &Path, imported: &Path) -> Result<FixtureView> {
    let root = scratch.join("repo");
    let spec = root.join("spec");
    let bin = spec.join("bin");
    fs::create_dir_all(&bin)?;
    fs::create_dir(root.join("bin"))?;
    symlink(imported.join("spec/testdata"), spec.join("testdata"))?;

    let executable = env::current_exe()?;
    for entry in fs::read_dir(imported.join("spec/bin"))? {
        let entry = entry?;
        let name = entry.file_name();
        let target = bin.join(&name);
        if helper_name(&name).is_some() {
            symlink(&executable, target)?;
        } else {
            symlink(entry.path(), target)?;
        }
    }
    symlink(&executable, bin.join("python2"))?;
    Ok(FixtureView { root, bin })
}

pub(crate) fn status_if_invoked() -> Option<i32> {
    let invoked = env::args_os().next()?;
    let name = Path::new(&invoked).file_name()?;
    let helper = helper_name(name)?;
    let arguments: Vec<OsString> = env::args_os().skip(1).collect();
    Some(match run(helper, &arguments) {
        Ok(status) => status,
        Err(error) => {
            eprintln!("{helper}: {error}");
            1
        }
    })
}

fn helper_name(name: &OsStr) -> Option<&'static str> {
    NATIVE_HELPERS
        .iter()
        .copied()
        .find(|candidate| name == OsStr::new(candidate))
}

fn run(name: &str, arguments: &[OsString]) -> Result<i32> {
    match name {
        "argv.py" => argv(arguments),
        "printenv.py" => printenv(arguments),
        "read_from_fd.py" => read_from_fd(arguments),
        "show_fd_table.py" => show_fd_table(),
        "stdout_stderr.py" => stdout_stderr(arguments),
        "python2" => python2(arguments),
        _ => Err(format!("unknown native fixture helper {name}").into()),
    }
}

fn argv(arguments: &[OsString]) -> Result<i32> {
    let mut output = std::io::stdout().lock();
    output.write_all(b"[")?;
    for (index, argument) in arguments.iter().enumerate() {
        if index != 0 {
            output.write_all(b", ")?;
        }
        output.write_all(&python_repr(argument.as_os_str().as_bytes()))?;
    }
    output.write_all(b"]\n")?;
    Ok(0)
}

fn printenv(arguments: &[OsString]) -> Result<i32> {
    let mut output = std::io::stdout().lock();
    for name in arguments {
        if let Some(value) = env::var_os(name) {
            output.write_all(value.as_os_str().as_bytes())?;
        } else {
            output.write_all(b"None")?;
        }
        output.write_all(b"\n")?;
    }
    Ok(0)
}

fn read_from_fd(arguments: &[OsString]) -> Result<i32> {
    let mut output = std::io::stdout().lock();
    for argument in arguments {
        let number: i32 = argument
            .to_str()
            .ok_or("descriptor number is not UTF-8")?
            .parse()?;
        let path = Path::new("/proc/self/fd").join(number.to_string());
        let mut bytes = Vec::new();
        match fs::File::open(&path).and_then(|file| file.take(1024).read_to_end(&mut bytes)) {
            Ok(_) => {
                write!(output, "{number}: ")?;
                output.write_all(&bytes)?;
            }
            Err(error) => {
                eprintln!("FATAL: Error reading from fd {number}: {error}");
                return Ok(1);
            }
        }
    }
    Ok(0)
}

fn show_fd_table() -> Result<i32> {
    let directory = Path::new("/proc/self/fd");
    let mut entries: Vec<_> = fs::read_dir(directory)?.collect::<std::io::Result<_>>()?;
    entries.sort_by_key(|entry| {
        entry
            .file_name()
            .to_string_lossy()
            .parse::<u32>()
            .unwrap_or(u32::MAX)
    });
    let mut output = std::io::stdout().lock();
    for entry in entries {
        let name = entry.file_name();
        match fs::read_link(entry.path()) {
            Ok(link) => {
                output.write_all(name.as_bytes())?;
                output.write_all(b" ")?;
                output.write_all(link.as_os_str().as_bytes())?;
                output.write_all(b"\n")?;
            }
            Err(error) => writeln!(output, "{} {error}", name.to_string_lossy())?,
        }
    }
    Ok(0)
}

fn stdout_stderr(arguments: &[OsString]) -> Result<i32> {
    let stdout = arguments
        .first()
        .map_or(b"STDOUT".as_slice(), |value| value.as_os_str().as_bytes());
    let stderr = arguments
        .get(1)
        .map_or(b"STDERR".as_slice(), |value| value.as_os_str().as_bytes());
    let status: i32 = match arguments.get(2) {
        Some(value) => value.to_str().ok_or("status is not UTF-8")?.parse()?,
        None => 0,
    };
    std::io::stdout().lock().write_all(stdout)?;
    std::io::stdout().lock().write_all(b"\n")?;
    std::io::stderr().lock().write_all(stderr)?;
    std::io::stderr().lock().write_all(b"\n")?;
    Ok(status)
}

fn python2(arguments: &[OsString]) -> Result<i32> {
    if arguments.first().is_none_or(|argument| argument != "-c") || arguments.len() < 2 {
        return delegate_python3(arguments);
    }
    let script = arguments[1]
        .to_str()
        .ok_or("python2 compatibility snippet is not UTF-8")?;
    let script_arguments = &arguments[2..];
    let trimmed = script.trim();

    if trimmed == "import sys; print repr(sys.argv[1])" {
        let value = script_arguments.first().ok_or("missing sys.argv[1]")?;
        std::io::stdout()
            .lock()
            .write_all(&python_repr(value.as_os_str().as_bytes()))?;
        std::io::stdout().lock().write_all(b"\n")?;
        return Ok(0);
    }
    if trimmed.contains("small = u\"\\u00DF\"") && trimmed.contains("small.upper().encode") {
        std::io::stdout().lock().write_all(b"SS\n\xc3\x9f\n")?;
        return Ok(0);
    }
    if trimmed.contains("\"x\" * 65535") || trimmed.contains("\"x\" * 65536") {
        let count = if trimmed.contains("65536") {
            65_536
        } else {
            65_535
        };
        let mut output = std::io::stdout().lock();
        output.write_all(b"echo -n ")?;
        write_repeated(&mut output, b'X'.to_ascii_lowercase(), count)?;
        output.write_all(b"\n")?;
        return Ok(0);
    }
    if trimmed.contains("\"X\"*9000") {
        let mut output = std::io::stdout().lock();
        output.write_all(b"echo ")?;
        write_repeated(&mut output, b'X', 9_000)?;
        output.write_all(b" >out.txt\n")?;
        return Ok(0);
    }
    if let Some(bytes) = simple_print(trimmed)? {
        let mut output = std::io::stdout().lock();
        output.write_all(&bytes)?;
        output.write_all(b"\n")?;
        return Ok(0);
    }
    delegate_python3(arguments)
}

fn delegate_python3(arguments: &[OsString]) -> Result<i32> {
    let status = Command::new("python3").args(arguments).status()?;
    Ok(status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0)))
}

fn simple_print(script: &str) -> Result<Option<Vec<u8>>> {
    let literal = if let Some(inner) = script
        .strip_prefix("print(")
        .and_then(|value| value.strip_suffix(')'))
    {
        inner
    } else if let Some(inner) = script.strip_prefix("print ") {
        inner
    } else {
        return Ok(None);
    };
    if literal.len() < 2 {
        return Ok(None);
    }
    let quote = literal.as_bytes()[0];
    if !matches!(quote, b'\'' | b'"') || literal.as_bytes().last() != Some(&quote) {
        return Ok(None);
    }
    decode_python_string(&literal.as_bytes()[1..literal.len() - 1]).map(Some)
}

fn decode_python_string(value: &[u8]) -> Result<Vec<u8>> {
    let mut result = Vec::with_capacity(value.len());
    let mut index = 0;
    while index < value.len() {
        if value[index] != b'\\' {
            result.push(value[index]);
            index += 1;
            continue;
        }
        index += 1;
        let escaped = *value
            .get(index)
            .ok_or("trailing backslash in Python string")?;
        index += 1;
        match escaped {
            b'n' => result.push(b'\n'),
            b'r' => result.push(b'\r'),
            b't' => result.push(b'\t'),
            b'v' => result.push(0x0b),
            b'f' => result.push(0x0c),
            b'a' => result.push(0x07),
            b'b' => result.push(0x08),
            b'\\' | b'\'' | b'"' => result.push(escaped),
            b'x' => {
                let high = hex_digit(*value.get(index).ok_or("short \\x escape")?)?;
                let low = hex_digit(*value.get(index + 1).ok_or("short \\x escape")?)?;
                result.push((high << 4) | low);
                index += 2;
            }
            b'0'..=b'7' => {
                let mut number = (escaped - b'0') as u16;
                let mut digits = 1;
                while digits < 3 && index < value.len() && matches!(value[index], b'0'..=b'7') {
                    number = number * 8 + (value[index] - b'0') as u16;
                    index += 1;
                    digits += 1;
                }
                result.push(number as u8);
            }
            _ => {
                result.push(b'\\');
                result.push(escaped);
            }
        }
    }
    Ok(result)
}

fn hex_digit(value: u8) -> Result<u8> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err(format!("invalid hexadecimal digit {:?}", value as char).into()),
    }
}

fn write_repeated(output: &mut impl Write, byte: u8, count: usize) -> Result<()> {
    let buffer = [byte; 1024];
    let mut remaining = count;
    while remaining != 0 {
        let chunk = remaining.min(buffer.len());
        output.write_all(&buffer[..chunk])?;
        remaining -= chunk;
    }
    Ok(())
}

fn python_repr(value: &[u8]) -> Vec<u8> {
    let quote = if value.contains(&b'\'') && !value.contains(&b'"') {
        b'"'
    } else {
        b'\''
    };
    let mut result = vec![quote];
    for byte in value {
        match *byte {
            b'\\' => result.extend_from_slice(b"\\\\"),
            byte if byte == quote => {
                result.push(b'\\');
                result.push(byte);
            }
            b'\t' => result.extend_from_slice(b"\\t"),
            b'\n' => result.extend_from_slice(b"\\n"),
            b'\r' => result.extend_from_slice(b"\\r"),
            0x20..=0x7e => result.push(*byte),
            byte => result.extend_from_slice(format!("\\x{byte:02x}").as_bytes()),
        }
    }
    result.push(quote);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_two_strings_decode_as_bytes() {
        assert_eq!(decode_python_string(br"a\n\xce\101").unwrap(), b"a\n\xceA");
    }

    #[test]
    fn python_repr_preserves_bytes_and_quotes() {
        assert_eq!(python_repr(b"a'b\n\xff"), b"\"a'b\\n\\xff\"");
    }
}
