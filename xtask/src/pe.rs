//! Reads which DLLs a Windows program imports, from the file's bytes alone (nothing is run or loaded).
//!
//! Used by `portable-verify` to refuse a program that needs the Visual C++ runtime (`VCRUNTIME140.dll`, `MSVCP140.dll`
//! and the like). A clean Windows does not have those, so such a program stops at first launch with "VCRUNTIME140.dll was
//! not found". BDF's programs are built with the static C runtime (`-C target-feature=+crt-static`), which has no such
//! import; the check makes that a fact about the shipped files instead of a sentence in the documentation.
//!
//! The input is a file this code did not write, so every read is bounds-checked, every count is capped and no step
//! can panic.

/// A PE file has at most 96 sections.
const MAX_SECTIONS: usize = 96;
/// Far more imported DLLs than any real program has.
const MAX_DESCRIPTORS: usize = 1024;
/// A DLL name longer than this is not a DLL name.
const MAX_NAME: usize = 256;
/// Data directory numbers.
const IMPORT_DIRECTORY: usize = 1;
const DELAY_IMPORT_DIRECTORY: usize = 13;

/// The DLLs `bytes` imports (including delay-loaded ones), in file order, or why it is not a readable program.
pub fn imported_dlls(bytes: &[u8]) -> Result<Vec<String>, &'static str> {
    const NOT_PE: &str = "not a Windows program (no PE header)";
    const DAMAGED: &str = "a damaged Windows program (its headers run past the end of the file)";
    if bytes.get(..2) != Some(b"MZ") {
        return Err(NOT_PE);
    }
    let pe = to_usize(u32_at(bytes, 0x3C).ok_or(DAMAGED)?);
    if bytes.get(pe..pe.checked_add(4).ok_or(DAMAGED)?) != Some(b"PE\0\0") {
        return Err(NOT_PE);
    }
    let coff = pe.saturating_add(4);
    let section_count = usize::from(u16_at(bytes, coff.saturating_add(2)).ok_or(DAMAGED)?);
    let optional_size = usize::from(u16_at(bytes, coff.saturating_add(16)).ok_or(DAMAGED)?);
    let optional = coff.saturating_add(20);
    // PE32 (0x10b) and PE32+ (0x20b) differ in where the data directories start.
    let (count_at, dirs_at) = match u16_at(bytes, optional).ok_or(DAMAGED)? {
        0x20b => (optional.saturating_add(108), optional.saturating_add(112)),
        0x10b => (optional.saturating_add(92), optional.saturating_add(96)),
        _ => return Err(NOT_PE),
    };
    let directory_count = to_usize(u32_at(bytes, count_at).ok_or(DAMAGED)?);
    let directory = |index: usize| -> Option<(u32, u32)> {
        let at = dirs_at.checked_add(index.checked_mul(8)?)?;
        if index >= directory_count || at.checked_add(8)? > optional.checked_add(optional_size)? {
            return None;
        }
        Some((u32_at(bytes, at)?, u32_at(bytes, at.checked_add(4)?)?))
    };

    let table_at = optional.checked_add(optional_size).ok_or(DAMAGED)?;
    let mut sections = Vec::new();
    for i in 0..section_count.min(MAX_SECTIONS) {
        let at = table_at.checked_add(i.saturating_mul(40)).ok_or(DAMAGED)?;
        let virtual_size = u32_at(bytes, at.saturating_add(8)).ok_or(DAMAGED)?;
        let virtual_address = u32_at(bytes, at.saturating_add(12)).ok_or(DAMAGED)?;
        let raw_size = u32_at(bytes, at.saturating_add(16)).ok_or(DAMAGED)?;
        let raw_pointer = u32_at(bytes, at.saturating_add(20)).ok_or(DAMAGED)?;
        sections.push((virtual_address, virtual_size.max(raw_size), raw_pointer));
    }
    // A relative virtual address to an offset in the file, if it lies inside a section.
    let offset = |rva: u32| -> Option<usize> {
        let (va, _, raw) = sections.iter().copied().find(|&(va, len, _)| rva >= va && rva - va < len)?;
        to_usize(raw).checked_add(to_usize(rva - va))
    };

    let mut names = Vec::new();
    if let Some((rva, _)) = directory(IMPORT_DIRECTORY).filter(|d| d.0 != 0) {
        // 20-byte descriptors, ended by an all-zero one; the DLL's name is the fourth word.
        let table = offset(rva).ok_or(DAMAGED)?;
        for i in 0..MAX_DESCRIPTORS {
            let at = table.checked_add(i.saturating_mul(20)).ok_or(DAMAGED)?;
            let words = bytes.get(at..at.checked_add(20).ok_or(DAMAGED)?).ok_or(DAMAGED)?;
            if words.iter().all(|b| *b == 0) {
                break;
            }
            if let Some(name) = u32_at(words, 12).filter(|n| *n != 0).and_then(offset).and_then(|o| c_string(bytes, o)) {
                names.push(name);
            }
        }
    }
    if let Some((rva, _)) = directory(DELAY_IMPORT_DIRECTORY).filter(|d| d.0 != 0) {
        // 32-byte descriptors; bit 0 of the first word says the addresses are RVAs (every current linker); the name is the second.
        let table = offset(rva).ok_or(DAMAGED)?;
        for i in 0..MAX_DESCRIPTORS {
            let at = table.checked_add(i.saturating_mul(32)).ok_or(DAMAGED)?;
            let words = bytes.get(at..at.checked_add(32).ok_or(DAMAGED)?).ok_or(DAMAGED)?;
            if words.iter().all(|b| *b == 0) {
                break;
            }
            if u32_at(words, 0).is_some_and(|attributes| attributes & 1 == 1)
                && let Some(name) = u32_at(words, 4).filter(|n| *n != 0).and_then(offset).and_then(|o| c_string(bytes, o))
            {
                names.push(name);
            }
        }
    }
    Ok(names)
}

