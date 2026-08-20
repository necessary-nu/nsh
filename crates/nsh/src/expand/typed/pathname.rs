//! Pathname expansion over quote-aware typed patterns.

use std::ffi::OsString;
use std::path::PathBuf;

use bstr::BString;
use nsh_platform::{NativeStrExt as _, ShellBytesExt as _};

use super::Field;
use crate::context::Shell;
use crate::pmatch::Pattern;

pub(super) fn expand(sh: &Shell, fields: Vec<Field>) -> Vec<Field> {
    fields
        .into_iter()
        .flat_map(|field| {
            let pattern = field.pattern();
            if !pattern.has_meta() {
                return vec![field];
            }
            let mut matches = matches(&sh.locale, &pattern);
            if matches.is_empty() {
                return vec![field];
            }
            matches.sort_by(|left, right| sh.locale.collate(left, right));
            matches
                .into_iter()
                .map(|bytes| Field::from_bytes(&bytes, false, false, false))
                .collect()
        })
        .collect()
}

fn matches(locale: &nsh_platform::Locale, pattern: &Pattern) -> Vec<BString> {
    let components = components(pattern);
    let absolute = pattern.as_bytes().first() == Some(&b'/');
    let trailing_slash = pattern.as_bytes().last() == Some(&b'/');
    let mut candidates = vec![(
        if absolute {
            PathBuf::from("/")
        } else {
            PathBuf::from(".")
        },
        if absolute {
            BString::from("/")
        } else {
            BString::new(Vec::new())
        },
    )];
    let mut saw_meta = false;

    for component in components {
        if component.as_bytes().is_empty() {
            continue;
        }
        if component.has_meta() {
            saw_meta = true;
            let mut next = Vec::new();
            for (directory, display) in candidates {
                let Ok(entries) = nsh_platform::read_directory(&directory) else {
                    continue;
                };
                let mut names = entries.into_iter().map(|entry| entry.name).collect::<Vec<_>>();
                if component.starts_with_literal_dot() {
                    names.push(OsString::from("."));
                    names.push(OsString::from(".."));
                }
                for name in names {
                    let bytes = name.to_shell_bytes();
                    if bytes.first() == Some(&b'.') && !component.starts_with_literal_dot() {
                        continue;
                    }
                    if component.matches(locale, &bytes) {
                        next.push((directory.join(&name), append_component(&display, &bytes)));
                    }
                }
            }
            candidates = next;
        } else {
            let Ok(name) = component.as_bytes().try_to_os_string() else {
                return Vec::new();
            };
            candidates = candidates
                .into_iter()
                .map(|(directory, display)| {
                    (
                        directory.join(&name),
                        append_component(&display, component.as_bytes()),
                    )
                })
                .collect();
        }
    }

    if saw_meta {
        candidates
            .into_iter()
            .filter_map(|(path, mut display)| {
                let exists = if trailing_slash {
                    nsh_platform::path_metadata(&path, true)
                        .is_ok_and(|metadata| metadata.kind == nsh_platform::FileKind::Directory)
                } else {
                    nsh_platform::path_metadata(&path, false).is_ok()
                };
                exists.then(|| {
                    if trailing_slash && display.last() != Some(&b'/') {
                        display.push(b'/');
                    }
                    display
                })
            })
            .collect()
    } else {
        Vec::new()
    }
}

fn components(pattern: &Pattern) -> Vec<Pattern> {
    let mut components = Vec::new();
    let mut start = 0;
    for (at, byte) in pattern.as_bytes().iter().enumerate() {
        if *byte == b'/' {
            components.push(pattern.slice(start..at));
            start = at + 1;
        }
    }
    components.push(pattern.slice(start..pattern.as_bytes().len()));
    components
}

fn append_component(prefix: &[u8], component: &[u8]) -> BString {
    let mut result = BString::from(prefix);
    if !result.is_empty() && result.last() != Some(&b'/') {
        result.push(b'/');
    }
    result.extend_from_slice(component);
    result
}