/// The imported DLLs that belong to the Visual C++ runtime, which a clean Windows does not have. The Universal C Runtime
/// (`api-ms-win-crt-*`, `ucrtbase.dll`) is part of Windows 10 and 11 and is not listed.
pub fn runtime_dependencies(dlls: &[String]) -> Vec<String> {
    const PREFIXES: [&str; 7] = ["vcruntime", "msvcp", "msvcr", "vcomp", "concrt", "vccorlib", "mfc"];
    dlls.iter().filter(|d| PREFIXES.iter().any(|p| d.to_ascii_lowercase().starts_with(p))).cloned().collect()
}

fn to_usize(n: u32) -> usize {
    usize::try_from(n).unwrap_or(usize::MAX)
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    let raw = bytes.get(at..at.checked_add(2)?)?;
    Some(u16::from_le_bytes([*raw.first()?, *raw.get(1)?]))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    let raw = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([*raw.first()?, *raw.get(1)?, *raw.get(2)?, *raw.get(3)?]))
}

/// The NUL-ended ASCII name at `at`, if one ends within [`MAX_NAME`] bytes.
fn c_string(bytes: &[u8], at: usize) -> Option<String> {
    let window = bytes.get(at..)?;
    let end = window.iter().take(MAX_NAME).position(|b| *b == 0)?;
    Some(String::from_utf8_lossy(window.get(..end)?).into_owned())
}

/// A minimal PE32+ file whose import table names `imports`, for tests here and in `release`.
#[cfg(test)]
pub(crate) fn synthetic(imports: &[&str]) -> Vec<u8> {
    const SECTION_RVA: u32 = 0x1000;
    let table = (imports.len() + 1) * 20;
    let mut names = Vec::new();
    let mut section = vec![0u8; table];
    for (i, name) in imports.iter().enumerate() {
        let rva = SECTION_RVA + u32::try_from(table + names.len()).unwrap_or(0);
        section[i * 20..i * 20 + 4].copy_from_slice(&1u32.to_le_bytes());
        section[i * 20 + 12..i * 20 + 16].copy_from_slice(&rva.to_le_bytes());
        names.extend_from_slice(name.as_bytes());
        names.push(0);
    }
    section.extend_from_slice(&names);
    let mut b = vec![0u8; 0x200];
    b[0..2].copy_from_slice(b"MZ");
    b[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    b[0x40..0x44].copy_from_slice(b"PE\0\0");
    let coff = 0x44;
    b[coff..coff + 2].copy_from_slice(&0x8664u16.to_le_bytes());
    b[coff + 2..coff + 4].copy_from_slice(&1u16.to_le_bytes());
    b[coff + 16..coff + 18].copy_from_slice(&240u16.to_le_bytes());
    let optional = coff.saturating_add(20);
    b[optional..optional + 2].copy_from_slice(&0x20bu16.to_le_bytes());
    b[optional + 108..optional + 112].copy_from_slice(&16u32.to_le_bytes());
    let import = optional + 112 + 8;
    b[import..import + 4].copy_from_slice(&SECTION_RVA.to_le_bytes());
    b[import + 4..import + 8].copy_from_slice(&u32::try_from(table).unwrap_or(0).to_le_bytes());
    let header = optional + 240;
    b[header..header + 6].copy_from_slice(b".idata");
    let len = u32::try_from(section.len()).unwrap_or(0);
    b[header + 8..header + 12].copy_from_slice(&len.to_le_bytes());
    b[header + 12..header + 16].copy_from_slice(&SECTION_RVA.to_le_bytes());
    b[header + 16..header + 20].copy_from_slice(&len.to_le_bytes());
    b[header + 20..header + 24].copy_from_slice(&0x200u32.to_le_bytes());
    b.extend_from_slice(&section);
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_import_table_is_read_in_order() {
        let pe = synthetic(&["KERNEL32.dll", "VCRUNTIME140.dll", "api-ms-win-crt-heap-l1-1-0.dll"]);
        let dlls = imported_dlls(&pe).unwrap();
        assert_eq!(dlls, ["KERNEL32.dll", "VCRUNTIME140.dll", "api-ms-win-crt-heap-l1-1-0.dll"]);
        assert_eq!(runtime_dependencies(&dlls), ["VCRUNTIME140.dll"], "only the Visual C++ runtime counts, not the Universal CRT");
        assert!(imported_dlls(&synthetic(&[])).unwrap().is_empty());
    }

    #[test]
    fn every_visual_cpp_runtime_family_is_named_whatever_its_case() {
        let dlls: Vec<String> = ["vcruntime140_1.dll", "MSVCP140.dll", "msvcr120.dll", "VCOMP140.DLL", "KERNEL32.dll", "ucrtbase.dll", "bcrypt.dll"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(runtime_dependencies(&dlls), ["vcruntime140_1.dll", "MSVCP140.dll", "msvcr120.dll", "VCOMP140.DLL"]);
    }

    #[test]
    fn what_is_not_a_program_is_refused_by_name() {
        for bad in [
            &b""[..],
            b"M",
            b"bdf.exe",
            b"MZ",
            b"MZ\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
        ] {
            assert!(imported_dlls(bad).is_err(), "{bad:?}");
        }
    }

    /// The file is not under our control: no truncation and no flipped byte of a valid program may panic.
    #[test]
    fn hostile_variations_of_a_valid_program_never_panic() {
        let pe = synthetic(&["KERNEL32.dll", "VCRUNTIME140.dll"]);
        for len in 0..pe.len() {
            let _ = imported_dlls(&pe[..len]);
        }
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..20_000 {
            let mut copy = pe.clone();
            for _ in 0..1 + next() % 4 {
                let at = usize::try_from(next() % copy.len() as u64).unwrap_or(0);
                copy[at] = (next() & 0xFF) as u8;
            }
            let _ = imported_dlls(&copy);
        }
    }

    /// A table with far more descriptors than any program has, and a name that never ends, are bounded.
    #[test]
    fn endless_tables_and_names_are_capped() {
        let many = vec!["A.dll"; 3000];
        assert_eq!(imported_dlls(&synthetic(&many)).unwrap().len(), MAX_DESCRIPTORS, "stops at the cap");
        let mut long = synthetic(&["A"]);
        let len = long.len();
        for b in &mut long[len - 2..] {
            *b = b'x'; // the name loses its NUL
        }
        assert!(imported_dlls(&long).unwrap().is_empty(), "a name with no end within the cap is skipped, not read to the end of the file");
    }

    /// A real program: the test executable itself imports KERNEL32 on Windows.
    #[cfg(windows)]
    #[test]
    fn the_test_executable_itself_imports_kernel32() {
        let me = std::fs::read(std::env::current_exe().unwrap()).unwrap();
        let dlls = imported_dlls(&me).unwrap();
        assert!(dlls.iter().any(|d| d.eq_ignore_ascii_case("kernel32.dll")), "{dlls:?}");
    }
}
